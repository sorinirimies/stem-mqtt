//! Session registry: owns the `client_id -> Session` map and every
//! operation that only needs that map — attach/detach on (re)connect,
//! per-client send/queue/QoS-2 bookkeeping, and fanning a published
//! message out to matching subscribers. Retained-message storage
//! ([`crate::retain::RetainStore`]) and event notification
//! ([`crate::events::EventHub`]) are separate concerns the broker
//! composes alongside this registry rather than folding into it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;
use mqtt_client::protocol::connect::Will;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::{MqttVersion, QoS};
use tokio::sync::{mpsc, oneshot};

use crate::session::{QueuedMessage, Session, Subscription};
use crate::topic::topic_matches;

/// Registry of every client session the broker currently knows about,
/// whether connected right now or persisted offline (`clean_start = false`).
pub struct SessionRegistry {
    sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
    client_count: AtomicU32,
}

impl SessionRegistry {
    pub fn new() -> Self {
        SessionRegistry {
            sessions: RwLock::new(HashMap::new()),
            client_count: AtomicU32::new(0),
        }
    }

    pub fn client_count(&self) -> u32 {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Attach (creating or resuming) the session for `client_id`. Returns
    /// `(session_present, previous_connection's_shutdown_handle)` — the
    /// caller is responsible for firing that shutdown handle to evict the
    /// client id's previous connection (MQTT-3.1.4-3).
    pub fn attach(
        &self,
        client_id: &str,
        version: MqttVersion,
        clean_start: bool,
        will: Option<Will>,
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

    /// Send a retained message directly to one (just-subscribed) client,
    /// downgrading QoS to the subscription's granted maximum.
    pub fn send_retained(&self, client_id: &str, topic: String, payload: Bytes, qos: QoS) {
        let sessions = self.sessions.read().unwrap();
        let Some(session) = sessions.get(client_id) else {
            return;
        };
        let guard = session.lock().unwrap();
        let version = guard.version;
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
            payload,
            properties: Properties::new(),
        });
        if let Ok(bytes) = publish.encode(version) {
            guard.send_raw(bytes.freeze());
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
    /// a future CONNECT can resume it. Returns the will to publish, if the
    /// disconnect was ungraceful and one was configured.
    pub fn detach(&self, client_id: &str, graceful: bool) -> Option<Will> {
        let (will, clean_start) = {
            let sessions = self.sessions.read().unwrap();
            let session = sessions.get(client_id)?;
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

        will
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
