//! Session registry: owns the `client_id -> Session` map and every
//! operation that only needs that map — attach/detach on (re)connect,
//! per-client send/queue/QoS-2 bookkeeping, and fanning a published
//! message out to matching subscribers. Retained-message storage
//! ([`crate::retain::RetainStore`]) and event notification
//! ([`crate::events::EventHub`]) are separate concerns the broker
//! composes alongside this registry rather than folding into it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
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
    LiveConn, OutboundBudget, PendingRedelivery, Qos0Frames, QueuedMessage, Session,
    ShutdownReason, Subscription, MAX_INCOMING_QOS2, MAX_INCOMING_QOS2_BYTES,
    MAX_SUBSCRIPTIONS_PER_SESSION,
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
    /// Byte budget for that channel (shared with the writer task).
    pub budget: OutboundBudget,
    /// Lets the registry (eviction, broker stop) shut this connection down.
    pub shutdown: oneshot::Sender<ShutdownReason>,
}

/// Why [`SessionRegistry::attach`] refused a connection.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachError {
    /// `max_clients` simultaneous connections are already live.
    QuotaExceeded,
    /// The broker is stopping (or stopped); no new connections are taken.
    Stopped,
}

/// Result of [`SessionRegistry::add_subscription`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SubscribeOutcome {
    /// A subscription that didn't exist before.
    New,
    /// An existing subscription had its options replaced.
    Replaced,
    /// Refused: the session is at [`MAX_SUBSCRIPTIONS_PER_SESSION`].
    LimitReached,
    /// The client has no session (already gone).
    NoSession,
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
    /// Set by [`Self::shutdown_all`]; makes [`Self::attach`] refuse, so a
    /// connection that finishes its handshake after `stop()` can't slip in.
    closed: AtomicBool,
}

impl SessionRegistry {
    pub fn new() -> Self {
        SessionRegistry {
            sessions: RwLock::new(HashMap::new()),
            client_count: AtomicU32::new(0),
            next_conn_id: AtomicU64::new(1),
            index: RwLock::new(SubscriptionIndex::default()),
            shared_cursor: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        }
    }

    /// Remove every subscription of a discarded session from the index.
    fn unindex_session(&self, client_id: &str, session: &Session) {
        let mut index = self.index.write_safe();
        for key in session.subscriptions.keys() {
            index.remove(client_id, key);
        }
    }

    /// Accept connections again (the broker was restarted).
    pub fn reopen(&self) {
        self.closed.store(false, Ordering::Release);
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
        if self.closed.load(Ordering::Acquire) {
            return Err(AttachError::Stopped);
        }
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
                budget: req.budget,
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
        // Flag first: an `attach` that runs after this sees it and refuses; one
        // that already ran is in the map and gets signalled below.
        self.closed.store(true, Ordering::Release);
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

    /// Hold an inbound QoS 2 PUBLISH until its PUBREL. Returns `false` when
    /// the session is already holding too many (count or bytes) — a protocol
    /// violation by the client (Receive Maximum exceeded) the caller should
    /// answer by dropping the connection. A retransmission of a held id is
    /// accepted and deduplicated.
    #[must_use]
    pub fn store_incoming_qos2(
        &self,
        client_id: &str,
        packet_id: u16,
        publish: PublishPacket,
    ) -> bool {
        self.with_session(client_id, |s| {
            if s.incoming_qos2.contains_key(&packet_id) {
                return true;
            }
            if s.incoming_qos2.len() >= MAX_INCOMING_QOS2
                || s.incoming_qos2_bytes + publish.payload.len() > MAX_INCOMING_QOS2_BYTES
            {
                return false;
            }
            s.incoming_qos2_bytes += publish.payload.len();
            s.incoming_qos2.insert(packet_id, publish);
            true
        })
        .unwrap_or(true)
    }

    pub fn take_incoming_qos2(&self, client_id: &str, packet_id: u16) -> Option<QueuedMessage> {
        self.with_session(client_id, |s| {
            s.incoming_qos2.remove(&packet_id).map(|p| {
                s.incoming_qos2_bytes = s.incoming_qos2_bytes.saturating_sub(p.payload.len());
                QueuedMessage::from(&p)
            })
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

    /// Add (or replace) a subscription, refusing a *new* one once the
    /// session already holds [`MAX_SUBSCRIPTIONS_PER_SESSION`].
    pub fn add_subscription(
        &self,
        client_id: &str,
        filter: &str,
        sub: Subscription,
    ) -> SubscribeOutcome {
        self.with_session(client_id, |s| {
            if !s.subscriptions.contains_key(filter)
                && s.subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_SESSION
            {
                return SubscribeOutcome::LimitReached;
            }
            let is_new = s.subscriptions.insert(filter.to_string(), sub).is_none();
            if is_new {
                if let Some(entry) = IndexEntry::from_key(client_id, filter) {
                    self.index.write_safe().insert(entry);
                }
                SubscribeOutcome::New
            } else {
                SubscribeOutcome::Replaced
            }
        })
        .unwrap_or(SubscribeOutcome::NoSession)
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

        // Borrow from `matches` (refcounted entries) — no per-publish string clones.
        let mut targets: HashMap<&str, Vec<&str>> = HashMap::with_capacity(matches.len());
        let mut groups: HashMap<(&str, &str), Vec<&IndexEntry>> = HashMap::new();
        for entry in &matches {
            match entry.group.as_deref() {
                None => targets
                    .entry(&entry.client_id)
                    .or_default()
                    .push(&entry.key),
                Some(group) => groups
                    .entry((group, &entry.filter))
                    .or_default()
                    .push(entry),
            }
        }

        let sessions = self.sessions.read_safe();
        for members in groups.into_values() {
            let start = self.shared_cursor.fetch_add(1, Ordering::Relaxed) % members.len();
            let pick = (0..members.len())
                .map(|i| members[(start + i) % members.len()])
                .find(|m| {
                    sessions
                        .get(&m.client_id)
                        .is_some_and(|s| s.lock_safe().is_connected())
                })
                .unwrap_or(members[start]);
            targets.entry(&pick.client_id).or_default().push(&pick.key);
        }

        let mut frames = Qos0Frames::default();
        for (client_id, keys) in targets {
            let Some(session) = sessions.get(client_id) else {
                continue; // dropped since the index was read
            };
            let mut guard = session.lock_safe();
            let best = keys
                .iter()
                .filter_map(|key| guard.subscriptions.get(*key))
                .filter(|sub| !(client_id == publisher_id && sub.no_local))
                .max_by_key(|sub| sub.qos)
                .cloned();
            let Some(sub) = best else { continue };

            let qos = message.qos.min(sub.qos);
            let retain = message.retain && sub.retain_as_published;
            if guard.is_connected() {
                guard.deliver_adjusted(message, qos, retain, &mut frames);
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
        writer_rx: mpsc::Receiver<Bytes>,
    }

    impl Peer {
        /// Frames delivered to this connection so far.
        fn received(&mut self) -> usize {
            std::iter::from_fn(|| self.writer_rx.try_recv().ok()).count()
        }
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
                budget: OutboundBudget::new(1 << 20),
                shutdown,
            },
            Peer {
                shutdown_rx,
                writer_rx,
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

    fn sub(qos: QoS, no_local: bool) -> Subscription {
        Subscription {
            qos,
            no_local,
            retain_as_published: false,
        }
    }

    fn connect(registry: &SessionRegistry, id: &str) -> Peer {
        let (req, peer) = request(id, true, 0);
        registry.attach(req).unwrap();
        peer
    }

    fn message(topic: &str, qos: QoS) -> QueuedMessage {
        QueuedMessage {
            topic: topic.into(),
            payload: Bytes::from_static(b"x"),
            qos,
            retain: false,
        }
    }

    #[test]
    fn fan_out_delivers_once_per_client_across_overlapping_filters() {
        let registry = SessionRegistry::new();
        let mut peer = connect(&registry, "c");
        registry.add_subscription("c", "a/+", sub(QoS::AtMostOnce, false));
        registry.add_subscription("c", "a/#", sub(QoS::AtLeastOnce, false));
        registry.fan_out("other", &message("a/b", QoS::ExactlyOnce));
        assert_eq!(peer.received(), 1, "one copy despite two matching filters");
        // Highest matching QoS (1) wins, so a pending QoS 1 redelivery exists.
        let pending = registry.with_session("c", |s| s.pending_redelivery.len());
        assert_eq!(pending, Some(1));
    }

    #[test]
    fn fan_out_honours_no_local() {
        let registry = SessionRegistry::new();
        let mut me = connect(&registry, "me");
        let mut you = connect(&registry, "you");
        registry.add_subscription("me", "t", sub(QoS::AtMostOnce, true));
        registry.add_subscription("you", "t", sub(QoS::AtMostOnce, true));
        registry.fan_out("me", &message("t", QoS::AtMostOnce));
        assert_eq!(me.received(), 0, "no_local suppresses the publisher's echo");
        assert_eq!(you.received(), 1);
    }

    #[test]
    fn fan_out_shared_group_picks_one_member_round_robin() {
        let registry = SessionRegistry::new();
        let mut a = connect(&registry, "a");
        let mut b = connect(&registry, "b");
        for id in ["a", "b"] {
            registry.add_subscription(id, "$share/g/jobs", sub(QoS::AtMostOnce, false));
        }
        for _ in 0..4 {
            registry.fan_out("p", &message("jobs", QoS::AtMostOnce));
        }
        let (ra, rb) = (a.received(), b.received());
        assert_eq!(ra + rb, 4, "each message goes to exactly one member");
        assert_eq!((ra, rb), (2, 2), "members take turns");
    }

    #[test]
    fn fan_out_queues_for_offline_persistent_sessions_only() {
        let registry = SessionRegistry::new();
        let (req, _peer) = request("p", false, 0);
        let conn = registry.attach(req).unwrap();
        registry.add_subscription("p", "t", sub(QoS::AtLeastOnce, false));
        registry.detach("p", conn.conn_id, true);
        registry.fan_out("x", &message("t", QoS::AtLeastOnce));
        assert_eq!(registry.with_session("p", |s| s.queued.len()), Some(1));
    }

    #[test]
    fn fan_out_ignores_unmatched_topics() {
        let registry = SessionRegistry::new();
        let mut peer = connect(&registry, "c");
        registry.add_subscription("c", "a/b", sub(QoS::AtMostOnce, false));
        registry.fan_out("x", &message("a/c", QoS::AtMostOnce));
        assert_eq!(peer.received(), 0);
    }

    #[test]
    fn subscription_count_is_capped_per_session() {
        let registry = SessionRegistry::new();
        let _peer = connect(&registry, "greedy");
        for n in 0..MAX_SUBSCRIPTIONS_PER_SESSION {
            let outcome =
                registry.add_subscription("greedy", &format!("t/{n}"), sub(QoS::AtMostOnce, false));
            assert_eq!(outcome, SubscribeOutcome::New);
        }
        assert_eq!(
            registry.add_subscription("greedy", "one/too/many", sub(QoS::AtMostOnce, false)),
            SubscribeOutcome::LimitReached
        );
        // Re-subscribing to an existing filter is still fine.
        assert_eq!(
            registry.add_subscription("greedy", "t/0", sub(QoS::AtLeastOnce, false)),
            SubscribeOutcome::Replaced
        );
        assert!(registry
            .index
            .read_safe()
            .matching("one/too/many")
            .is_empty());
    }

    fn qos2_publish(id: u16, payload_len: usize) -> PublishPacket {
        PublishPacket {
            dup: false,
            qos: QoS::ExactlyOnce,
            retain: false,
            topic: "t".into(),
            packet_id: Some(id),
            payload: Bytes::from(vec![0; payload_len]),
            properties: mqtt_client::protocol::properties::Properties::new(),
        }
    }

    #[test]
    fn inbound_qos2_is_bounded_by_count_and_bytes() {
        let registry = SessionRegistry::new();
        let _peer = connect(&registry, "c");
        for id in 1..=MAX_INCOMING_QOS2 as u16 {
            assert!(registry.store_incoming_qos2("c", id, qos2_publish(id, 1)));
        }
        let over = MAX_INCOMING_QOS2 as u16 + 1;
        assert!(!registry.store_incoming_qos2("c", over, qos2_publish(over, 1)));
        // A retransmission of a held id is deduplicated, not refused.
        assert!(registry.store_incoming_qos2("c", 1, qos2_publish(1, 1)));
        // Releasing one frees a slot.
        assert!(registry.take_incoming_qos2("c", 1).is_some());
        assert!(registry.store_incoming_qos2("c", over, qos2_publish(over, 1)));

        let registry = SessionRegistry::new();
        let _peer = connect(&registry, "big");
        assert!(registry.store_incoming_qos2("big", 1, qos2_publish(1, MAX_INCOMING_QOS2_BYTES)));
        assert!(
            !registry.store_incoming_qos2("big", 2, qos2_publish(2, 1)),
            "byte budget spent"
        );
        registry.take_incoming_qos2("big", 1);
        assert!(
            registry.store_incoming_qos2("big", 2, qos2_publish(2, 1)),
            "bytes released"
        );
    }

    #[test]
    fn attach_is_refused_after_shutdown_until_reopened() {
        let registry = SessionRegistry::new();
        registry.shutdown_all();
        let (req, _peer) = request("late", true, 0);
        assert_eq!(registry.attach(req).unwrap_err(), AttachError::Stopped);
        assert_eq!(registry.client_count(), 0);
        registry.reopen();
        let (req, _peer) = request("late", true, 0);
        assert!(registry.attach(req).is_ok());
    }
}
