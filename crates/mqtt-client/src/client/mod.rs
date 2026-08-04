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
mod types;

pub use types::{
    ConnectOptions, ConnectResult, MqttMessage, MqttMessageListener, SubscribeResult, WillOptions,
};

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{MqttError, MqttResult};
use crate::protocol::{
    ack::SimpleAck,
    packet::{DisconnectPacket, Packet},
    properties::Properties,
    publish::PublishPacket,
    subscribe::{SubAckReasonCode, SubscribeFilter, SubscribePacket, UnsubscribePacket},
    QoS,
};

use inner::{register_pending, unexpected, wait_for, Inner};
use io::{build_connect_packet, keepalive_loop, read_loop, read_one_packet, send_packet};

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
        let inner = Arc::new(Inner::new(
            self.options.version,
            writer,
            listener,
            self.options.operation_timeout(),
        ));

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

impl MqttClient {
    async fn connected_inner(&self) -> MqttResult<Arc<Inner>> {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(inner) if inner.connected.load(Ordering::Relaxed) => Ok(inner.clone()),
            _ => Err(MqttError::NotConnected),
        }
    }
}
