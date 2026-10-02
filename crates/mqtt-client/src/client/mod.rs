//! Async, `tokio`-based MQTT client built on top of [`crate::protocol`].
//!
//! Exposed to foreign languages through UniFFI: [`MqttClient`] is a UniFFI
//! *Object* (reference-counted handle), its configuration/result types
//! (see `types`, re-exported below) are UniFFI *Records* (plain data classes), and incoming
//! messages are delivered through the [`MqttMessageListener`] *callback
//! interface* so Kotlin/Swift/Python/etc. code can subscribe without
//! polling.
//!
//! Internally split by responsibility:
//! - `types` (public re-exports below) — public data types, no logic.
//! - `inner` (private) — live-connection state (`Inner`) and the
//!   pending-ack/QoS-2-dedup bookkeeping built directly on it.
//! - `io` (private) — wire I/O: the background read loop, keep-alive
//!   loop, and packet encode/write helpers.
//!
//! This module itself holds only the public [`MqttClient`] API surface —
//! `new`/`connect`/`publish`/`subscribe`/`unsubscribe`/`disconnect` — which
//! reads as the client's actual protocol/session logic without I/O
//! plumbing or type definitions in the way.

mod inner;
mod io;
mod tls;
mod types;

pub use types::{
    ConnectOptions, ConnectResult, MqttMessage, MqttMessageListener, SubscribeResult, TlsOptions,
    WillOptions,
};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{MqttError, MqttResult};
use crate::protocol::{
    ack::SimpleAck,
    packet::{DisconnectPacket, Packet},
    properties::Properties,
    publish::PublishPacket,
    subscribe::{SubAckReasonCode, SubscribeFilter, SubscribePacket, UnsubscribePacket},
    topic::{is_valid_filter, is_valid_topic_name},
    QoS,
};
use crate::support::LockExt;

use inner::{unexpected, Inner, ListenerSlot};
use io::{
    build_connect_packet, exchange, exchange_with_retry, keepalive_loop, read_loop,
    read_one_packet, send_packet,
};

/// How many times [`MqttClient::publish`] resends an unacked QoS 1/2
/// PUBLISH (with DUP=1) — or, for QoS 2, an unacked PUBREL — before giving
/// up and returning [`MqttError::Timeout`]. The broker doesn't resend on
/// our behalf; MQTT's "at least once"/"exactly once" delivery guarantees
/// are the *publisher's* responsibility to enforce via redelivery.
const MAX_PUBLISH_RETRIES: u32 = 3;

/// How often the reconnect supervisor (see [`spawn_reconnect_supervisor`])
/// polls for a connection loss while otherwise idle.
const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// MQTT 5.0 reason codes at or above this value are failures (§2.4).
const REASON_FAILURE_THRESHOLD: u8 = 0x80;

/// State shared between [`MqttClient`] and its background reconnect
/// supervisor task (when [`ConnectOptions::auto_reconnect`] is set). Split
/// out from `MqttClient` itself so the supervisor can hold a [`Weak`]
/// reference to it — a *strong* `Arc` would keep the whole client (and its
/// socket) alive forever even after the last real handle to it is dropped.
struct ClientShared {
    options: ConnectOptions,
    inner: AsyncMutex<Option<Arc<Inner>>>,
    /// The registered message listener. One slot shared with every `Inner`
    /// this client ever creates, so registering it never races with
    /// `connect()` and it survives reconnects.
    listener: ListenerSlot,
    /// Topic filters this client is currently subscribed to, tracked
    /// client-side (the broker doesn't tell us this) so the reconnect
    /// supervisor can replay them after re-establishing the connection.
    /// QoS is what was originally requested, not necessarily what was
    /// granted.
    subscriptions: Mutex<HashMap<String, QoS>>,
    /// Set by `disconnect()` so the reconnect supervisor (if running)
    /// knows this loss was deliberate and should stop instead of
    /// reconnecting.
    user_disconnected: AtomicBool,
    /// `true` while a reconnect supervisor task is alive, so repeated
    /// `connect()` calls never stack up duplicate supervisors.
    supervisor_running: AtomicBool,
}

impl ClientShared {
    async fn connected_inner(&self) -> MqttResult<Arc<Inner>> {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(inner) if inner.is_connected() => Ok(inner.clone()),
            _ => Err(MqttError::NotConnected),
        }
    }

    async fn is_live(&self) -> bool {
        self.inner
            .lock()
            .await
            .as_ref()
            .is_some_and(|i| i.is_connected())
    }
}

/// A connected (or not-yet-connected) MQTT client instance.
///
/// One `MqttClient` corresponds to one broker connection / MQTT session.
/// Create it with [`MqttClient::new`], call [`MqttClient::connect`], then
/// use [`MqttClient::publish`] / [`MqttClient::subscribe`] /
/// [`MqttClient::unsubscribe`]. Call [`MqttClient::disconnect`] (or simply
/// drop it) to close the connection.
#[derive(uniffi::Object)]
pub struct MqttClient {
    shared: Arc<ClientShared>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MqttClient {
    /// Create a new client from `options`. Does not connect yet.
    #[uniffi::constructor]
    pub fn new(options: ConnectOptions) -> Self {
        MqttClient {
            shared: Arc::new(ClientShared {
                options,
                inner: AsyncMutex::new(None),
                listener: Arc::new(Mutex::new(None)),
                subscriptions: Mutex::new(HashMap::new()),
                user_disconnected: AtomicBool::new(false),
                supervisor_running: AtomicBool::new(false),
            }),
        }
    }

    /// Register (or replace) the listener that receives incoming messages
    /// and disconnect notifications. Safe to call before or after connect.
    pub fn set_message_listener(&self, listener: Arc<dyn MqttMessageListener>) {
        *self.shared.listener.lock_safe() = Some(listener);
    }

    /// Open the TCP connection and complete the CONNECT/CONNACK handshake.
    /// Safe to call again after a connection loss (including one this
    /// client is still in the middle of auto-reconnecting from).
    ///
    /// If [`ConnectOptions::auto_reconnect`] is set, a background
    /// supervisor is started (once) that automatically re-runs this same
    /// handshake — with exponential backoff — after any *unexpected*
    /// connection loss (i.e. not caused by [`Self::disconnect`]), then
    /// replays every topic this client is currently subscribed to.
    pub async fn connect(&self) -> MqttResult<ConnectResult> {
        self.shared.options.validate()?;
        self.shared
            .user_disconnected
            .store(false, Ordering::Relaxed);
        let result = connect_once(&self.shared).await?;
        if self.shared.options.auto_reconnect {
            spawn_reconnect_supervisor(&self.shared);
        }
        Ok(result)
    }

    /// `true` once [`connect`](Self::connect) has succeeded and the
    /// connection has not since been lost or closed.
    pub fn is_connected(&self) -> bool {
        match self.shared.inner.try_lock() {
            Ok(guard) => guard.as_ref().is_some_and(|i| i.is_connected()),
            Err(_) => false,
        }
    }

    /// Publish `payload` to `topic`. Resolves once the broker has
    /// acknowledged the message (QoS 1: PUBACK; QoS 2: full four-part
    /// handshake). Resolves immediately for QoS 0.
    ///
    /// For QoS 1/2, an unacked step is retried (with DUP=1) up to
    /// `MAX_PUBLISH_RETRIES` (3) times before giving up with
    /// [`MqttError::Timeout`] — the broker never resends on our behalf, so
    /// enforcing "at least once"/"exactly once" delivery is the
    /// publisher's job.
    ///
    /// A topic containing wildcards (`+`, `#`) or that is empty is
    /// rejected locally with [`MqttError::Protocol`] (a broker would drop
    /// the whole connection for it). An MQTT 5.0 broker that answers with a
    /// failure reason code (e.g. "not authorized") surfaces as
    /// [`MqttError::Session`] instead of being reported as success.
    pub async fn publish(
        &self,
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
    ) -> MqttResult<()> {
        if !is_valid_topic_name(&topic) {
            return Err(MqttError::Protocol(format!(
                "invalid topic name {topic:?}: must be non-empty and contain no wildcards"
            )));
        }
        let inner = self.shared.connected_inner().await?;
        let payload = Bytes::from(payload);
        let publish = |packet_id: Option<u16>, dup: bool| {
            Packet::Publish(PublishPacket {
                dup,
                qos,
                retain,
                topic: topic.clone(),
                packet_id,
                payload: payload.clone(),
                properties: Properties::new(),
            })
        };

        match qos {
            QoS::AtMostOnce => send_packet(&inner, &publish(None, false)).await,
            QoS::AtLeastOnce => {
                let id = inner.alloc_packet_id();
                let ack = exchange_with_retry(&inner, id, MAX_PUBLISH_RETRIES, |dup| {
                    publish(Some(id), dup)
                })
                .await?;
                match ack {
                    Packet::PubAck(ack) => check_reason("PUBLISH", ack.reason_code),
                    other => Err(unexpected(&other)),
                }
            }
            QoS::ExactlyOnce => {
                let id = inner.alloc_packet_id();
                // Step 1: PUBLISH -> PUBREC.
                let rec = exchange_with_retry(&inner, id, MAX_PUBLISH_RETRIES, |dup| {
                    publish(Some(id), dup)
                })
                .await?;
                match rec {
                    Packet::PubRec(ack) => check_reason("PUBLISH", ack.reason_code)?,
                    other => return Err(unexpected(&other)),
                }
                // Step 2: PUBREL -> PUBCOMP. PUBREL has no DUP flag
                // (MQTT-5.0 §3.6.1) — it's resent as-is on timeout.
                let comp = exchange_with_retry(&inner, id, MAX_PUBLISH_RETRIES, |_| {
                    Packet::PubRel(SimpleAck::success(id))
                })
                .await?;
                match comp {
                    Packet::PubComp(ack) => check_reason("PUBREL", ack.reason_code),
                    other => Err(unexpected(&other)),
                }
            }
        }
    }

    /// Subscribe to `topic_filter` requesting at most `qos`. Returns the
    /// reason/return code the broker granted.
    ///
    /// Recorded in this client's own subscription bookkeeping so
    /// [`ConnectOptions::auto_reconnect`] can replay it after a reconnect
    /// — the broker has no way to tell a resumed client what it was
    /// subscribed to beyond what a persistent (`clean_start = false`)
    /// session already covers.
    pub async fn subscribe(&self, topic_filter: String, qos: QoS) -> MqttResult<SubscribeResult> {
        if !is_valid_filter(&topic_filter) {
            return Err(MqttError::Protocol(format!(
                "invalid topic filter {topic_filter:?}"
            )));
        }
        let inner = self.shared.connected_inner().await?;
        let response = exchange(&inner, |id| subscribe_packet(id, &topic_filter, qos)).await?;
        match response {
            Packet::SubAck(ack) => {
                let code = ack
                    .reason_codes
                    .first()
                    .copied()
                    .unwrap_or(SubAckReasonCode::FAILURE);
                if code.0 < REASON_FAILURE_THRESHOLD {
                    self.shared
                        .subscriptions
                        .lock_safe()
                        .insert(topic_filter, qos);
                }
                Ok(SubscribeResult {
                    reason_code: code.0,
                })
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Unsubscribe from `topic_filter`.
    pub async fn unsubscribe(&self, topic_filter: String) -> MqttResult<()> {
        if !is_valid_filter(&topic_filter) {
            return Err(MqttError::Protocol(format!(
                "invalid topic filter {topic_filter:?}"
            )));
        }
        let inner = self.shared.connected_inner().await?;
        let response = exchange(&inner, |id| {
            Packet::Unsubscribe(UnsubscribePacket {
                packet_id: id,
                topic_filters: vec![topic_filter.clone()],
                properties: Properties::new(),
            })
        })
        .await?;
        match response {
            Packet::UnsubAck(_) => {
                self.shared.subscriptions.lock_safe().remove(&topic_filter);
                Ok(())
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Gracefully disconnect: sends a DISCONNECT packet, stops background
    /// tasks, and closes the socket. Safe to call multiple times. Also
    /// stops the [`ConnectOptions::auto_reconnect`] supervisor, if one is
    /// running — this is a deliberate disconnect, not a connection loss to
    /// recover from.
    pub async fn disconnect(&self) -> MqttResult<()> {
        self.shared.user_disconnected.store(true, Ordering::Relaxed);
        let Some(inner) = self.shared.inner.lock().await.take() else {
            return Ok(());
        };
        if inner.is_connected() {
            let disconnect = Packet::Disconnect(DisconnectPacket::normal());
            let _ = send_packet(&inner, &disconnect).await;
        }
        inner.shutdown().await;
        Ok(())
    }
}

/// Best-effort, synchronous teardown for when an `MqttClient` is simply
/// dropped rather than explicitly [`disconnect`](MqttClient::disconnect)ed
/// (e.g. to simulate a crashed client, or because a caller forgot to call
/// `disconnect()`). Without this, the background read/keep-alive tasks
/// each hold their own `Arc<Inner>` clone — including the socket halves —
/// so dropping the `MqttClient` alone would leak both the connection and
/// the tasks forever instead of closing the socket. This deliberately does
/// *not* send an MQTT DISCONNECT packet, so the broker sees an ungraceful
/// loss (as it would for a real crash) and publishes this connection's
/// Last Will, if one was configured.
///
/// The reconnect supervisor (if any) only holds a [`Weak`] reference to
/// `ClientShared`, so it doesn't need any explicit handling here — once
/// this is the last strong `Arc<ClientShared>`, dropping it means the
/// supervisor's next `Weak::upgrade()` simply fails and it exits.
impl Drop for MqttClient {
    fn drop(&mut self) {
        let Ok(mut guard) = self.shared.inner.try_lock() else {
            return;
        };
        if let Some(inner) = guard.take() {
            inner.shutdown_now();
        }
    }
}

/// `Ok` unless an MQTT 5.0 ack carries a failure reason code.
fn check_reason(what: &str, reason_code: u8) -> MqttResult<()> {
    if reason_code >= REASON_FAILURE_THRESHOLD {
        Err(MqttError::Session(format!(
            "{what} rejected by broker: reason code 0x{reason_code:02X}"
        )))
    } else {
        Ok(())
    }
}

fn subscribe_packet(packet_id: u16, topic_filter: &str, qos: QoS) -> Packet {
    Packet::Subscribe(SubscribePacket {
        packet_id,
        filters: vec![SubscribeFilter::new(topic_filter.to_string(), qos)],
        properties: Properties::new(),
    })
}

/// The actual connect handshake, shared between [`MqttClient::connect`]
/// and the reconnect supervisor (which calls this again after a loss).
/// Allows reconnecting over a dead-but-still-present `Inner` (not just a
/// completely fresh client) — without this, a client could never be
/// reconnected at all once its first connection dropped, auto-reconnect
/// or not.
async fn connect_once(shared: &Arc<ClientShared>) -> MqttResult<ConnectResult> {
    let mut guard = shared.inner.lock().await;
    if guard.as_ref().is_some_and(|i| i.is_connected()) {
        return Err(MqttError::AlreadyConnected);
    }

    let options = &shared.options;
    let connect_timeout = options.connect_timeout();
    let max_packet_size = options.max_packet_size();
    let stream: Box<dyn tls::Transport> =
        tokio::time::timeout(connect_timeout, tls::connect_transport(options))
            .await
            .map_err(|_| MqttError::Timeout)??;

    let (mut reader, mut writer) = tokio::io::split(stream);

    let encoded = Packet::Connect(build_connect_packet(options))
        .encode(options.version)
        .map_err(|e| MqttError::Protocol(e.to_string()))?;
    writer.write_all(&encoded).await?;

    let mut buf = BytesMut::with_capacity(1024);
    let connack = read_one_packet(
        &mut reader,
        &mut buf,
        options.version,
        connect_timeout,
        max_packet_size,
    )
    .await?;
    let connack = match connack {
        Packet::ConnAck(ack) => ack,
        other => {
            return Err(MqttError::MalformedPacket(format!(
                "expected CONNACK, got {other:?}"
            )))
        }
    };
    if !connack.reason_code.is_success() {
        return Err(MqttError::ConnectionRefused(format!(
            "reason code 0x{:02X}",
            connack.reason_code.0
        )));
    }

    // Release whatever the previous (dead) connection still holds — its
    // read task keeps the old socket half alive otherwise.
    if let Some(old) = guard.take() {
        old.shutdown().await;
    }

    let inner = Arc::new(Inner::new(
        options.version,
        writer,
        shared.listener.clone(),
        options.operation_timeout(),
    ));

    let read_handle = tokio::spawn(read_loop(inner.clone(), reader, buf, max_packet_size));
    *inner.read_task.lock().await = Some(read_handle);

    if options.keep_alive_secs > 0 {
        let ka_handle = tokio::spawn(keepalive_loop(inner.clone(), options.keep_alive_secs));
        *inner.keepalive_task.lock().await = Some(ka_handle);
    }

    *guard = Some(inner);

    Ok(ConnectResult {
        session_present: connack.session_present,
        reason_code: connack.reason_code.0,
    })
}

/// Re-sends SUBSCRIBE for every topic filter this client's own
/// bookkeeping says it was subscribed to, best-effort (an individual
/// re-subscribe failing doesn't abort the rest, and isn't itself
/// reported anywhere beyond a `tracing` line — there's no additional
/// foreign-visible event for "reconnected", only the same
/// `on_disconnected` that already fired for the original loss).
async fn replay_subscriptions(shared: &Arc<ClientShared>) {
    let subs: Vec<(String, QoS)> = shared
        .subscriptions
        .lock_safe()
        .iter()
        .map(|(topic, qos)| (topic.clone(), *qos))
        .collect();
    let Ok(inner) = shared.connected_inner().await else {
        return;
    };
    for (topic_filter, qos) in subs {
        match exchange(&inner, |id| subscribe_packet(id, &topic_filter, qos)).await {
            Ok(_) => {}
            // The connection itself is gone again; the supervisor will
            // reconnect and replay once more, so don't keep hammering it.
            Err(MqttError::NotConnected | MqttError::Io(_) | MqttError::Session(_)) => {
                tracing::warn!(topic = %topic_filter, "re-subscribe failed while reconnecting");
                return;
            }
            Err(e) => {
                tracing::warn!(topic = %topic_filter, error = %e, "re-subscribe was not acked while reconnecting");
            }
        }
    }
}

/// Spawned (at most one at a time) from [`MqttClient::connect`] when
/// [`ConnectOptions::auto_reconnect`] is set. Holds only a [`Weak`]
/// reference to [`ClientShared`] so it never keeps a dropped `MqttClient`
/// (and its socket) alive — see the [`Drop`] impl's doc comment.
fn spawn_reconnect_supervisor(shared: &Arc<ClientShared>) {
    if shared.supervisor_running.swap(true, Ordering::AcqRel) {
        return; // one is already watching this client
    }
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        loop {
            reconnect_supervisor(&weak).await;
            let Some(strong) = weak.upgrade() else {
                return;
            };
            strong.supervisor_running.store(false, Ordering::Release);
            // Close the exit race: if the user called `connect()` again
            // between our last `user_disconnected` check and the flag
            // being cleared above, that call saw `supervisor_running`
            // still set and spawned nothing — so take over here.
            if strong.user_disconnected.load(Ordering::Relaxed)
                || strong.supervisor_running.swap(true, Ordering::AcqRel)
            {
                return;
            }
        }
    });
}

async fn reconnect_supervisor(shared: &Weak<ClientShared>) {
    loop {
        // Wait for the current connection (if any) to die.
        loop {
            let Some(strong) = shared.upgrade() else {
                return;
            };
            if strong.user_disconnected.load(Ordering::Relaxed) {
                return;
            }
            let live = strong.is_live().await;
            drop(strong);
            if !live {
                break;
            }
            tokio::time::sleep(RECONNECT_POLL_INTERVAL).await;
        }

        let Some(strong) = shared.upgrade() else {
            return;
        };
        let mut backoff = strong.options.initial_reconnect_backoff();
        let max_backoff = strong.options.max_reconnect_backoff();
        drop(strong);

        loop {
            tokio::time::sleep(backoff).await;
            let Some(strong) = shared.upgrade() else {
                return;
            };
            if strong.user_disconnected.load(Ordering::Relaxed) {
                return;
            }
            match connect_once(&strong).await {
                Ok(_) => {
                    replay_subscriptions(&strong).await;
                    break;
                }
                // Someone (the user, via `connect()`) beat us to it.
                Err(MqttError::AlreadyConnected) => break,
                Err(e) => {
                    tracing::debug!(error = %e, ?backoff, "reconnect attempt failed");
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }
}
