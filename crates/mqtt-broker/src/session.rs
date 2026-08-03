//! Per-client session state: subscriptions, the outbound delivery channel,
//! offline message queueing, and QoS 2 dedup bookkeeping.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU16, Ordering};

use bytes::Bytes;
use mqtt_client::protocol::connect::Will;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::MqttVersion;
use tokio::sync::mpsc;

/// One subscription held by a session.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub qos: mqtt_client::QoS,
    pub no_local: bool,
    pub retain_as_published: bool,
}

/// A queued message waiting for an offline (`clean_start = false`) session
/// to reconnect.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub topic: String,
    pub payload: Bytes,
    pub qos: mqtt_client::QoS,
    pub retain: bool,
}

/// All state the broker keeps for one client identifier, whether currently
/// connected or persisted across a `clean_start = false` disconnect.
pub struct Session {
    #[allow(dead_code)] // kept for debugging/observability; not read internally
    pub client_id: String,
    pub version: MqttVersion,
    pub clean_start: bool,
    pub subscriptions: HashMap<String, Subscription>,
    pub will: Option<Will>,
    /// Set while the client is connected; encoded packets for this client
    /// are sent down this channel to its writer task.
    pub sender: Option<mpsc::UnboundedSender<Bytes>>,
    /// Messages queued while offline, to flush on the next CONNECT.
    pub queued: VecDeque<QueuedMessage>,
    /// Packet ids this session has sent us a PUBLISH for at QoS 2, not yet
    /// resolved with PUBREL (dedup + hold-until-PUBREL semantics).
    pub incoming_qos2: HashMap<u16, PublishPacket>,
    /// Packet ids of QoS 2 PUBLISHes *we* sent to this session, waiting for
    /// its PUBREC before we can send PUBREL (MQTT-5.0 §4.3.3). Without this,
    /// a broker-initiated QoS 2 delivery would stop dead after PUBREC —
    /// the subscriber would never see PUBREL/PUBCOMP and the message would
    /// never actually complete delivery.
    pub outgoing_qos2: HashSet<u16>,
    /// Fires to tell a *previous* connection for this same client id to shut
    /// down (MQTT-3.1.4-3: a new CONNECT with the same ClientID takes over).
    pub shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    next_packet_id: AtomicU16,
    max_queued: usize,
}

impl Session {
    pub fn new(
        client_id: String,
        version: MqttVersion,
        clean_start: bool,
        max_queued: usize,
    ) -> Self {
        Session {
            client_id,
            version,
            clean_start,
            subscriptions: HashMap::new(),
            will: None,
            sender: None,
            queued: VecDeque::new(),
            incoming_qos2: HashMap::new(),
            outgoing_qos2: HashSet::new(),
            shutdown: None,
            next_packet_id: AtomicU16::new(1),
            max_queued,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.sender.is_some()
    }

    pub fn alloc_packet_id(&self) -> u16 {
        loop {
            let id = self.next_packet_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }

    pub fn enqueue_offline(&mut self, message: QueuedMessage) {
        if self.max_queued > 0 && self.queued.len() >= self.max_queued {
            self.queued.pop_front();
        }
        self.queued.push_back(message);
    }

    /// Send raw, already-encoded bytes to this session's connection.
    /// Returns `false` if the session has no live connection.
    pub fn send_raw(&self, bytes: Bytes) -> bool {
        match &self.sender {
            Some(tx) => tx.send(bytes).is_ok(),
            None => false,
        }
    }
}
