//! Shared broker state and the UniFFI-exported [`MqttBroker`] object.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::{MqttVersion, QoS};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::{
    MqttAuthProvider, MqttBrokerConfig, MqttBrokerEventListener, SharedAuthProvider,
    SharedEventListener,
};
use crate::connection::handle_connection;
use crate::session::{QueuedMessage, Session, Subscription};
use crate::topic::topic_matches;

/// One retained message stored per topic (MQTT-3.3.1.3 / MQTT-5.0 §3.3.1.3).
#[derive(Debug, Clone)]
struct RetainedMessage {
    payload: Bytes,
    qos: QoS,
}

/// State shared by every connection task belonging to one [`MqttBroker`].
pub(crate) struct BrokerState {
    pub config: MqttBrokerConfig,
    sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
    retained: Mutex<HashMap<String, RetainedMessage>>,
    pub auth_provider: Mutex<Option<SharedAuthProvider>>,
    event_listener: Mutex<Option<SharedEventListener>>,
    client_count: AtomicU32,
}

impl BrokerState {
    fn new(config: MqttBrokerConfig) -> Self {
        BrokerState {
            config,
            sessions: RwLock::new(HashMap::new()),
            retained: Mutex::new(HashMap::new()),
            auth_provider: Mutex::new(None),
            event_listener: Mutex::new(None),
            client_count: AtomicU32::new(0),
        }
    }

    pub fn client_count(&self) -> u32 {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Attach (creating or resuming) the session for `client_id`. Returns
    /// `(session_present, previous_connections_shutdown_handle)`.
    pub fn attach_session(
        &self,
        client_id: &str,
        version: MqttVersion,
        clean_start: bool,
        will: Option<mqtt_client::protocol::connect::Will>,
        max_queued: usize,
    ) -> (bool, Option<oneshot::Sender<()>>) {
        let mut sessions = self.sessions.write().unwrap();
        let existed = sessions.contains_key(client_id);

        let prev_shutdown = if existed {
            let arc = sessions.get(client_id).unwrap().clone();
            let mut s = arc.lock().unwrap();
            let old_shutdown = s.shutdown.take();
            let was_connected = s.sender.take().is_some();
            if was_connected {
                self.client_count.fetch_sub(1, Ordering::Relaxed);
            }
            old_shutdown
        } else {
            None
        };

        let session_present = existed && !clean_start;
        if !session_present {
            let mut new_session =
                Session::new(client_id.to_string(), version, clean_start, max_queued);
            new_session.will = will;
            sessions.insert(client_id.to_string(), Arc::new(Mutex::new(new_session)));
        } else {
            let arc = sessions.get(client_id).unwrap().clone();
            let mut s = arc.lock().unwrap();
            s.version = version;
            s.clean_start = clean_start;
            s.will = will;
        }
        (session_present, prev_shutdown)
    }

    pub fn set_sender(&self, client_id: &str, tx: mpsc::UnboundedSender<Bytes>) {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            session.lock().unwrap().sender = Some(tx);
            self.client_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_shutdown_handle(&self, client_id: &str, tx: oneshot::Sender<()>) {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            session.lock().unwrap().shutdown = Some(tx);
        }
    }

    pub fn send_to(&self, client_id: &str, packet: &Packet) -> bool {
        let sessions = self.sessions.read().unwrap();
        let Some(session) = sessions.get(client_id) else {
            return false;
        };
        let guard = session.lock().unwrap();
        let version = guard.version;
        match packet.encode(version) {
            Ok(bytes) => guard.send_raw(bytes.freeze()),
            Err(_) => false,
        }
    }

    pub fn flush_offline_queue(&self, client_id: &str) {
        let sessions = self.sessions.read().unwrap();
        let Some(session) = sessions.get(client_id) else {
            return;
        };
        let mut guard = session.lock().unwrap();
        let version = guard.version;
        let queued: Vec<QueuedMessage> = guard.queued.drain(..).collect();
        for msg in queued {
            let packet_id = if msg.qos != QoS::AtMostOnce {
                Some(guard.alloc_packet_id())
            } else {
                None
            };
            let publish = Packet::Publish(PublishPacket {
                dup: false,
                qos: msg.qos,
                retain: msg.retain,
                topic: msg.topic,
                packet_id,
                payload: msg.payload,
                properties: Properties::new(),
            });
            if let Ok(bytes) = publish.encode(version) {
                guard.send_raw(bytes.freeze());
            }
        }
    }

    pub fn store_incoming_qos2(&self, client_id: &str, packet_id: u16, publish: PublishPacket) {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            session
                .lock()
                .unwrap()
                .incoming_qos2
                .entry(packet_id)
                .or_insert(publish);
        }
    }

    pub fn take_incoming_qos2(&self, client_id: &str, packet_id: u16) -> Option<QueuedMessage> {
        let session = self.sessions.read().unwrap().get(client_id)?.clone();
        let mut guard = session.lock().unwrap();
        guard
            .incoming_qos2
            .remove(&packet_id)
            .map(|p| QueuedMessage {
                topic: p.topic,
                payload: p.payload,
                qos: p.qos,
                retain: p.retain,
            })
    }

    /// Called on receiving a PUBREC for a QoS 2 PUBLISH we sent. Returns
    /// `true` (and clears the pending flag) if `packet_id` was one of
    /// ours — the caller should then reply with PUBREL.
    pub fn complete_outgoing_qos2(&self, client_id: &str, packet_id: u16) -> bool {
        match self.sessions.read().unwrap().get(client_id) {
            Some(session) => session.lock().unwrap().outgoing_qos2.remove(&packet_id),
            None => false,
        }
    }

    pub fn update_retained(&self, publish: &PublishPacket) {
        let mut retained = self.retained.lock().unwrap();
        if publish.payload.is_empty() {
            retained.remove(&publish.topic);
            return;
        }
        if self.config.max_retained_messages > 0
            && retained.len() as u32 >= self.config.max_retained_messages
            && !retained.contains_key(&publish.topic)
        {
            return; // at capacity; silently drop new retained topics
        }
        retained.insert(
            publish.topic.clone(),
            RetainedMessage {
                payload: publish.payload.clone(),
                qos: publish.qos,
            },
        );
    }

    pub fn send_matching_retained(&self, client_id: &str, filter: &str, max_qos: QoS) {
        let matches: Vec<(String, RetainedMessage)> = {
            let retained = self.retained.lock().unwrap();
            retained
                .iter()
                .filter(|(topic, _)| topic_matches(filter, topic))
                .map(|(t, m)| (t.clone(), m.clone()))
                .collect()
        };
        for (topic, msg) in matches {
            let sessions = self.sessions.read().unwrap();
            let Some(session) = sessions.get(client_id) else {
                return;
            };
            let guard = session.lock().unwrap();
            let version = guard.version;
            let qos = msg.qos.min(max_qos);
            let packet_id = if qos != QoS::AtMostOnce {
                Some(guard.alloc_packet_id())
            } else {
                None
            };
            let publish = Packet::Publish(PublishPacket {
                dup: false,
                qos,
                retain: true,
                topic,
                packet_id,
                payload: msg.payload,
                properties: Properties::new(),
            });
            if let Ok(bytes) = publish.encode(version) {
                guard.send_raw(bytes.freeze());
            }
        }
    }

    pub fn add_subscription(&self, client_id: &str, filter: &str, sub: Subscription) -> bool {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            let mut guard = session.lock().unwrap();
            guard
                .subscriptions
                .insert(filter.to_string(), sub)
                .is_none()
        } else {
            false
        }
    }

    pub fn remove_subscription(&self, client_id: &str, filter: &str) {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            session.lock().unwrap().subscriptions.remove(filter);
        }
    }

    pub fn discard_will(&self, client_id: &str) {
        if let Some(session) = self.sessions.read().unwrap().get(client_id) {
            session.lock().unwrap().will = None;
        }
    }

    /// Deliver `message` (published by `publisher_id`) to every matching
    /// subscriber, live or queued for later delivery.
    pub fn fan_out(&self, publisher_id: &str, message: &QueuedMessage) {
        let sessions = self.sessions.read().unwrap();
        for (client_id, session) in sessions.iter() {
            let mut guard = session.lock().unwrap();
            let best = guard
                .subscriptions
                .iter()
                .filter(|(filter, sub)| {
                    topic_matches(filter, &message.topic)
                        && !(client_id.as_str() == publisher_id && sub.no_local)
                })
                .map(|(_, sub)| sub.clone())
                .max_by_key(|sub| sub.qos);
            let Some(sub) = best else { continue };

            let qos = message.qos.min(sub.qos);
            let retain = message.retain && sub.retain_as_published;

            if guard.is_connected() {
                let packet_id = if qos != QoS::AtMostOnce {
                    Some(guard.alloc_packet_id())
                } else {
                    None
                };
                if qos == QoS::ExactlyOnce {
                    if let Some(id) = packet_id {
                        guard.outgoing_qos2.insert(id);
                    }
                }
                let publish = Packet::Publish(PublishPacket {
                    dup: false,
                    qos,
                    retain,
                    topic: message.topic.clone(),
                    packet_id,
                    payload: message.payload.clone(),
                    properties: Properties::new(),
                });
                if let Ok(bytes) = publish.encode(guard.version) {
                    guard.send_raw(bytes.freeze());
                }
            } else if !guard.clean_start {
                guard.enqueue_offline(QueuedMessage {
                    topic: message.topic.clone(),
                    payload: message.payload.clone(),
                    qos,
                    retain,
                });
            }
        }
    }

    /// Detach `client_id`'s connection. If `clean_start` is set the whole
    /// session is discarded; otherwise it's kept (minus the live sender) so
    /// a future CONNECT can resume it. Publishes the will, if any.
    pub fn detach_session(self: &Arc<Self>, client_id: &str, graceful: bool) {
        let (will, clean_start) = {
            let sessions = self.sessions.read().unwrap();
            let Some(session) = sessions.get(client_id) else {
                return;
            };
            let mut guard = session.lock().unwrap();
            let was_connected = guard.sender.take().is_some();
            if was_connected {
                self.client_count.fetch_sub(1, Ordering::Relaxed);
            }
            guard.shutdown = None;
            (
                if graceful { None } else { guard.will.take() },
                guard.clean_start,
            )
        };

        if clean_start {
            self.sessions.write().unwrap().remove(client_id);
        }

        if let Some(will) = will {
            let message = QueuedMessage {
                topic: will.topic,
                payload: will.payload,
                qos: will.qos,
                retain: will.retain,
            };
            if message.retain {
                self.update_retained(&PublishPacket {
                    dup: false,
                    qos: message.qos,
                    retain: true,
                    topic: message.topic.clone(),
                    packet_id: None,
                    payload: message.payload.clone(),
                    properties: Properties::new(),
                });
            }
            self.fan_out(client_id, &message);
        }
    }

    pub fn notify_connected(&self, client_id: &str) {
        if let Some(listener) = self.event_listener.lock().unwrap().clone() {
            listener.on_client_connected(client_id.to_string());
        }
    }

    pub fn notify_disconnected(&self, client_id: &str, reason: &str) {
        if let Some(listener) = self.event_listener.lock().unwrap().clone() {
            listener.on_client_disconnected(client_id.to_string(), reason.to_string());
        }
    }

    pub fn notify_message_published(&self, client_id: &str, topic: &str, qos: QoS) {
        if let Some(listener) = self.event_listener.lock().unwrap().clone() {
            listener.on_message_published(client_id.to_string(), topic.to_string(), qos);
        }
    }
}

/// A running (or not-yet-started) MQTT broker instance, speaking both MQTT
/// 3.1.1 and MQTT 5.0 — the protocol version is negotiated independently
/// per connection from each client's CONNECT packet.
#[derive(uniffi::Object)]
pub struct MqttBroker {
    state: Arc<BrokerState>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
    bound_port: Mutex<Option<u16>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MqttBroker {
    #[uniffi::constructor]
    pub fn new(config: MqttBrokerConfig) -> Self {
        MqttBroker {
            state: Arc::new(BrokerState::new(config)),
            accept_task: Mutex::new(None),
            bound_port: Mutex::new(None),
        }
    }

    /// Register a pluggable authentication provider. Must be called before
    /// [`start`](Self::start) to affect connections made right away, but
    /// may be replaced at any time.
    pub fn set_auth_provider(&self, provider: Arc<dyn MqttAuthProvider>) {
        *self.state.auth_provider.lock().unwrap() = Some(provider);
    }

    /// Register a listener for connect/disconnect/publish events.
    pub fn set_event_listener(&self, listener: Arc<dyn MqttBrokerEventListener>) {
        *self.state.event_listener.lock().unwrap() = Some(listener);
    }

    /// Bind the listening socket and start accepting connections.
    pub async fn start(&self) -> mqtt_client::error::MqttResult<()> {
        if self.accept_task.lock().unwrap().is_some() {
            return Err(mqtt_client::MqttError::AlreadyConnected);
        }
        let addr = format!(
            "{}:{}",
            self.state.config.bind_address, self.state.config.port
        );
        let listener = TcpListener::bind(&addr).await?;
        let local_port = listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(self.state.config.port);
        *self.bound_port.lock().unwrap() = Some(local_port);

        let state = self.state.clone();
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let state = state.clone();
                        tokio::spawn(handle_connection(state, stream, peer.to_string()));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept() failed");
                        break;
                    }
                }
            }
        });
        *self.accept_task.lock().unwrap() = Some(handle);
        Ok(())
    }

    /// Stop accepting new connections and abort the accept loop. Existing
    /// client connections are left running until they naturally close.
    pub async fn stop(&self) -> mqtt_client::error::MqttResult<()> {
        if let Some(handle) = self.accept_task.lock().unwrap().take() {
            handle.abort();
        }
        *self.bound_port.lock().unwrap() = None;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.accept_task.lock().unwrap().is_some()
    }

    /// Number of currently connected clients.
    pub fn client_count(&self) -> u32 {
        self.state.client_count()
    }

    /// The TCP port actually bound (useful when `config.port == 0` was used
    /// to request an ephemeral port, e.g. in tests).
    pub fn bound_port(&self) -> Option<u16> {
        *self.bound_port.lock().unwrap()
    }
}
