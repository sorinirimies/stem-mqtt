//! `StemMqtt.Client`: an MQTT client owned by one Erlang process.

use std::sync::Arc;

use mqtt_client::{MqttClient, MqttError, MqttMessage, MqttMessageListener};
use rustler::{Binary, Encoder, Env, LocalPid, Monitor, NifResult, Resource, ResourceArc, Term};

use crate::{atoms, binary, error_term, next_handle_id, notify, options, runtime, spawn_reply};

pub struct ClientResource {
    client: Arc<MqttClient>,
}

#[rustler::resource_impl]
impl Resource for ClientResource {
    // The owner exited: nobody can receive this client's messages any more.
    fn down<'a>(&'a self, _env: Env<'a>, _pid: LocalPid, _monitor: Monitor) {
        disconnect_in_background(&self.client);
    }
}

impl Drop for ClientResource {
    // Garbage-collected while connected: close the connection.
    fn drop(&mut self) {
        disconnect_in_background(&self.client);
    }
}

fn disconnect_in_background(client: &Arc<MqttClient>) {
    let client = Arc::clone(client);
    runtime().spawn(async move {
        // Ignored on purpose: nobody is left to report to.
        let _ = client.disconnect().await;
    });
}

/// Pushes messages and disconnects to the owner. Holds the handle id, not the
/// resource, so it never keeps the client alive.
struct OwnerListener {
    owner: LocalPid,
    handle_id: u64,
}

impl MqttMessageListener for OwnerListener {
    fn on_message(&self, msg: MqttMessage) {
        notify(&self.owner, self.handle_id, |env| {
            (atoms::message(), message_term(env, &msg)).encode(env)
        });
    }

    fn on_disconnected(&self, reason: String) {
        notify(&self.owner, self.handle_id, |env| {
            (atoms::disconnected(), reason).encode(env)
        });
    }
}

/// `%StemMqtt.Message{topic, payload, qos, retain}`.
fn message_term<'a>(env: Env<'a>, msg: &MqttMessage) -> Term<'a> {
    let module = rustler::Atom::from_str(env, "Elixir.StemMqtt.Message").expect("valid atom");
    let pairs = [
        ("__struct__", module.encode(env)),
        ("topic", msg.topic.encode(env)),
        ("payload", binary(env, &msg.payload)),
        ("qos", msg.qos.as_u8().encode(env)),
        ("retain", msg.retain.encode(env)),
    ];
    let keys: Vec<Term> = pairs
        .iter()
        .map(|(k, _)| {
            rustler::Atom::from_str(env, k)
                .expect("valid atom")
                .encode(env)
        })
        .collect();
    let values: Vec<Term> = pairs.iter().map(|(_, v)| *v).collect();
    Term::map_from_arrays(env, &keys, &values).expect("unique keys")
}

fn error_kind(e: &MqttError) -> &'static str {
    match e {
        MqttError::MalformedPacket(_) => "malformed_packet",
        MqttError::Protocol(_) => "protocol",
        MqttError::Io(_) => "io",
        MqttError::ConnectionRefused(_) => "connection_refused",
        MqttError::NotConnected => "not_connected",
        MqttError::PacketTooLarge(_) => "packet_too_large",
        MqttError::AlreadyConnected => "already_connected",
        MqttError::Timeout => "timeout",
        MqttError::Session(_) => "session",
    }
}

fn unit_reply<'a>(env: Env<'a>, result: Result<(), MqttError>) -> Term<'a> {
    match result {
        Ok(()) => atoms::ok().encode(env),
        Err(e) => error_term(env, error_kind(&e), e.to_string()),
    }
}

#[rustler::nif]
fn client_new<'a>(env: Env<'a>, options: Term<'a>, owner: LocalPid) -> NifResult<Term<'a>> {
    let options = options::connect_options(options)?;
    let handle_id = next_handle_id();
    let client = Arc::new(MqttClient::new(options));
    client.set_message_listener(Arc::new(OwnerListener { owner, handle_id }));
    let resource = ResourceArc::new(ClientResource { client });
    resource.monitor(Some(env), &owner);
    Ok((resource, handle_id).encode(env))
}

#[rustler::nif]
fn client_connect(env: Env, client: ResourceArc<ClientResource>, request: u64) -> rustler::Atom {
    let c = Arc::clone(&client.client);
    spawn_reply(
        env,
        request,
        async move { c.connect().await },
        |env, result| match result {
            Ok(r) => {
                let map = Term::map_new(env)
                    .map_put(
                        rustler::Atom::from_str(env, "session_present").expect("atom"),
                        r.session_present,
                    )
                    .and_then(|m| {
                        m.map_put(
                            rustler::Atom::from_str(env, "reason_code").expect("atom"),
                            r.reason_code,
                        )
                    })
                    .expect("fresh map");
                (atoms::ok(), map).encode(env)
            }
            Err(e) => error_term(env, error_kind(&e), e.to_string()),
        },
    );
    atoms::ok()
}

#[rustler::nif]
fn client_publish(
    env: Env,
    client: ResourceArc<ClientResource>,
    request: u64,
    topic: String,
    payload: Binary,
    qos: Term,
    retain: bool,
) -> NifResult<rustler::Atom> {
    let qos = options::qos(qos)?;
    let payload = payload.as_slice().to_vec();
    let c = Arc::clone(&client.client);
    spawn_reply(
        env,
        request,
        async move { c.publish(topic, payload, qos, retain).await },
        unit_reply,
    );
    Ok(atoms::ok())
}

#[rustler::nif]
fn client_subscribe(
    env: Env,
    client: ResourceArc<ClientResource>,
    request: u64,
    filter: String,
    qos: Term,
) -> NifResult<rustler::Atom> {
    let qos = options::qos(qos)?;
    let c = Arc::clone(&client.client);
    spawn_reply(
        env,
        request,
        async move { c.subscribe(filter, qos).await },
        |env, result| match result {
            Ok(r) => (atoms::ok(), r.reason_code).encode(env),
            Err(e) => error_term(env, error_kind(&e), e.to_string()),
        },
    );
    Ok(atoms::ok())
}

#[rustler::nif]
fn client_unsubscribe(
    env: Env,
    client: ResourceArc<ClientResource>,
    request: u64,
    filter: String,
) -> rustler::Atom {
    let c = Arc::clone(&client.client);
    spawn_reply(
        env,
        request,
        async move { c.unsubscribe(filter).await },
        unit_reply,
    );
    atoms::ok()
}

#[rustler::nif]
fn client_disconnect(env: Env, client: ResourceArc<ClientResource>, request: u64) -> rustler::Atom {
    let c = Arc::clone(&client.client);
    spawn_reply(
        env,
        request,
        async move { c.disconnect().await },
        unit_reply,
    );
    atoms::ok()
}

#[rustler::nif]
fn client_is_connected(client: ResourceArc<ClientResource>) -> bool {
    client.client.is_connected()
}

#[rustler::nif]
fn client_last_disconnect_reason(client: ResourceArc<ClientResource>) -> Option<String> {
    client.client.last_disconnect_reason()
}
