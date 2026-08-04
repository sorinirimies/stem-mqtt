//! Live-connection state ([`Inner`]) and the small helpers that operate on
//! it directly: packet-id allocation, the pending-ack map used to resolve
//! [`crate::client::MqttClient::publish`]/`subscribe`/`unsubscribe` futures,
//! and QoS 2 inbound dedup bookkeeping. Wire I/O (reading/writing packets,
//! the background read/keep-alive loops) lives in
//! [`crate::client::io`], which only ever touches `Inner` through the
//! helpers defined here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

use crate::error::{MqttError, MqttResult};
use crate::protocol::packet::Packet;
use crate::protocol::MqttVersion;

use super::types::{MqttMessage, MqttMessageListener};

/// QoS 2 PUBLISHes received from the broker, held until the matching
/// PUBREL arrives (MQTT-5.0 §4.3.3): dedups redelivery and defers handing
/// the message to the listener until the handshake completes.
pub(super) struct PendingQos2Incoming {
    pub messages: HashMap<u16, MqttMessage>,
}

/// Everything about one live connection: the write half, in-flight
/// operations awaiting a broker ack, the registered listener, and the
/// background tasks (read loop, keep-alive) that keep it all moving.
pub(super) struct Inner {
    pub version: MqttVersion,
    pub writer: AsyncMutex<Option<OwnedWriteHalf>>,
    next_packet_id: AtomicU16,
    pub pending: std::sync::Mutex<HashMap<u16, oneshot::Sender<Packet>>>,
    pub incoming_qos2: std::sync::Mutex<PendingQos2Incoming>,
    pub listener: std::sync::Mutex<Option<Arc<dyn MqttMessageListener>>>,
    pub connected: AtomicBool,
    pub read_task: AsyncMutex<Option<JoinHandle<()>>>,
    pub keepalive_task: AsyncMutex<Option<JoinHandle<()>>>,
    pub operation_timeout: Duration,
}

impl Inner {
    pub fn new(
        version: MqttVersion,
        writer: OwnedWriteHalf,
        listener: Option<Arc<dyn MqttMessageListener>>,
        operation_timeout: Duration,
    ) -> Self {
        Inner {
            version,
            writer: AsyncMutex::new(Some(writer)),
            next_packet_id: AtomicU16::new(1),
            pending: std::sync::Mutex::new(HashMap::new()),
            incoming_qos2: std::sync::Mutex::new(PendingQos2Incoming {
                messages: HashMap::new(),
            }),
            listener: std::sync::Mutex::new(listener),
            connected: AtomicBool::new(true),
            read_task: AsyncMutex::new(None),
            keepalive_task: AsyncMutex::new(None),
            operation_timeout,
        }
    }

    pub fn alloc_packet_id(&self) -> u16 {
        loop {
            let id = self.next_packet_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }

    pub fn take_listener(&self) -> Option<Arc<dyn MqttMessageListener>> {
        self.listener.lock().unwrap().clone()
    }
}

/// Register a packet id as awaiting a broker ack; the returned receiver
/// resolves once [`crate::client::io::handle_incoming`] sees the matching
/// PUBACK/PUBREC/PUBCOMP/SUBACK/UNSUBACK and calls [`resolve_pending`].
pub(super) fn register_pending(inner: &Arc<Inner>, id: u16) -> oneshot::Receiver<Packet> {
    let (tx, rx) = oneshot::channel();
    inner.pending.lock().unwrap().insert(id, tx);
    rx
}

/// Resolve a previously-[`register_pending`]ed packet id with the ack
/// packet that just arrived. A no-op if nothing is waiting on `id`
/// (e.g. a duplicate ack after we already timed out).
pub(super) fn resolve_pending(inner: &Arc<Inner>, id: u16, packet: Packet) {
    if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(packet);
    }
}

/// Await a [`register_pending`] receiver, applying the connection's
/// configured operation timeout.
pub(super) async fn wait_for(
    inner: &Arc<Inner>,
    rx: oneshot::Receiver<Packet>,
) -> MqttResult<Packet> {
    match tokio::time::timeout(inner.operation_timeout, rx).await {
        Ok(Ok(packet)) => Ok(packet),
        Ok(Err(_)) => Err(MqttError::Session(
            "connection closed while waiting for ack".into(),
        )),
        Err(_) => Err(MqttError::Timeout),
    }
}

pub(super) fn unexpected(packet: &Packet) -> MqttError {
    MqttError::MalformedPacket(format!("unexpected packet while awaiting ack: {packet:?}"))
}

/// Hand a delivered message to the registered listener, if any.
pub(super) fn deliver(inner: &Arc<Inner>, message: MqttMessage) {
    if let Some(listener) = inner.take_listener() {
        listener.on_message(message);
    }
}

/// Mark the connection dead and notify the listener, if any. Idempotent:
/// safe to call from the read loop, the keep-alive loop, or both.
pub(super) fn finish_disconnected(inner: &Arc<Inner>, reason: String) {
    inner.connected.store(false, Ordering::Relaxed);
    if let Some(listener) = inner.take_listener() {
        listener.on_disconnected(reason);
    }
}
