//! Async, `tokio`-based MQTT client built on top of [`crate::protocol`].
//!
//! Exposed to foreign languages through UniFFI: [`MqttClient`] is a UniFFI
//! *Object* (reference-counted handle), its configuration/result types are
//! UniFFI *Records* (plain data classes), and incoming messages are
//! delivered through the [`MqttMessageListener`] *callback interface* so
//! Kotlin/Swift/Python/etc. code can subscribe without polling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

use crate::error::{MqttError, MqttResult};
use crate::protocol::{
    ack::SimpleAck,
    connect::{ConnectPacket, Will},
    packet::{DisconnectPacket, Packet},
    properties::Properties,
    publish::PublishPacket,
    subscribe::{SubAckReasonCode, SubscribeFilter, SubscribePacket, UnsubscribePacket},
    MqttVersion, QoS,
};

/// Default time to wait for a broker acknowledgement (PUBACK/PUBREC/
/// PUBCOMP/SUBACK/UNSUBACK) before returning [`MqttError::Timeout`].
const DEFAULT_OPERATION_TIMEOUT_SECS: u32 = 15;

/// Last-Will-and-Testament configuration for a [`ConnectOptions`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WillOptions {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// Everything needed to establish an MQTT connection.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    /// Which protocol version to speak. The broker never up/downgrades a
    /// connection once CONNECT is sent.
    pub version: MqttVersion,
    /// MQTT 3.1.1 "Clean Session" / MQTT 5.0 "Clean Start".
    pub clean_start: bool,
    /// Keep-alive interval in seconds. `0` disables keep-alive pings.
    pub keep_alive_secs: u16,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub will: Option<WillOptions>,
    /// Seconds to wait for the TCP connection + CONNACK before failing.
    pub connect_timeout_secs: u32,
    /// Seconds to wait for a QoS 1/2 handshake step before failing. `0`
    /// selects the built-in default (15s).
    pub operation_timeout_secs: u32,
}

impl ConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        client_id: impl Into<String>,
        version: MqttVersion,
    ) -> Self {
        ConnectOptions {
            host: host.into(),
            port,
            client_id: client_id.into(),
            version,
            clean_start: true,
            keep_alive_secs: 30,
            username: None,
            password: None,
            will: None,
            connect_timeout_secs: 10,
            operation_timeout_secs: 0,
        }
    }

    fn operation_timeout(&self) -> Duration {
        let secs = if self.operation_timeout_secs == 0 {
            DEFAULT_OPERATION_TIMEOUT_SECS
        } else {
            self.operation_timeout_secs
        };
        Duration::from_secs(secs as u64)
    }
}

/// Outcome of a successful [`MqttClient::connect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ConnectResult {
    pub session_present: bool,
    /// `0x00` on success. MQTT 5.0 reason code (MQTT 3.1.1 codes are mapped
    /// onto the equivalent 5.0 code by
    /// [`ConnectReasonCode`](crate::protocol::connect::ConnectReasonCode)).
    pub reason_code: u8,
}

/// A message delivered to a subscriber.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// Result of a subscribe request: the reason/return code the broker
/// granted for the requested filter (`< 0x80` is success; MQTT 3.1.1 codes
/// 0/1/2 map onto the granted QoS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SubscribeResult {
    pub reason_code: u8,
}

/// Callback interface implemented by foreign code to receive events from an
/// [`MqttClient`] without polling.
#[uniffi::export(with_foreign)]
pub trait MqttMessageListener: Send + Sync {
    /// Called for every PUBLISH delivered to a subscription this client holds.
    fn on_message(&self, message: MqttMessage);
    /// Called when the connection to the broker is lost or closed, whether
    /// by the client, the broker, or the network.
    fn on_disconnected(&self, reason: String);
}

struct PendingQos2Incoming {
    messages: HashMap<u16, MqttMessage>,
}

struct Inner {
    version: MqttVersion,
    writer: AsyncMutex<Option<OwnedWriteHalf>>,
    next_packet_id: AtomicU16,
    pending: std::sync::Mutex<HashMap<u16, oneshot::Sender<Packet>>>,
    incoming_qos2: std::sync::Mutex<PendingQos2Incoming>,
    listener: std::sync::Mutex<Option<Arc<dyn MqttMessageListener>>>,
    connected: AtomicBool,
    read_task: AsyncMutex<Option<JoinHandle<()>>>,
    keepalive_task: AsyncMutex<Option<JoinHandle<()>>>,
    operation_timeout: Duration,
}

impl Inner {
    fn alloc_packet_id(&self) -> u16 {
        loop {
            let id = self.next_packet_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }

    fn take_listener(&self) -> Option<Arc<dyn MqttMessageListener>> {
        self.listener.lock().unwrap().clone()
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
    options: ConnectOptions,
    inner: AsyncMutex<Option<Arc<Inner>>>,
    /// Listener registered before `connect()` completes; moved into the
    /// live `Inner` as soon as the connection is established.
    pending_listener: std::sync::Mutex<Option<Arc<dyn MqttMessageListener>>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MqttClient {
    /// Create a new client from `options`. Does not connect yet.
    #[uniffi::constructor]
    pub fn new(options: ConnectOptions) -> Self {
        MqttClient {
            options,
            inner: AsyncMutex::new(None),
            pending_listener: std::sync::Mutex::new(None),
        }
    }

    /// Register (or replace) the listener that receives incoming messages
    /// and disconnect notifications. Safe to call before or after connect.
    pub fn set_message_listener(&self, listener: Arc<dyn MqttMessageListener>) {
        // Listener is stashed on `options`-adjacent state lazily created at
        // connect time; if we're already connected, push it straight in.
        if let Ok(guard) = self.inner.try_lock() {
            if let Some(inner) = guard.as_ref() {
                *inner.listener.lock().unwrap() = Some(listener);
                return;
            }
        }
        self.pending_listener.lock().unwrap().replace(listener);
    }

    /// Open the TCP connection and complete the CONNECT/CONNACK handshake.
    pub async fn connect(&self) -> MqttResult<ConnectResult> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Err(MqttError::AlreadyConnected);
        }

        let addr = format!("{}:{}", self.options.host, self.options.port);
        let connect_fut = TcpStream::connect(&addr);
        let stream = tokio::time::timeout(
            Duration::from_secs(self.options.connect_timeout_secs.max(1) as u64),
            connect_fut,
        )
        .await
        .map_err(|_| MqttError::Timeout)??;
        stream.set_nodelay(true).ok();

        let (mut reader, mut writer) = stream.into_split();

        let connect_packet = build_connect_packet(&self.options);
        let encoded = Packet::Connect(connect_packet)
            .encode(self.options.version)
            .map_err(|e| MqttError::Protocol(e.to_string()))?;
        writer.write_all(&encoded).await?;

        let mut buf = BytesMut::with_capacity(1024);
        let connack = read_one_packet(
            &mut reader,
            &mut buf,
            self.options.version,
            self.options.connect_timeout_secs,
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

        let listener = self.pending_listener.lock().unwrap().take();
        let inner = Arc::new(Inner {
            version: self.options.version,
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
            operation_timeout: self.options.operation_timeout(),
        });

        let read_handle = tokio::spawn(read_loop(inner.clone(), reader, buf));
        *inner.read_task.lock().await = Some(read_handle);

        if self.options.keep_alive_secs > 0 {
            let ka_handle =
                tokio::spawn(keepalive_loop(inner.clone(), self.options.keep_alive_secs));
            *inner.keepalive_task.lock().await = Some(ka_handle);
        }

        *guard = Some(inner);

        Ok(ConnectResult {
            session_present: connack.session_present,
            reason_code: connack.reason_code.0,
        })
    }

    /// `true` once [`connect`](Self::connect) has succeeded and the
    /// connection has not since been lost or closed.
    pub fn is_connected(&self) -> bool {
        match self.inner.try_lock() {
            Ok(guard) => guard
                .as_ref()
                .map(|i| i.connected.load(Ordering::Relaxed))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Publish `payload` to `topic`. Resolves once the broker has
    /// acknowledged the message (QoS 1: PUBACK; QoS 2: full four-part
    /// handshake). Resolves immediately for QoS 0.
    pub async fn publish(
        &self,
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
    ) -> MqttResult<()> {
        let inner = self.connected_inner().await?;
        let payload = Bytes::from(payload);

        match qos {
            QoS::AtMostOnce => {
                let pkt = Packet::Publish(PublishPacket {
                    dup: false,
                    qos,
                    retain,
                    topic,
                    packet_id: None,
                    payload,
                    properties: Properties::new(),
                });
                send_packet(&inner, &pkt).await
            }
            QoS::AtLeastOnce => {
                let id = inner.alloc_packet_id();
                let rx = register_pending(&inner, id);
                let pkt = Packet::Publish(PublishPacket {
                    dup: false,
                    qos,
                    retain,
                    topic,
                    packet_id: Some(id),
                    payload,
                    properties: Properties::new(),
                });
                send_packet(&inner, &pkt).await?;
                match wait_for(&inner, rx).await? {
                    Packet::PubAck(_) => Ok(()),
                    other => Err(unexpected(&other)),
                }
            }
            QoS::ExactlyOnce => {
                let id = inner.alloc_packet_id();
                let rx = register_pending(&inner, id);
                let pkt = Packet::Publish(PublishPacket {
                    dup: false,
                    qos,
                    retain,
                    topic,
                    packet_id: Some(id),
                    payload,
                    properties: Properties::new(),
                });
                send_packet(&inner, &pkt).await?;
                match wait_for(&inner, rx).await? {
                    Packet::PubRec(_) => {}
                    other => return Err(unexpected(&other)),
                }

                let rx2 = register_pending(&inner, id);
                let rel = Packet::PubRel(SimpleAck::success(id));
                send_packet(&inner, &rel).await?;
                match wait_for(&inner, rx2).await? {
                    Packet::PubComp(_) => Ok(()),
                    other => Err(unexpected(&other)),
                }
            }
        }
    }

    /// Subscribe to `topic_filter` requesting at most `qos`. Returns the
    /// reason/return code the broker granted.
    pub async fn subscribe(&self, topic_filter: String, qos: QoS) -> MqttResult<SubscribeResult> {
        let inner = self.connected_inner().await?;
        let id = inner.alloc_packet_id();
        let rx = register_pending(&inner, id);
        let pkt = Packet::Subscribe(SubscribePacket {
            packet_id: id,
            filters: vec![SubscribeFilter::new(topic_filter, qos)],
            properties: Properties::new(),
        });
        send_packet(&inner, &pkt).await?;
        match wait_for(&inner, rx).await? {
            Packet::SubAck(ack) => {
                let code = ack
                    .reason_codes
                    .first()
                    .copied()
                    .unwrap_or(SubAckReasonCode::FAILURE);
                Ok(SubscribeResult {
                    reason_code: code.0,
                })
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Unsubscribe from `topic_filter`.
    pub async fn unsubscribe(&self, topic_filter: String) -> MqttResult<()> {
        let inner = self.connected_inner().await?;
        let id = inner.alloc_packet_id();
        let rx = register_pending(&inner, id);
        let pkt = Packet::Unsubscribe(UnsubscribePacket {
            packet_id: id,
            topic_filters: vec![topic_filter],
            properties: Properties::new(),
        });
        send_packet(&inner, &pkt).await?;
        match wait_for(&inner, rx).await? {
            Packet::UnsubAck(_) => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// Gracefully disconnect: sends a DISCONNECT packet, stops background
    /// tasks, and closes the socket. Safe to call multiple times.
    pub async fn disconnect(&self) -> MqttResult<()> {
        let mut guard = self.inner.lock().await;
        let Some(inner) = guard.take() else {
            return Ok(());
        };
        inner.connected.store(false, Ordering::Relaxed);
        let disconnect = Packet::Disconnect(DisconnectPacket::normal());
        let _ = send_packet(&inner, &disconnect).await;
        if let Some(mut w) = inner.writer.lock().await.take() {
            let _ = w.shutdown().await;
        }
        if let Some(h) = inner.read_task.lock().await.take() {
            h.abort();
        }
        if let Some(h) = inner.keepalive_task.lock().await.take() {
            h.abort();
        }
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
impl Drop for MqttClient {
    fn drop(&mut self) {
        let Ok(mut guard) = self.inner.try_lock() else {
            return;
        };
        let Some(inner) = guard.take() else {
            return;
        };
        inner.connected.store(false, Ordering::Relaxed);
        // Dropping the write half closes that side of the socket outright.
        if let Ok(mut writer) = inner.writer.try_lock() {
            writer.take();
        }
        // Aborting the read/keep-alive tasks cancels them at their next
        // await point, which drops their captured `Arc<Inner>` clone and
        // (for the read task) the read half, closing the socket the rest
        // of the way.
        if let Ok(mut task) = inner.read_task.try_lock() {
            if let Some(handle) = task.take() {
                handle.abort();
            }
        };
        if let Ok(mut task) = inner.keepalive_task.try_lock() {
            if let Some(handle) = task.take() {
                handle.abort();
            }
        };
    }
}

fn build_connect_packet(options: &ConnectOptions) -> ConnectPacket {
    let will = options.will.as_ref().map(|w| Will {
        topic: w.topic.clone(),
        payload: Bytes::from(w.payload.clone()),
        qos: w.qos,
        retain: w.retain,
        properties: Properties::new(),
        delay_interval: 0,
    });
    ConnectPacket {
        version: options.version,
        client_id: options.client_id.clone(),
        clean_start: options.clean_start,
        keep_alive: options.keep_alive_secs,
        username: options.username.clone(),
        password: options.password.clone().map(Bytes::from),
        will,
        properties: Properties::new(),
    }
}

impl MqttClient {
    async fn connected_inner(&self) -> MqttResult<Arc<Inner>> {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(inner) if inner.connected.load(Ordering::Relaxed) => Ok(inner.clone()),
            _ => Err(MqttError::NotConnected),
        }
    }
}

fn register_pending(inner: &Arc<Inner>, id: u16) -> oneshot::Receiver<Packet> {
    let (tx, rx) = oneshot::channel();
    inner.pending.lock().unwrap().insert(id, tx);
    rx
}

async fn wait_for(inner: &Arc<Inner>, rx: oneshot::Receiver<Packet>) -> MqttResult<Packet> {
    match tokio::time::timeout(inner.operation_timeout, rx).await {
        Ok(Ok(packet)) => Ok(packet),
        Ok(Err(_)) => Err(MqttError::Session(
            "connection closed while waiting for ack".into(),
        )),
        Err(_) => Err(MqttError::Timeout),
    }
}

fn unexpected(packet: &Packet) -> MqttError {
    MqttError::MalformedPacket(format!("unexpected packet while awaiting ack: {packet:?}"))
}

async fn send_packet(inner: &Arc<Inner>, packet: &Packet) -> MqttResult<()> {
    let encoded = packet
        .encode(inner.version)
        .map_err(|e| MqttError::Protocol(e.to_string()))?;
    let mut guard = inner.writer.lock().await;
    match guard.as_mut() {
        Some(writer) => {
            writer.write_all(&encoded).await?;
            Ok(())
        }
        None => Err(MqttError::NotConnected),
    }
}

async fn read_one_packet(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    buf: &mut BytesMut,
    version: MqttVersion,
    timeout_secs: u32,
) -> MqttResult<Packet> {
    let deadline = Duration::from_secs(timeout_secs.max(1) as u64);
    tokio::time::timeout(deadline, async {
        loop {
            if let Some(packet) = Packet::decode(buf, version)? {
                return Ok(packet);
            }
            let n = reader.read_buf(buf).await?;
            if n == 0 {
                return Err(MqttError::Io("connection closed".into()));
            }
        }
    })
    .await
    .map_err(|_| MqttError::Timeout)?
}

async fn read_loop(
    inner: Arc<Inner>,
    mut reader: tokio::net::tcp::OwnedReadHalf,
    mut buf: BytesMut,
) {
    loop {
        let packet = match Packet::decode(&mut buf, inner.version) {
            Ok(Some(packet)) => packet,
            Ok(None) => {
                let mut read_buf = [0u8; 4096];
                match reader.read(&mut read_buf).await {
                    Ok(0) => {
                        finish_disconnected(&inner, "connection closed by peer".into());
                        return;
                    }
                    Ok(n) => {
                        buf.extend_from_slice(&read_buf[..n]);
                        continue;
                    }
                    Err(e) => {
                        finish_disconnected(&inner, format!("read error: {e}"));
                        return;
                    }
                }
            }
            Err(e) => {
                finish_disconnected(&inner, format!("protocol error: {e}"));
                return;
            }
        };

        if !handle_incoming(&inner, packet).await {
            return;
        }
    }
}

/// Returns `false` if the connection should be considered terminated.
async fn handle_incoming(inner: &Arc<Inner>, packet: Packet) -> bool {
    match packet {
        Packet::Publish(p) => {
            let message = MqttMessage {
                topic: p.topic.clone(),
                payload: p.payload.to_vec(),
                qos: p.qos,
                retain: p.retain,
            };
            match p.qos {
                QoS::AtMostOnce => deliver(inner, message),
                QoS::AtLeastOnce => {
                    if let Some(id) = p.packet_id {
                        deliver(inner, message);
                        let _ = send_packet(inner, &Packet::PubAck(SimpleAck::success(id))).await;
                    }
                }
                QoS::ExactlyOnce => {
                    if let Some(id) = p.packet_id {
                        inner
                            .incoming_qos2
                            .lock()
                            .unwrap()
                            .messages
                            .insert(id, message);
                        let _ = send_packet(inner, &Packet::PubRec(SimpleAck::success(id))).await;
                    }
                }
            }
            true
        }
        Packet::PubRel(ack) => {
            let message = inner
                .incoming_qos2
                .lock()
                .unwrap()
                .messages
                .remove(&ack.packet_id);
            if let Some(message) = message {
                deliver(inner, message);
            }
            let _ = send_packet(inner, &Packet::PubComp(SimpleAck::success(ack.packet_id))).await;
            true
        }
        Packet::PubAck(ack) => {
            resolve_pending(inner, ack.packet_id, Packet::PubAck(ack));
            true
        }
        Packet::PubRec(ack) => {
            resolve_pending(inner, ack.packet_id, Packet::PubRec(ack));
            true
        }
        Packet::PubComp(ack) => {
            resolve_pending(inner, ack.packet_id, Packet::PubComp(ack));
            true
        }
        Packet::SubAck(ack) => {
            resolve_pending(inner, ack.packet_id, Packet::SubAck(ack));
            true
        }
        Packet::UnsubAck(ack) => {
            resolve_pending(inner, ack.packet_id, Packet::UnsubAck(ack));
            true
        }
        Packet::PingResp => true,
        Packet::Disconnect(d) => {
            finish_disconnected(
                inner,
                format!("server disconnected (reason 0x{:02X})", d.reason_code),
            );
            false
        }
        _ => true,
    }
}

fn deliver(inner: &Arc<Inner>, message: MqttMessage) {
    if let Some(listener) = inner.take_listener() {
        listener.on_message(message);
    }
}

fn resolve_pending(inner: &Arc<Inner>, id: u16, packet: Packet) {
    if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(packet);
    }
}

fn finish_disconnected(inner: &Arc<Inner>, reason: String) {
    inner.connected.store(false, Ordering::Relaxed);
    if let Some(listener) = inner.take_listener() {
        listener.on_disconnected(reason);
    }
}

async fn keepalive_loop(inner: Arc<Inner>, keep_alive_secs: u16) {
    let interval = Duration::from_secs((keep_alive_secs as u64).max(1)).mul_f32(0.8);
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if !inner.connected.load(Ordering::Relaxed) {
            return;
        }
        if send_packet(&inner, &Packet::PingReq).await.is_err() {
            finish_disconnected(&inner, "keep-alive ping failed".into());
            return;
        }
    }
}
