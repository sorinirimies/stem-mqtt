//! `StemMqtt.Broker`: an embedded MQTT broker owned by one Erlang process.

use std::sync::{Arc, Weak};

use mqtt_broker::{MqttBroker, MqttBrokerError, MqttBrokerEventListener, QoS};
use rustler::{Encoder, Env, LocalPid, Monitor, NifResult, Resource, ResourceArc, Term};

use crate::{atoms, binary, error_term, next_handle_id, notify, options, runtime, spawn_reply};

/// Pending CONNECTs the owner has not answered yet; more are denied.
const AUTH_QUEUE_CAPACITY: u32 = 1024;

pub struct BrokerResource {
    broker: Arc<MqttBroker>,
}

#[rustler::resource_impl]
impl Resource for BrokerResource {
    // The owner exited: stop serving, nobody answers auth requests any more.
    fn down<'a>(&'a self, _env: Env<'a>, _pid: LocalPid, _monitor: Monitor) {
        stop_in_background(&self.broker);
    }
}

impl Drop for BrokerResource {
    // Garbage-collected while running: release the ports.
    fn drop(&mut self) {
        stop_in_background(&self.broker);
    }
}

fn stop_in_background(broker: &Arc<MqttBroker>) {
    if broker.is_running() {
        let broker = Arc::clone(broker);
        runtime().spawn(async move {
            // Ignored on purpose: nobody is left to report to.
            let _ = broker.stop().await;
        });
    }
}

struct OwnerEvents {
    owner: LocalPid,
    handle_id: u64,
}

impl MqttBrokerEventListener for OwnerEvents {
    fn on_client_connected(&self, client_id: String) {
        notify(&self.owner, self.handle_id, |env| {
            (atoms::client_connected(), client_id).encode(env)
        });
    }

    fn on_client_disconnected(&self, client_id: String, reason: String) {
        notify(&self.owner, self.handle_id, |env| {
            (atoms::client_disconnected(), client_id, reason).encode(env)
        });
    }

    fn on_message_published(&self, client_id: String, topic: String, qos: QoS) {
        notify(&self.owner, self.handle_id, |env| {
            (atoms::message_published(), client_id, topic, qos as u8).encode(env)
        });
    }
}

/// Forwards queued CONNECTs to the owner as
/// `{:auth_request, id, client_id, username, password}` until the broker is gone.
fn pump_auth_requests(broker: Weak<MqttBroker>, owner: LocalPid, handle_id: u64) {
    runtime().spawn(async move {
        while let Some(broker) = broker.upgrade() {
            let Some(request) = broker.next_auth_request(1_000).await else {
                continue;
            };
            notify(&owner, handle_id, |env| {
                let password = match &request.password {
                    Some(p) => binary(env, p),
                    None => rustler::types::atom::nil().encode(env),
                };
                (
                    atoms::auth_request(),
                    request.id,
                    request.client_id.as_str(),
                    request.username.as_deref(),
                    password,
                )
                    .encode(env)
            });
        }
    });
}

fn error_kind(e: &MqttBrokerError) -> &'static str {
    match e {
        MqttBrokerError::AlreadyRunning => "already_running",
        MqttBrokerError::Io(_) => "io",
        MqttBrokerError::Tls(_) => "tls",
    }
}

fn unit_reply<'a>(env: Env<'a>, result: Result<(), MqttBrokerError>) -> Term<'a> {
    match result {
        Ok(()) => atoms::ok().encode(env),
        Err(e) => error_term(env, error_kind(&e), e.to_string()),
    }
}

/// A new, stopped broker. `events` pushes connect/disconnect/publish events to
/// the owner; `auth` sends every CONNECT to the owner for a decision.
#[rustler::nif]
fn broker_new<'a>(
    env: Env<'a>,
    config: Term<'a>,
    owner: LocalPid,
    events: bool,
    auth: bool,
) -> NifResult<Term<'a>> {
    let config = options::broker_config(config)?;
    let handle_id = next_handle_id();
    let broker = Arc::new(MqttBroker::new(config));
    if events {
        broker.set_event_listener(Arc::new(OwnerEvents { owner, handle_id }));
    }
    if auth {
        broker.enable_auth_queue(AUTH_QUEUE_CAPACITY);
        pump_auth_requests(Arc::downgrade(&broker), owner, handle_id);
    }
    let resource = ResourceArc::new(BrokerResource { broker });
    resource.monitor(Some(env), &owner);
    Ok((resource, handle_id).encode(env))
}

#[rustler::nif]
fn broker_start(env: Env, broker: ResourceArc<BrokerResource>, request: u64) -> rustler::Atom {
    let b = Arc::clone(&broker.broker);
    spawn_reply(env, request, async move { b.start().await }, unit_reply);
    atoms::ok()
}

#[rustler::nif]
fn broker_stop(env: Env, broker: ResourceArc<BrokerResource>, request: u64) -> rustler::Atom {
    let b = Arc::clone(&broker.broker);
    spawn_reply(env, request, async move { b.stop().await }, unit_reply);
    atoms::ok()
}

#[rustler::nif]
fn broker_is_running(broker: ResourceArc<BrokerResource>) -> bool {
    broker.broker.is_running()
}

#[rustler::nif]
fn broker_client_count(broker: ResourceArc<BrokerResource>) -> u32 {
    broker.broker.client_count()
}

#[rustler::nif]
fn broker_bound_port(broker: ResourceArc<BrokerResource>) -> Option<u16> {
    broker.broker.bound_port()
}

#[rustler::nif]
fn broker_bound_ws_port(broker: ResourceArc<BrokerResource>) -> Option<u16> {
    broker.broker.bound_ws_port()
}

#[rustler::nif]
fn broker_bound_tls_port(broker: ResourceArc<BrokerResource>) -> Option<u16> {
    broker.broker.bound_tls_port()
}

#[rustler::nif]
fn broker_resolve_auth(broker: ResourceArc<BrokerResource>, id: u64, allow: bool) -> bool {
    broker.broker.resolve_auth_request(id, allow)
}
