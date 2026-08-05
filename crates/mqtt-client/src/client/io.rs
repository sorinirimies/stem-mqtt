//! Wire I/O for a live connection: encoding/writing packets, the
//! background read loop that decodes incoming packets and dispatches
//! them, and the keep-alive ping loop. Everything here operates on
//! [`super::inner::Inner`] through its public helpers — no public API
//! surface (that's [`super`]) or type definitions (that's
//! [`super::types`]) live in this module.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf};

use crate::error::MqttResult;
use crate::protocol::ack::SimpleAck;
use crate::protocol::connect::{ConnectPacket, Will};
use crate::protocol::packet::Packet;
use crate::protocol::properties::Properties;
use crate::protocol::MqttVersion;
use crate::MqttError;

use super::inner::{deliver, finish_disconnected, resolve_pending, Inner};
use super::tls::Transport;
use super::types::{ConnectOptions, MqttMessage};
use crate::protocol::QoS;

/// Build the CONNECT packet body for a [`ConnectOptions`].
pub(super) fn build_connect_packet(options: &ConnectOptions) -> ConnectPacket {
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

/// Encode `packet` and write it to the connection's write half.
pub(super) async fn send_packet(inner: &Arc<Inner>, packet: &Packet) -> MqttResult<()> {
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

/// Read exactly one packet during the CONNECT/CONNACK handshake, before
/// the background read loop exists.
pub(super) async fn read_one_packet(
    reader: &mut ReadHalf<Box<dyn Transport>>,
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

/// Background task: decode packets off the socket for the lifetime of the
/// connection and dispatch each to [`handle_incoming`]. Exits (dropping
/// its `Arc<Inner>` clone and the read half, closing the socket) when the
/// peer closes the connection, a read error occurs, or a protocol error is
/// decoded.
pub(super) async fn read_loop(
    inner: Arc<Inner>,
    mut reader: ReadHalf<Box<dyn Transport>>,
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

/// Dispatch one packet decoded from the broker: deliver PUBLISHes (driving
/// the QoS 1/2 ack handshakes), resolve pending client-initiated
/// operations (PUBACK/PUBREC/PUBCOMP/SUBACK/UNSUBACK), and answer
/// PINGRESP/DISCONNECT. Returns `false` if the connection should be
/// considered terminated.
pub(super) async fn handle_incoming(inner: &Arc<Inner>, packet: Packet) -> bool {
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

/// Background task: send a PINGREQ every `0.8 * keep_alive_secs`, per
/// MQTT-3.1.2-23's "should send well before" guidance. Exits (without
/// notifying the listener, since [`read_loop`] will independently detect
/// and report the same dead connection) once the connection is marked
/// disconnected, or immediately if a ping write fails.
pub(super) async fn keepalive_loop(inner: Arc<Inner>, keep_alive_secs: u16) {
    let interval = Duration::from_secs((keep_alive_secs as u64).max(1)).mul_f32(0.8);
    let mut ticker = tokio::time::interval(interval);
    // `tokio::time::interval`'s first `.tick()` resolves immediately, not
    // after `interval` — without this, every connection would send a
    // spurious PINGREQ right after CONNECT instead of waiting a full
    // keep-alive interval like every subsequent ping does.
    ticker.tick().await;
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
