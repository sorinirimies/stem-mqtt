//! Per-client session state: subscriptions, the outbound delivery channel,
//! offline message queueing, QoS 2 dedup bookkeeping, and outgoing-message
//! redelivery tracking.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use bytes::Bytes;
use mqtt_client::protocol::connect::Will;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::MqttVersion;
use tokio::sync::{mpsc, oneshot};

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

impl QueuedMessage {
    /// Build the PUBLISH packet that carries this message to a subscriber.
    pub fn to_publish(&self, packet_id: Option<u16>, dup: bool) -> PublishPacket {
        PublishPacket {
            dup,
            qos: self.qos,
            retain: self.retain,
            topic: self.topic.clone(),
            packet_id,
            payload: self.payload.clone(),
            properties: Properties::new(),
        }
    }
}

impl From<&PublishPacket> for QueuedMessage {
    fn from(p: &PublishPacket) -> Self {
        QueuedMessage {
            topic: p.topic.clone(),
            payload: p.payload.clone(),
            qos: p.qos,
            retain: p.retain,
        }
    }
}

/// What we're waiting to hear back for one packet id we sent to this
/// session, and what to resend (with DUP=1) if that ack takes too long.
/// See [`crate::registry::SessionRegistry::retry_pending`].
#[derive(Debug, Clone)]
pub enum PendingRedelivery {
    /// Awaiting PUBACK (QoS 1) or PUBREC (QoS 2, first handshake step) for
    /// this PUBLISH. Retried by resending the same PUBLISH with DUP=1.
    Publish(QueuedMessage),
    /// Awaiting PUBCOMP (QoS 2, second handshake step) for this PUBREL —
    /// the PUBLISH itself already got its PUBREC, so only the packet id
    /// needs resending, not the payload. PUBREL has no DUP flag; it's just
    /// resent as-is.
    PubRel,
}

/// One in-flight outgoing acknowledgement: a packet id we sent to this
/// session and are still waiting on the peer to ack, plus how many times
/// we've already retried it.
#[derive(Debug, Clone)]
pub struct RedeliveryEntry {
    pub kind: PendingRedelivery,
    pub attempts: u32,
    pub last_sent: Instant,
}

/// Why the broker is asking a live connection to shut down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    /// A new CONNECT with the same client id took the session over
    /// (MQTT-3.1.4-3).
    TakenOver,
    /// The broker itself is stopping.
    BrokerStopped,
}

/// The live network connection currently attached to a [`Session`].
pub struct LiveConn {
    /// Unique per accepted connection. Lets a connection task tell, at
    /// teardown, whether the session is *still its own* or has since been
    /// taken over by a newer connection with the same client id — without
    /// it, a taken-over connection's cleanup would tear down its
    /// replacement.
    pub id: u64,
    /// Bounded channel of encoded packets to the connection's writer task.
    pub sender: mpsc::Sender<Bytes>,
    /// Fires to tell the connection task to shut down.
    pub shutdown: Option<oneshot::Sender<ShutdownReason>>,
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
    /// Set while the client is connected.
    pub conn: Option<LiveConn>,
    /// Messages queued while offline, to flush on the next CONNECT.
    pub queued: VecDeque<QueuedMessage>,
    /// Packet ids this session has sent us a PUBLISH for at QoS 2, not yet
    /// resolved with PUBREL (dedup + hold-until-PUBREL semantics).
    pub incoming_qos2: HashMap<u16, PublishPacket>,
    /// QoS 1/2 PUBLISHes and QoS 2 PUBRELs *we* sent to this session,
    /// awaiting their ack, eligible for a timed dup-flagged resend — see
    /// [`crate::registry::SessionRegistry::retry_pending`]. Without this, a
    /// broker-initiated QoS 1/2 delivery that's lost in flight (or whose
    /// ack is lost) would just silently never complete.
    pub pending_redelivery: HashMap<u16, RedeliveryEntry>,
    next_packet_id: u16,
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
            conn: None,
            queued: VecDeque::new(),
            incoming_qos2: HashMap::new(),
            pending_redelivery: HashMap::new(),
            next_packet_id: 1,
            max_queued,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Next non-zero packet id not already awaiting an ack from this
    /// client (MQTT-2.2.1-3). Overwriting an in-flight id would silently
    /// lose that message's redelivery tracking.
    pub fn alloc_packet_id(&mut self) -> u16 {
        for _ in 0..=u16::MAX {
            let id = self.next_packet_id;
            self.next_packet_id = self.next_packet_id.wrapping_add(1);
            if id != 0 && !self.pending_redelivery.contains_key(&id) {
                return id;
            }
        }
        // Every id is in flight; hand out the next one anyway rather than
        // stall (the oldest entry is simply superseded).
        self.next_packet_id
    }

    pub fn enqueue_offline(&mut self, message: QueuedMessage) {
        if self.max_queued > 0 && self.queued.len() >= self.max_queued {
            self.queued.pop_front();
        }
        self.queued.push_back(message);
    }

    /// Hand already-encoded bytes to this session's writer task. Returns
    /// `false` if the session has no live connection **or its outbound
    /// queue is full** (a slow consumer): the bytes are dropped rather than
    /// buffered without bound.
    pub fn send_raw(&self, bytes: Bytes) -> bool {
        let Some(conn) = &self.conn else {
            return false;
        };
        match conn.sender.try_send(bytes) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!(client_id = %self.client_id, "outbound queue full; dropping packet");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Encode `packet` for this session's protocol version and queue it.
    pub fn send_packet(&self, packet: &Packet) -> bool {
        match packet.encode(self.version) {
            Ok(bytes) => self.send_raw(bytes.freeze()),
            Err(_) => false,
        }
    }

    /// Deliver `message` (whose QoS/retain have already been adjusted for
    /// this subscriber) to the live connection, and — for QoS 1/2 — start
    /// tracking it for timed redelivery. The single place a PUBLISH to a
    /// subscriber is built, so live fan-out, retained replay and offline
    /// flush all get identical packet-id and redelivery handling.
    pub fn deliver(&mut self, message: QueuedMessage) {
        let packet_id =
            (message.qos != mqtt_client::QoS::AtMostOnce).then(|| self.alloc_packet_id());
        self.send_packet(&Packet::Publish(message.to_publish(packet_id, false)));
        if let Some(id) = packet_id {
            self.pending_redelivery.insert(
                id,
                RedeliveryEntry {
                    kind: PendingRedelivery::Publish(message),
                    attempts: 0,
                    last_sent: Instant::now(),
                },
            );
        }
    }
}
