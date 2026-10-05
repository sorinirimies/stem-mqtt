//! Session registry: owns the `client_id -> Session` map and every
//! operation that only needs that map — attach/detach on (re)connect,
//! per-client send/queue/QoS-2 bookkeeping, and fanning a published
//! message out to matching subscribers. Retained-message storage
//! ([`crate::retain::RetainStore`]) and event notification
//! ([`crate::events::EventHub`]) are separate concerns the broker
//! composes alongside this registry rather than folding into it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::Will;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::support::{LockExt, RwLockExt};
use mqtt_client::{MqttVersion, QoS};
use tokio::sync::{mpsc, oneshot};

use crate::index::{IndexEntry, SubscriptionIndex};
use crate::session::{
    LiveConn, PendingRedelivery, QueuedMessage, Session, ShutdownReason, Subscription,
};

/// How often [`SessionRegistry::retry_pending`] sweeps for unacked
/// outgoing QoS 1/2 packets to resend, unless overridden by
/// [`crate::config::MqttBrokerConfig::redelivery_interval_secs`].
pub const DEFAULT_REDELIVERY_INTERVAL: Duration = Duration::from_secs(5);
/// Resends attempted before giving up on a packet (best-effort beyond
/// this — there's no unbounded retry queue to avoid unbounded memory
/// growth for a permanently-gone peer).
pub const MAX_REDELIVERY_ATTEMPTS: u32 = 5;

/// Everything [`SessionRegistry::attach`] needs to bind a new connection to
/// a session.
pub struct AttachRequest {
    pub client_id: String,
    pub version: MqttVersion,
    pub clean_start: bool,
    pub will: Option<Will>,
    /// Per-session offline queue cap (`0` = unlimited).
    pub max_queued: usize,
    /// Cap on simultaneously connected clients (`0` = unlimited).
    pub max_clients: u32,
    /// Writer-task channel for this connection.
    pub sender: mpsc::Sender<Bytes>,
    /// Lets the registry (eviction, broker stop) shut this connection down.
    pub shutdown: oneshot::Sender<ShutdownReason>,
}

/// Why [`SessionRegistry::attach`] refused a connection.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachError {
    /// `max_clients` simultaneous connections are already live.
    QuotaExceeded,
}

/// A successfully attached connection.
#[derive(Debug, PartialEq, Eq)]
pub struct Attached {
    /// Whether a previous session for this client id was resumed.
    pub session_present: bool,
    /// Identifies this connection to [`SessionRegistry::detach`].
    pub conn_id: u64,
}

/// Registry of every client session the broker currently knows about,
/// whether connected right now or persisted offline (`clean_start = false`).
pub struct SessionRegistry {
    sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
    /// Number of sessions that currently have a live connection.
    client_count: AtomicU32,
    next_conn_id: AtomicU64,
    /// Topic trie of every subscription, so a PUBLISH finds its subscribers
    /// without scanning all sessions. Lock order: `sessions` → a session →
    /// `index`; a lookup copies its matches out and releases `index` before
    /// touching any session.
    index: RwLock<SubscriptionIndex>,
    /// Round-robin cursor for shared-subscription groups.
    shared_cursor: AtomicUsize,
}

impl SessionRegistry {
    pub fn new() -> Self {
        SessionRegistry {
            sessions: RwLock::new(HashMap::new()),
            client_count: AtomicU32::new(0),
            next_conn_id: AtomicU64::new(1),
            index: RwLock::new(SubscriptionIndex::default()),
            shared_cursor: AtomicUsize::new(0),
        }
    }

    /// Remove every subscription of a discarded session from the index.
    fn unindex_session(&self, client_id: &str, session: &Session) {
        let mut index = self.index.write_safe();
        for key in session.subscriptions.keys() {
            index.remove(client_id, key);
        }
    }

    pub fn client_count(&self) -> u32 {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Run `f` on `client_id`'s session, if there is one. The common shape
    /// of almost every registry operation — look up, lock, act — so each
    /// one below is just its action.
    fn with_session<R>(&self, client_id: &str, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
        let sessions = self.sessions.read_safe();
        let session = sessions.get(client_id)?;
        let mut guard = session.lock_safe();
        Some(f(&mut guard))
    }

    /// Attach (creating or resuming) the session for `client_id` and bind
    /// the new connection to it, **atomically**: the quota check, eviction
    /// of any previous connection with the same client id
    /// (MQTT-3.1.4-3), session creation/resumption, and installing the
    /// new connection's channels all happen under one lock, so no other
    /// CONNECT can interleave.
    pub fn attach(&self, req: AttachRequest) -> Result<Attached, AttachError> {
        let mut sessions = self.sessions.write_safe();
        let existing = sessions.get(&req.client_id).cloned();

        let was_connected = existing
            .as_ref()
            .is_some_and(|s| s.lock_safe().is_connected());
        // A takeover replaces a connection rather than adding one, so it
        // never counts against the quota.
        if !was_connected
            && req.max_clients > 0
            && self.client_count.load(Ordering::Relaxed) >= req.max_clients
        {
            return Err(AttachError::QuotaExceeded);
        }

        // Evict the previous connection, if any.
        if let Some(session) = &existing {
            if let Some(mut old) = session.lock_safe().conn.take() {
                self.client_count.fetch_sub(1, Ordering::Relaxed);
                if let Some(shutdown) = old.shutdown.take() {
                    let _ = shutdown.send(ShutdownReason::TakenOver);
                }
            }
        }

        let session_present = existing.is_some() && !req.clean_start;
        let session = match existing {
            Some(session) if session_present => session,
            replaced => {
                if let Some(old) = replaced {
                    self.unindex_session(&req.client_id, &old.lock_safe());
                }
                let session = Arc::new(Mutex::new(Session::new(
                    req.client_id.clone(),
                    req.version,
                    req.clean_start,
                    req.max_queued,
                )));
                sessions.insert(req.client_id.clone(), session.clone());
                session
            }
        };

        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut guard = session.lock_safe();
            guard.version = req.version;
            guard.clean_start = req.clean_start;
            guard.will = req.will;
            guard.offline_since = None;
            guard.conn = Some(LiveConn {
                id: conn_id,
                sender: req.sender,
                shutdown: Some(req.shutdown),
            });
        }
        self.client_count.fetch_add(1, Ordering::Relaxed);
        Ok(Attached {
            session_present,
            conn_id,
        })
    }

    /// Detach connection `conn_id` from `client_id`'s session. If
    /// `clean_start` is set the whole session is discarded; otherwise it's
    /// kept (minus the live connection) so a future CONNECT can resume it.
    /// Returns the will to publish, if the disconnect was ungraceful and
    /// one was configured.
    ///
    /// A **no-op if `conn_id` is no longer the session's current
    /// connection** — i.e. a newer connection already took the session
    /// over. Without this check, the evicted connection's teardown would
    /// rip the session (and publish a spurious will) out from under its
    /// replacement.
    pub fn detach(&self, client_id: &str, conn_id: u64, graceful: bool) -> Option<Will> {
        // One write lock for the whole operation: releasing it between
        // "mark detached" and "remove clean session" would let a racing
        // CONNECT resume a session that's about to be deleted.
        let mut sessions = self.sessions.write_safe();
        let session = sessions.get(client_id)?.clone();
        let mut guard = session.lock_safe();
        if guard.conn.as_ref().map(|c| c.id) != Some(conn_id) {
            return None;
        }
        guard.conn = None;
        self.client_count.fetch_sub(1, Ordering::Relaxed);
        let will = if graceful { None } else { guard.will.take() };
        let clean_start = guard.clean_start;
        if !clean_start {
            guard.offline_since = Some(Instant::now());
        }
        drop(guard);

        if clean_start {
            if let Some(removed) = sessions.remove(client_id) {
                self.unindex_session(client_id, &removed.lock_safe());
            }
        }
        will
    }

    /// Ask every live connection to shut down (broker stop). Sessions and
    /// their queued state are left intact.
    pub fn shutdown_all(&self) {
        let sessions = self.sessions.read_safe();
        for session in sessions.values() {
            if let Some(conn) = session.lock_safe().conn.as_mut() {
                if let Some(shutdown) = conn.shutdown.take() {
                    let _ = shutdown.send(ShutdownReason::BrokerStopped);
                }
            }
        }
    }

    pub fn send_to(&self, client_id: &str, packet: &Packet) -> bool {
        self.with_session(client_id, |s| s.send_packet(packet))
            .unwrap_or(false)
    }

    /// Flush messages queued while the client was offline (they were
    /// already QoS-adjusted when queued).
    pub fn flush_offline_queue(&self, client_id: &str) {
        self.with_session(client_id, |s| {
            for msg in std::mem::take(&mut s.queued) {
                s.deliver(msg);
            }
        });
    }

    pub fn store_incoming_qos2(&self, client_id: &str, packet_id: u16, publish: PublishPacket) {
        self.with_session(client_id, |s| {
            s.incoming_qos2.entry(packet_id).or_insert(publish);
        });
    }

    pub fn take_incoming_qos2(&self, client_id: &str, packet_id: u16) -> Option<QueuedMessage> {
        self.with_session(client_id, |s| {
            s.incoming_qos2
                .remove(&packet_id)
                .map(|p| QueuedMessage::from(&p))
        })
        .flatten()
    }

    /// Called on receiving a PUBREC for a QoS 2 PUBLISH we sent. Returns
    /// `true` (and transitions the pending-redelivery entry from awaiting
    /// PUBREC to awaiting PUBCOMP) if `packet_id` was one of ours — the
    /// caller should then reply with PUBREL.
    pub fn complete_outgoing_qos2(&self, client_id: &str, packet_id: u16) -> bool {
        self.with_session(client_id, |s| {
            match s.pending_redelivery.get_mut(&packet_id) {
                Some(entry) if matches!(entry.kind, PendingRedelivery::Publish(_)) => {
                    entry.kind = PendingRedelivery::PubRel;
                    entry.attempts = 0;
                    entry.last_sent = Instant::now();
                    true
                }
                _ => false,
            }
        })
        .unwrap_or(false)
    }

    /// Called on receiving a PUBACK (QoS 1) or PUBCOMP (QoS 2, final step)
    /// for a packet we sent — clears its pending-redelivery entry so
    /// [`Self::retry_pending`] stops resending it.
    pub fn clear_pending_redelivery(&self, client_id: &str, packet_id: u16) {
        self.with_session(client_id, |s| {
            s.pending_redelivery.remove(&packet_id);
        });
    }

    /// Resend, with DUP=1, every outgoing QoS 1/2 packet that's been
    /// awaiting its ack for longer than `min_age` — covers both a QoS 1/2
    /// PUBLISH awaiting PUBACK/PUBREC and a QoS 2 PUBREL awaiting PUBCOMP.
    /// Packets that have already been retried [`MAX_REDELIVERY_ATTEMPTS`]
    /// times are dropped instead (best-effort QoS beyond that point, to
    /// bound memory for a peer that's gone for good but whose session
    /// hasn't been reaped yet). Called periodically from a background task
    /// spawned in `MqttBroker::start`, with `min_age` matching that task's
    /// own tick interval (see [`MqttBrokerConfig::redelivery_interval_secs`](crate::config::MqttBrokerConfig::redelivery_interval_secs)).
    pub fn retry_pending(&self, min_age: Duration) {
        let now = Instant::now();
        let sessions = self.sessions.read_safe();
        for session in sessions.values() {
            let mut guard = session.lock_safe();
            if !guard.is_connected() {
                continue;
            }
            let due: Vec<(u16, PendingRedelivery)> = guard
                .pending_redelivery
                .iter_mut()
                .filter(|(_, entry)| now.duration_since(entry.last_sent) >= min_age)
                .filter_map(|(id, entry)| {
                    if entry.attempts >= MAX_REDELIVERY_ATTEMPTS {
                        return None; // reaped below instead of resent
                    }
                    entry.attempts += 1;
                    entry.last_sent = now;
                    Some((*id, entry.kind.clone()))
                })
                .collect();
            guard
                .pending_redelivery
                .retain(|_, entry| entry.attempts < MAX_REDELIVERY_ATTEMPTS);

            for (packet_id, kind) in due {
                let packet = match kind {
                    PendingRedelivery::Publish(msg) => {
                        Packet::Publish(msg.to_publish(Some(packet_id), true))
                    }
                    PendingRedelivery::PubRel => Packet::PubRel(SimpleAck::success(packet_id)),
                };
                guard.send_packet(&packet);
            }
        }
    }

    pub fn add_subscription(&self, client_id: &str, filter: &str, sub: Subscription) -> bool {
        self.with_session(client_id, |s| {
            let is_new = s.subscriptions.insert(filter.to_string(), sub).is_none();
            if is_new {
                if let Some(entry) = IndexEntry::from_key(client_id, filter) {
                    self.index.write_safe().insert(entry);
                }
            }
            is_new
        })
        .unwrap_or(false)
    }

    /// Remove a subscription. Returns whether it existed.
    pub fn remove_subscription(&self, client_id: &str, filter: &str) -> bool {
        self.with_session(client_id, |s| {
            let existed = s.subscriptions.remove(filter).is_some();
            if existed {
                self.index.write_safe().remove(client_id, filter);
            }
            existed
        })
        .unwrap_or(false)
    }

    /// Discard persistent sessions that have been offline for at least `ttl`
    /// (`MqttBrokerConfig::session_expiry_secs`) — otherwise every
    /// `clean_start = false` client that never returns (random client ids are
    /// the usual culprit) would stay in memory forever. Returns how many were
    /// removed.
    pub fn sweep_expired(&self, ttl: Duration) -> usize {
        self.sweep_expired_at(Instant::now(), ttl)
    }

    fn sweep_expired_at(&self, now: Instant, ttl: Duration) -> usize {
        let mut sessions = self.sessions.write_safe();
        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| {
                let s = s.lock_safe();
                !s.is_connected()
                    && s.offline_since
                        .is_some_and(|since| now.saturating_duration_since(since) >= ttl)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            if let Some(removed) = sessions.remove(id) {
                self.unindex_session(id, &removed.lock_safe());
            }
        }
        expired.len()
    }

    pub fn discard_will(&self, client_id: &str) {
        self.with_session(client_id, |s| s.will = None);
    }

    /// Send a retained message directly to one (just-subscribed) client,
    /// downgrading QoS to the subscription's granted maximum.
    pub fn send_retained(&self, client_id: &str, topic: String, payload: Bytes, qos: QoS) {
        self.with_session(client_id, |s| {
            s.deliver(QueuedMessage {
                topic,
                payload,
                qos,
                retain: true,
            })
        });
    }

    /// Deliver `message` (published by `publisher_id`) to every matching
    /// subscriber, live or queued for later delivery.
    ///
    /// Subscribers come from the [`SubscriptionIndex`], not a scan of every
    /// session. A client with several matching filters still gets the
    /// message once, at the highest QoS among them. Shared subscriptions
    /// (`$share/<group>/<filter>`) deliver to **one** member per group,
    /// rotating round-robin and preferring connected members.
    pub fn fan_out(&self, publisher_id: &str, message: &QueuedMessage) {
        // Copy the matches out so `index` is released before any session lock.
        let matches = self.index.read_safe().matching(&message.topic);
        if matches.is_empty() {
            return;
        }

        let mut targets: HashMap<String, Vec<String>> = HashMap::new();
        let mut groups: HashMap<(String, String), Vec<IndexEntry>> = HashMap::new();
        for entry in matches {
            match &entry.group {
                None => targets.entry(entry.client_id).or_default().push(entry.key),
                Some(group) => groups
                    .entry((group.clone(), entry.filter.clone()))
                    .or_default()
                    .push(entry),
            }
        }

        let sessions = self.sessions.read_safe();
        for members in groups.into_values() {
            let start = self.shared_cursor.fetch_add(1, Ordering::Relaxed) % members.len();
            let rotated = (0..members.len()).map(|i| &members[(start + i) % members.len()]);
            let pick = rotated
                .clone()
                .find(|m| {
                    sessions
                        .get(&m.client_id)
                        .is_some_and(|s| s.lock_safe().is_connected())
                })
                .unwrap_or(&members[start]);
            targets
                .entry(pick.client_id.clone())
                .or_default()
                .push(pick.key.clone());
        }

        for (client_id, keys) in targets {
            let Some(session) = sessions.get(&client_id) else {
                continue; // dropped since the index was read
            };
            let mut guard = session.lock_safe();
            let best = keys
                .iter()
                .filter_map(|key| guard.subscriptions.get(key))
                .filter(|sub| !(client_id == publisher_id && sub.no_local))
                .max_by_key(|sub| sub.qos)
                .cloned();
            let Some(sub) = best else { continue };

            let outgoing = QueuedMessage {
                topic: message.topic.clone(),
                payload: message.payload.clone(),
                qos: message.qos.min(sub.qos),
                retain: message.retain && sub.retain_as_published,
            };
            if guard.is_connected() {
                guard.deliver(outgoing);
            } else if !guard.clean_start {
                guard.enqueue_offline(outgoing);
            }
        }
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RedeliveryEntry;

    /// The test-side ends of a connection's channels. Keeping the writer
    /// receiver alive matters: dropping it closes the channel, which makes
    /// `send_to` report the connection as gone.
    struct Peer {
        shutdown_rx: oneshot::Receiver<ShutdownReason>,
        _writer_rx: mpsc::Receiver<Bytes>,
    }

    fn request(client_id: &str, clean_start: bool, max_clients: u32) -> (AttachRequest, Peer) {
        let (sender, writer_rx) = mpsc::channel(8);
        let (shutdown, shutdown_rx) = oneshot::channel();
        (
            AttachRequest {
                client_id: client_id.into(),
                version: MqttVersion::V5,
                clean_start,
                will: None,
                max_queued: 10,
                max_clients,
                sender,
                shutdown,
            },
            Peer {
                shutdown_rx,
                _writer_rx: writer_rx,
            },
        )
    }

    #[test]
    fn evicted_connection_cannot_detach_its_replacement() {
        let registry = SessionRegistry::new();
        let (req, mut first_peer) = request("c", true, 0);
        let first = registry.attach(req).unwrap();
        let (req, _second_peer) = request("c", true, 0);
        let second = registry.attach(req).unwrap();

        // The first connection was told to go away...
        assert_eq!(
            first_peer.shutdown_rx.try_recv(),
            Ok(ShutdownReason::TakenOver),
            "takeover must signal the old connection"
        );
        // ...and its late teardown must not touch the new connection.
        assert!(registry.detach("c", first.conn_id, false).is_none());
        assert_eq!(registry.client_count(), 1, "replacement still counted");
        assert!(registry.send_to("c", &Packet::PingResp), "still attached");

        registry.detach("c", second.conn_id, true);
        assert_eq!(registry.client_count(), 0);
    }

    #[test]
    fn quota_ignores_takeovers_but_blocks_new_clients() {
        let registry = SessionRegistry::new();
        let (req, _a) = request("a", true, 1);
        registry.attach(req).unwrap();
        let (req, _b) = request("b", true, 1);
        assert_eq!(
            registry.attach(req).unwrap_err(),
            AttachError::QuotaExceeded
        );
        let (req, _a2) = request("a", true, 1);
        assert!(registry.attach(req).is_ok(), "takeover is not a new client");
        assert_eq!(registry.client_count(), 1);
    }

    #[test]
    fn persistent_session_is_resumed_and_survives_detach() {
        let registry = SessionRegistry::new();
        let (req, _a) = request("p", false, 0);
        let first = registry.attach(req).unwrap();
        assert!(!first.session_present);
        registry.detach("p", first.conn_id, true);
        let (req, _b) = request("p", false, 0);
        assert!(registry.attach(req).unwrap().session_present);
    }

    #[test]
    fn full_outbound_queue_drops_instead_of_growing() {
        let registry = SessionRegistry::new();
        let (req, _peer) = request("slow", true, 0); // capacity 8, never drained
        registry.attach(req).unwrap();
        let sent = (0..20)
            .filter(|_| registry.send_to("slow", &Packet::PingResp))
            .count();
        assert_eq!(sent, 8, "only the channel capacity is buffered");
    }

    #[test]
    fn shutdown_all_signals_every_live_connection() {
        let registry = SessionRegistry::new();
        let (req, mut a) = request("a", true, 0);
        registry.attach(req).unwrap();
        let (req, mut b) = request("b", true, 0);
        registry.attach(req).unwrap();
        registry.shutdown_all();
        assert_eq!(a.shutdown_rx.try_recv(), Ok(ShutdownReason::BrokerStopped));
        assert_eq!(b.shutdown_rx.try_recv(), Ok(ShutdownReason::BrokerStopped));
    }

    #[test]
    fn expired_sessions_are_removed_with_their_index_entries() {
        let registry = SessionRegistry::new();
        let (req, _a) = request("ghost", false, 0);
        let attached = registry.attach(req).unwrap();
        registry.add_subscription(
            "ghost",
            "t/#",
            Subscription {
                qos: QoS::AtLeastOnce,
                no_local: false,
                retain_as_published: false,
            },
        );
        registry.detach("ghost", attached.conn_id, true);

        let ttl = Duration::from_secs(60);
        let now = Instant::now();
        assert_eq!(registry.sweep_expired_at(now, ttl), 0, "not old enough yet");
        assert_eq!(
            registry.sweep_expired_at(now + Duration::from_secs(61), ttl),
            1
        );
        assert!(registry.index.read_safe().matching("t/x").is_empty());
        assert_eq!(
            registry.sweep_expired_at(now + Duration::from_secs(120), ttl),
            0
        );
    }

    #[test]
    fn connected_sessions_never_expire() {
        let registry = SessionRegistry::new();
        let (req, _a) = request("alive", false, 0);
        registry.attach(req).unwrap();
        let far = Instant::now() + Duration::from_secs(10_000);
        assert_eq!(registry.sweep_expired_at(far, Duration::from_secs(1)), 0);
    }

    #[test]
    fn dropped_clean_session_leaves_nothing_in_the_index() {
        let registry = SessionRegistry::new();
        let (req, _a) = request("clean", true, 0);
        let attached = registry.attach(req).unwrap();
        registry.add_subscription(
            "clean",
            "$share/g/a/+",
            Subscription {
                qos: QoS::AtMostOnce,
                no_local: false,
                retain_as_published: false,
            },
        );
        assert_eq!(registry.index.read_safe().matching("a/b").len(), 1);
        registry.detach("clean", attached.conn_id, true);
        assert!(registry.index.read_safe().matching("a/b").is_empty());
    }

    #[test]
    fn packet_ids_skip_in_flight_entries() {
        let mut s = Session::new("c".into(), MqttVersion::V5, true, 0);
        let first = s.alloc_packet_id();
        s.pending_redelivery.insert(
            first.wrapping_add(1),
            RedeliveryEntry {
                kind: PendingRedelivery::PubRel,
                attempts: 0,
                last_sent: Instant::now(),
            },
        );
        let next = s.alloc_packet_id();
        assert_ne!(next, first.wrapping_add(1));
        assert_ne!(next, 0);
    }
}
