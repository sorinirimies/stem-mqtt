//! Wire I/O for a live connection: encoding/writing packets, the
//! background read loop that decodes incoming packets and dispatches
//! them, and the keep-alive ping loop. Everything here operates on
//! [`super::inner::Inner`] through its public helpers — no public API
//! surface (that's [`super`]) or type definitions (that's
//! [`super::types`]) live in this module.

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

use super::inner::{
    deliver, finish_disconnected, forget_pending, register_pending, resolve_pending, wait_for,
    Inner,
};
use super::tls::Transport;
use super::types::{ConnectOptions, MqttMessage};
use crate::protocol::QoS;
use crate::support::LockExt;

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
///
/// A write that fails or stalls past the operation timeout leaves the byte
/// stream in an unknown (possibly half-written) state, so the connection is
/// declared dead rather than risking a corrupted packet stream — the same
/// path a read error takes, so listeners and the reconnect supervisor see
/// one consistent "connection lost" event.
pub(super) async fn send_packet(inner: &Arc<Inner>, packet: &Packet) -> MqttResult<()> {
    let encoded = packet
        .encode(inner.version)
        .map_err(|e| MqttError::Protocol(e.to_string()))?;
    let mut guard = inner.writer.lock().await;
    let Some(writer) = guard.as_mut() else {
        return Err(MqttError::NotConnected);
    };
    match tokio::time::timeout(inner.operation_timeout, writer.write_all(&encoded)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            drop(guard);
            finish_disconnected(inner, format!("write error: {e}"));
            Err(e.into())
        }
        Err(_) => {
            drop(guard);
            finish_disconnected(inner, "write timed out".into());
            Err(MqttError::Timeout)
        }
    }
}

/// Send the packet built by `build(packet_id)` under a freshly allocated
/// packet id and wait (with the connection's operation timeout) for the
/// single acknowledgement that answers it. Used for SUBSCRIBE/UNSUBSCRIBE,
/// which — unlike QoS 1/2 PUBLISH — are never retransmitted.
pub(super) async fn exchange(
    inner: &Arc<Inner>,
    build: impl FnOnce(u16) -> Packet,
) -> MqttResult<Packet> {
    let id = inner.alloc_packet_id();
    let rx = register_pending(inner, id);
    if let Err(e) = send_packet(inner, &build(id)).await {
        forget_pending(inner, id);
        return Err(e);
    }
    wait_for(inner, id, rx).await
}

/// Like [`exchange`] for a fixed, caller-chosen `id`, but retransmitting
/// when no ack arrives in time: `build(dup)` is re-sent (with `dup = true`
/// from the second attempt on) up to `max_retries` extra times before
/// giving up with [`MqttError::Timeout`]. This is the publisher-side
/// redelivery MQTT's QoS 1/2 guarantees depend on — the broker never
/// resends on our behalf.
pub(super) async fn exchange_with_retry(
    inner: &Arc<Inner>,
    id: u16,
    max_retries: u32,
    build: impl Fn(bool) -> Packet,
) -> MqttResult<Packet> {
    let mut dup = false;
    let mut attempts = 0;
    loop {
        let rx = register_pending(inner, id);
        if let Err(e) = send_packet(inner, &build(dup)).await {
            forget_pending(inner, id);
            return Err(e);
        }
        match wait_for(inner, id, rx).await {
            Err(MqttError::Timeout) if attempts < max_retries => {
                attempts += 1;
                dup = true;
            }
            other => return other,
        }
    }
}

/// Read exactly one packet during the CONNECT/CONNACK handshake, before
/// the background read loop exists.
pub(super) async fn read_one_packet(
    reader: &mut ReadHalf<Box<dyn Transport>>,
    buf: &mut BytesMut,
    version: MqttVersion,
    timeout: Duration,
    max_packet_size: usize,
) -> MqttResult<Packet> {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(packet) = Packet::decode_with_limit(buf, version, max_packet_size)? {
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
    max_packet_size: usize,
) {
    loop {
        match Packet::decode_with_limit(&mut buf, inner.version, max_packet_size) {
            Ok(Some(packet)) => {
                inner.touch();
                if !handle_incoming(&inner, packet).await {
                    return;
                }
            }
            Ok(None) => match reader.read_buf(&mut buf).await {
                Ok(0) => {
                    finish_disconnected(&inner, "connection closed by peer".into());
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    finish_disconnected(&inner, format!("read error: {e}"));
                    return;
                }
            },
            Err(e) => {
                finish_disconnected(&inner, format!("protocol error: {e}"));
                return;
            }
        }
    }
}

/// Dispatch one packet decoded from the broker: deliver PUBLISHes (driving
/// the QoS 1/2 ack handshakes), resolve pending client-initiated
/// operations (PUBACK/PUBREC/PUBCOMP/SUBACK/UNSUBACK), and answer
/// PINGRESP/DISCONNECT. Returns `false` if the connection should be
/// considered terminated.
pub(super) async fn handle_incoming(inner: &Arc<Inner>, packet: Packet) -> bool {
    // Acks that complete a client-initiated request (publish handshake
    // steps, SUBACK, UNSUBACK) all resolve the same way.
    if let Some(id) = packet.response_id() {
        resolve_pending(inner, id, packet);
        return true;
    }

    match packet {
        Packet::Publish(p) => {
            let message = MqttMessage::from(&p);
            match (p.qos, p.packet_id) {
                (QoS::AtMostOnce, _) => deliver(inner, message),
                (QoS::AtLeastOnce, Some(id)) => {
                    deliver(inner, message);
                    let _ = send_packet(inner, &Packet::PubAck(SimpleAck::success(id))).await;
                }
                (QoS::ExactlyOnce, Some(id)) => {
                    inner.incoming_qos2.lock_safe().insert(id, message);
                    let _ = send_packet(inner, &Packet::PubRec(SimpleAck::success(id))).await;
                }
                // QoS > 0 without a packet id is rejected by the decoder.
                (_, None) => {}
            }
            true
        }
        Packet::PubRel(ack) => {
            let message = inner.incoming_qos2.lock_safe().remove(&ack.packet_id);
            if let Some(message) = message {
                deliver(inner, message);
            }
            let _ = send_packet(inner, &Packet::PubComp(SimpleAck::success(ack.packet_id))).await;
            true
        }
        Packet::Disconnect(d) => {
            finish_disconnected(
                inner,
                format!("server disconnected (reason 0x{:02X})", d.reason_code),
            );
            false
        }
        // PINGRESP only needs to refresh the activity timestamp (done by the
        // read loop); AUTH and anything unexpected from a broker is ignored.
        _ => true,
    }
}

/// Background task: send a PINGREQ every `0.8 * keep_alive_secs`, per
/// MQTT-3.1.2-23's "should send well before" guidance, and declare the
/// connection dead if the broker has sent *nothing* for 1.5x the keep-alive
/// interval — the half-open-socket case (cable pulled, NAT entry expired)
/// where writes appear to succeed forever but no PINGRESP ever returns.
/// Exits once the connection is marked disconnected.
pub(super) async fn keepalive_loop(inner: Arc<Inner>, keep_alive_secs: u16) {
    let keep_alive = Duration::from_secs(u64::from(keep_alive_secs).max(1));
    let interval = keep_alive.mul_f32(0.8);
    let dead_after = keep_alive.mul_f32(1.5);
    let mut ticker = tokio::time::interval(interval);
    // `tokio::time::interval`'s first `.tick()` resolves immediately, not
    // after `interval` — without this, every connection would send a
    // spurious PINGREQ right after CONNECT instead of waiting a full
    // keep-alive interval like every subsequent ping does.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if !inner.is_connected() {
            return;
        }
        if inner.idle_for() > dead_after {
            finish_disconnected(&inner, "keep-alive timeout: no data from broker".into());
            // The read loop is likely parked on a dead socket; stop it so
            // the socket is released instead of lingering until the OS
            // notices.
            if let Some(handle) = inner.read_task.lock().await.take() {
                handle.abort();
            }
            return;
        }
        if send_packet(&inner, &Packet::PingReq).await.is_err() {
            // `send_packet` already reported the loss.
            return;
        }
    }
}
