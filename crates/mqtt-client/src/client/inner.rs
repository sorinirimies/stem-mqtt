//! Live-connection state ([`Inner`]) and the small helpers that operate on
//! it directly: packet-id allocation, the pending-ack map used to resolve
//! [`crate::client::MqttClient::publish`]/`subscribe`/`unsubscribe` futures,
//! and QoS 2 inbound dedup bookkeeping. Wire I/O (reading/writing packets,
//! the background read/keep-alive loops) lives in
//! [`crate::client::io`], which only ever touches `Inner` through the
//! helpers defined here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

use crate::error::{MqttError, MqttResult};
use crate::protocol::packet::Packet;
use crate::protocol::MqttVersion;
use crate::support::{guard_callback, EventQueue, LockExt};

use super::tls::Transport;
use super::types::{MqttMessage, MqttMessageListener};

/// How incoming messages and disconnects reach the application, shared
/// between the [`crate::client::MqttClient`] handle and every live connection
/// it ever creates. A single shared object (instead of a copy handed over at
/// connect time) means `set_message_listener` takes effect immediately
/// whether it races with `connect()`, runs before it, or runs after it — and
/// survives reconnects.
///
/// Two delivery styles coexist: the push-style `listener` callback, and a
/// pull-style `queue` for runtimes that can't receive callbacks from Rust
/// threads (see [`crate::support::EventQueue`]).
#[derive(Default)]
pub(super) struct Delivery {
    pub listener: Mutex<Option<Arc<dyn MqttMessageListener>>>,
    pub queue: EventQueue<MqttMessage>,
    pub last_disconnect: Mutex<Option<String>>,
}

pub(super) type ListenerSlot = Arc<Delivery>;

/// Everything about one live connection: the write half, in-flight
/// operations awaiting a broker ack, the registered listener, and the
/// background tasks (read loop, keep-alive) that keep it all moving.
pub(super) struct Inner {
    pub version: MqttVersion,
    pub writer: AsyncMutex<Option<WriteHalf<Box<dyn Transport>>>>,
    next_packet_id: AtomicU16,
    pub pending: Mutex<HashMap<u16, oneshot::Sender<Packet>>>,
    /// QoS 2 PUBLISHes received from the broker, held until the matching
    /// PUBREL arrives (MQTT-5.0 §4.3.3): dedups redelivery and defers
    /// handing the message to the listener until the handshake completes.
    pub incoming_qos2: Mutex<HashMap<u16, MqttMessage>>,
    listener: ListenerSlot,
    pub connected: AtomicBool,
    pub read_task: AsyncMutex<Option<JoinHandle<()>>>,
    pub keepalive_task: AsyncMutex<Option<JoinHandle<()>>>,
    pub operation_timeout: Duration,
    /// When the last packet of *any* kind arrived from the broker; the
    /// keep-alive loop uses it to detect a half-open connection.
    last_rx: Mutex<Instant>,
}

impl Inner {
    pub fn new(
        version: MqttVersion,
        writer: WriteHalf<Box<dyn Transport>>,
        listener: ListenerSlot,
        operation_timeout: Duration,
    ) -> Self {
        Inner {
            version,
            writer: AsyncMutex::new(Some(writer)),
            next_packet_id: AtomicU16::new(1),
            pending: Mutex::new(HashMap::new()),
            incoming_qos2: Mutex::new(HashMap::new()),
            listener,
            connected: AtomicBool::new(true),
            read_task: AsyncMutex::new(None),
            keepalive_task: AsyncMutex::new(None),
            operation_timeout,
            last_rx: Mutex::new(Instant::now()),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Next non-zero packet id that isn't already awaiting an ack
    /// (MQTT-2.2.1-3: ids are non-zero, and one may not be reused while its
    /// exchange is still in flight).
    pub fn alloc_packet_id(&self) -> u16 {
        let pending = self.pending.lock_safe();
        let mut id = 0;
        for _ in 0..=u16::MAX {
            id = self.next_packet_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 && !pending.contains_key(&id) {
                break;
            }
        }
        id
    }

    pub fn take_listener(&self) -> Option<Arc<dyn MqttMessageListener>> {
        self.listener.listener.lock_safe().clone()
    }

    /// Record that the broker just sent us something.
    pub fn touch(&self) {
        *self.last_rx.lock_safe() = Instant::now();
    }

    /// How long since the broker last sent us anything.
    pub fn idle_for(&self) -> Duration {
        self.last_rx.lock_safe().elapsed()
    }

    /// Best-effort synchronous teardown (used from `Drop` and from sync
    /// contexts): mark disconnected, drop the write half, abort the
    /// background tasks. Never blocks; a lock that's busy is skipped (its
    /// holder is already in the middle of tearing the connection down).
    pub fn shutdown_now(&self) {
        self.connected.store(false, Ordering::Relaxed);
        self.pending.lock_safe().clear();
        if let Ok(mut writer) = self.writer.try_lock() {
            writer.take();
        }
        for task in [&self.read_task, &self.keepalive_task] {
            if let Ok(mut task) = task.try_lock() {
                if let Some(handle) = task.take() {
                    handle.abort();
                }
            }
        }
    }

    /// Graceful asynchronous teardown: flush-close the socket and abort the
    /// background tasks. Safe to call on an already-dead connection.
    pub async fn shutdown(&self) {
        self.connected.store(false, Ordering::Relaxed);
        self.pending.lock_safe().clear();
        if let Some(mut w) = self.writer.lock().await.take() {
            let _ = w.shutdown().await;
        }
        for task in [&self.read_task, &self.keepalive_task] {
            if let Some(handle) = task.lock().await.take() {
                handle.abort();
            }
        }
    }
}

/// Register a packet id as awaiting a broker ack; the returned receiver
/// resolves once [`crate::client::io::handle_incoming`] sees the matching
/// PUBACK/PUBREC/PUBCOMP/SUBACK/UNSUBACK and calls [`resolve_pending`].
pub(super) fn register_pending(inner: &Arc<Inner>, id: u16) -> oneshot::Receiver<Packet> {
    let (tx, rx) = oneshot::channel();
    inner.pending.lock_safe().insert(id, tx);
    rx
}

/// Forget a [`register_pending`]ed id whose request never made it onto the
/// wire (or whose wait ended). Without this, every timed-out or failed
/// request would leave a dead entry in the map forever.
pub(super) fn forget_pending(inner: &Arc<Inner>, id: u16) {
    inner.pending.lock_safe().remove(&id);
}

/// Resolve a previously-[`register_pending`]ed packet id with the ack
/// packet that just arrived. A no-op if nothing is waiting on `id`
/// (e.g. a duplicate ack after we already timed out).
pub(super) fn resolve_pending(inner: &Arc<Inner>, id: u16, packet: Packet) {
    if let Some(tx) = inner.pending.lock_safe().remove(&id) {
        let _ = tx.send(packet);
    }
}

/// Await a [`register_pending`] receiver, applying the connection's
/// configured operation timeout. On timeout the pending entry is removed.
pub(super) async fn wait_for(
    inner: &Arc<Inner>,
    id: u16,
    rx: oneshot::Receiver<Packet>,
) -> MqttResult<Packet> {
    match tokio::time::timeout(inner.operation_timeout, rx).await {
        Ok(Ok(packet)) => Ok(packet),
        Ok(Err(_)) => Err(MqttError::Session(
            "connection closed while waiting for ack".into(),
        )),
        Err(_) => {
            forget_pending(inner, id);
            Err(MqttError::Timeout)
        }
    }
}

pub(super) fn unexpected(packet: &Packet) -> MqttError {
    MqttError::MalformedPacket(format!("unexpected packet while awaiting ack: {packet:?}"))
}

/// Hand a delivered message to the registered listener, if any. A panic in
/// the foreign callback is contained (see [`guard_callback`]) so it can't
/// kill the read loop.
pub(super) fn deliver(inner: &Arc<Inner>, message: MqttMessage) {
    if inner.listener.queue.is_enabled() {
        inner.listener.queue.push(message.clone());
    }
    if let Some(listener) = inner.take_listener() {
        guard_callback("on_message", || listener.on_message(message));
    }
}

/// Mark the connection dead, fail every in-flight request immediately
/// (rather than letting each wait out its full timeout), and notify the
/// listener — exactly once, however many of the read loop, the keep-alive
/// loop, and a failed write race to report the same loss.
pub(super) fn finish_disconnected(inner: &Arc<Inner>, reason: String) {
    let was_connected = inner.connected.swap(false, Ordering::Relaxed);
    // Dropping the senders wakes every `wait_for` with "connection closed".
    inner.pending.lock_safe().clear();
    if !was_connected {
        return;
    }
    *inner.listener.last_disconnect.lock_safe() = Some(reason.clone());
    if let Some(listener) = inner.take_listener() {
        guard_callback("on_disconnected", || listener.on_disconnected(reason));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QoS;
    use std::sync::atomic::AtomicUsize;

    fn inner(timeout: Duration) -> (Arc<Inner>, ListenerSlot) {
        let (a, _b) = tokio::io::duplex(64);
        let transport: Box<dyn Transport> = Box::new(a);
        let (_r, w) = tokio::io::split(transport);
        let slot: ListenerSlot = Arc::default();
        (
            Arc::new(Inner::new(MqttVersion::V5, w, slot.clone(), timeout)),
            slot,
        )
    }

    #[derive(Default)]
    struct Recorder {
        messages: AtomicUsize,
        disconnects: Mutex<Vec<String>>,
    }
    impl MqttMessageListener for Recorder {
        fn on_message(&self, _: MqttMessage) {
            self.messages.fetch_add(1, Ordering::Relaxed);
        }
        fn on_disconnected(&self, reason: String) {
            self.disconnects.lock_safe().push(reason);
        }
    }

    fn message() -> MqttMessage {
        MqttMessage {
            topic: "t".into(),
            payload: vec![1],
            qos: QoS::AtMostOnce,
            retain: false,
        }
    }

    #[tokio::test]
    async fn packet_ids_are_nonzero_and_skip_in_flight() {
        let (inner, _) = inner(Duration::from_secs(1));
        let _rx = register_pending(&inner, 2);
        let ids: Vec<u16> = (0..4).map(|_| inner.alloc_packet_id()).collect();
        assert!(ids.iter().all(|&i| i != 0));
        assert!(!ids.contains(&2), "in-flight id must not be reused");
    }

    #[tokio::test]
    async fn packet_ids_wrap_past_zero() {
        let (inner, _) = inner(Duration::from_secs(1));
        inner.next_packet_id.store(u16::MAX, Ordering::Relaxed);
        assert_eq!(inner.alloc_packet_id(), u16::MAX);
        assert_eq!(inner.alloc_packet_id(), 1, "0 is skipped on wrap");
    }

    #[tokio::test]
    async fn resolve_completes_the_waiter() {
        let (inner, _) = inner(Duration::from_secs(1));
        let rx = register_pending(&inner, 7);
        resolve_pending(&inner, 7, Packet::PingResp);
        assert!(matches!(
            wait_for(&inner, 7, rx).await,
            Ok(Packet::PingResp)
        ));
        resolve_pending(&inner, 7, Packet::PingResp); // duplicate ack: no-op
    }

    #[tokio::test(start_paused = true)]
    async fn wait_times_out_and_forgets_the_id() {
        let (inner, _) = inner(Duration::from_millis(50));
        let rx = register_pending(&inner, 9);
        assert!(matches!(
            wait_for(&inner, 9, rx).await,
            Err(MqttError::Timeout)
        ));
        assert!(inner.pending.lock_safe().is_empty());
    }

    #[tokio::test]
    async fn disconnect_fails_waiters_and_notifies_exactly_once() {
        let (inner, slot) = inner(Duration::from_secs(5));
        let recorder = Arc::new(Recorder::default());
        *slot.listener.lock_safe() = Some(recorder.clone());
        let rx = register_pending(&inner, 3);

        finish_disconnected(&inner, "lost".into());
        finish_disconnected(&inner, "again".into());

        assert!(matches!(
            wait_for(&inner, 3, rx).await,
            Err(MqttError::Session(_))
        ));
        assert!(!inner.is_connected());
        assert_eq!(*recorder.disconnects.lock_safe(), ["lost"]);
        assert_eq!(slot.last_disconnect.lock_safe().as_deref(), Some("lost"));
    }

    #[tokio::test]
    async fn deliver_reaches_listener_and_pull_queue() {
        let (inner, slot) = inner(Duration::from_secs(1));
        let recorder = Arc::new(Recorder::default());
        *slot.listener.lock_safe() = Some(recorder.clone());
        slot.queue.enable(4);
        deliver(&inner, message());
        assert_eq!(recorder.messages.load(Ordering::Relaxed), 1);
        assert!(slot.queue.is_enabled());
    }

    #[tokio::test]
    async fn shutdown_marks_disconnected_and_drops_writer() {
        let (inner, _) = inner(Duration::from_secs(1));
        inner.shutdown().await;
        assert!(!inner.is_connected());
        assert!(inner.writer.lock().await.is_none());
    }

    #[tokio::test]
    async fn idle_time_resets_on_touch() {
        let (inner, _) = inner(Duration::from_secs(1));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(inner.idle_for() >= Duration::from_millis(20));
        inner.touch();
        assert!(inner.idle_for() < Duration::from_millis(20));
    }
}
