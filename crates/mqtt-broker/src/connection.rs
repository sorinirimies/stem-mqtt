//! Per-connection task: CONNECT handshake, the read loop, and packet
//! dispatch into shared broker state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::{Bytes, BytesMut};
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::{ConnAckPacket, ConnectPacket, ConnectReasonCode};
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::subscribe::{
    RetainHandling, SubAckPacket, SubAckReasonCode, SubscribePacket, UnsubAckPacket,
    UnsubscribePacket,
};
use mqtt_client::support::{guard_callback, LockExt};
use mqtt_client::{MqttError, MqttVersion, QoS};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::broker::BrokerState;
use crate::registry::{AttachError, AttachRequest};
use crate::session::{QueuedMessage, ShutdownReason, Subscription};
use crate::topic::{is_valid_filter, is_valid_topic_name};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// MQTT 5.0 UNSUBACK reason code: "No subscription existed".
const UNSUB_NO_SUBSCRIPTION: u8 = 0x11;

/// Handle one accepted connection end-to-end — CONNECT handshake, session
/// (re)attachment, message loop, and teardown — generic over the byte
/// transport so the exact same logic drives both raw TCP
/// ([`crate::broker::MqttBroker::start`]) and MQTT-over-WebSocket
/// ([`crate::ws::WsByteStream`]) connections.
pub(crate) async fn handle_connection<S>(state: Arc<BrokerState>, stream: S, peer: String)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut buf = BytesMut::with_capacity(1024);
    let max_packet_size = state.config.max_packet_size_bytes();

    let connect = match read_connect(&mut reader, &mut buf, max_packet_size).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(%peer, error = %e, "connection closed before valid CONNECT");
            return;
        }
    };

    let version = connect.version;

    // MQTT-3.1.3-8: a zero-length client id is only acceptable together
    // with a clean session; otherwise the broker must refuse it.
    if connect.client_id.is_empty() && !connect.clean_start {
        reject(
            &mut writer,
            version,
            ConnectReasonCode::CLIENT_IDENTIFIER_NOT_VALID,
        )
        .await;
        return;
    }
    let client_id = if connect.client_id.is_empty() {
        generate_client_id()
    } else {
        connect.client_id.clone()
    };

    if !authenticate(&state, &client_id, &connect).await {
        reject(
            &mut writer,
            version,
            ConnectReasonCode::BAD_USERNAME_OR_PASSWORD,
        )
        .await;
        return;
    }

    // ── Take over / create the session ───────────────────────────────────
    let (tx, mut rx) = mpsc::channel::<Bytes>(state.config.outbound_queue_capacity());
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<ShutdownReason>();
    let attached = match state.sessions.attach(AttachRequest {
        client_id: client_id.clone(),
        version,
        clean_start: connect.clean_start,
        will: connect.will.clone(),
        max_queued: state.config.max_queued_per_client as usize,
        max_clients: state.config.max_clients,
        sender: tx,
        shutdown: shutdown_tx,
    }) {
        Ok(attached) => attached,
        Err(AttachError::QuotaExceeded) => {
            reject(&mut writer, version, ConnectReasonCode::QUOTA_EXCEEDED).await;
            return;
        }
    };
    let conn_id = attached.conn_id;

    // Writer task: drains the channel to the socket.
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });

    let connack = Packet::ConnAck(ConnAckPacket {
        session_present: attached.session_present,
        reason_code: ConnectReasonCode::SUCCESS,
        properties: Properties::new(),
    });
    if !state.sessions.send_to(&client_id, &connack) {
        writer_task.abort();
        state.detach_session(&client_id, conn_id, true);
        return;
    }

    state.sessions.flush_offline_queue(&client_id);
    state.events.notify_connected(&client_id);
    tracing::info!(%client_id, %peer, ?version, "client connected");

    let keep_alive = connect.keep_alive;
    let disconnect_reason: String;
    let mut graceful = false;

    loop {
        let read_fut =
            read_next_packet(&mut reader, &mut buf, version, keep_alive, max_packet_size);
        tokio::select! {
            reason = &mut shutdown_rx => {
                match reason {
                    Ok(ShutdownReason::TakenOver) => {
                        disconnect_reason = "replaced by a new connection with the same client id".into();
                    }
                    Ok(ShutdownReason::BrokerStopped) | Err(_) => {
                        // A stopping broker isn't the client's fault: no will.
                        graceful = true;
                        disconnect_reason = "broker stopped".into();
                    }
                }
                break;
            }
            result = read_fut => {
                match result {
                    Ok(packet) => {
                        if matches!(packet, Packet::Disconnect(_)) {
                            graceful = true;
                            disconnect_reason = "client disconnected".into();
                            handle_packet(&state, &client_id, packet).await;
                            break;
                        }
                        if !handle_packet(&state, &client_id, packet).await {
                            disconnect_reason = "protocol error".into();
                            break;
                        }
                    }
                    Err(e) => {
                        disconnect_reason = e.to_string();
                        break;
                    }
                }
            }
        }
    }

    writer_task.abort();
    state.detach_session(&client_id, conn_id, graceful);
    state
        .events
        .notify_disconnected(&client_id, &disconnect_reason);
    tracing::info!(%client_id, reason = %disconnect_reason, "client disconnected");
}

/// Decide whether `connect` may proceed: the registered
/// [`MqttAuthProvider`](crate::config::MqttAuthProvider) has the final say;
/// without one, anonymous access is governed by `allow_anonymous`.
///
/// The provider is foreign code that may block (a database or network
/// lookup is typical), so it runs on the blocking pool rather than
/// stalling an async worker thread — and a panicking provider means
/// "denied", never "allowed".
async fn authenticate(state: &Arc<BrokerState>, client_id: &str, connect: &ConnectPacket) -> bool {
    let provider = state.auth_provider.lock_safe().clone();
    let Some(provider) = provider else {
        return state.config.allow_anonymous || connect.username.is_some();
    };
    let client_id = client_id.to_string();
    let username = connect.username.clone();
    let password = connect.password.as_ref().map(|b| b.to_vec());
    tokio::task::spawn_blocking(move || {
        guard_callback("authenticate", || {
            provider.authenticate(client_id, username, password)
        })
        .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Refuse a connection: send a CONNACK carrying `reason`, then close.
async fn reject<W: AsyncWrite + Unpin>(
    writer: &mut W,
    version: MqttVersion,
    reason: ConnectReasonCode,
) {
    let ack = Packet::ConnAck(ConnAckPacket {
        session_present: false,
        reason_code: reason,
        properties: Properties::new(),
    });
    if let Ok(bytes) = ack.encode(version) {
        let _ = writer.write_all(&bytes).await;
    }
    let _ = writer.shutdown().await;
}

async fn read_connect<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut BytesMut,
    max_packet_size: usize,
) -> mqtt_client::error::MqttResult<ConnectPacket> {
    // The version passed here is irrelevant: CONNECT decodes its own
    // protocol level from the packet body.
    let packet = tokio::time::timeout(CONNECT_TIMEOUT, async {
        loop {
            if let Some(packet) =
                Packet::decode_with_limit(buf, MqttVersion::V311, max_packet_size)?
            {
                return Ok(packet);
            }
            let n = reader.read_buf(buf).await?;
            if n == 0 {
                return Err(MqttError::Io("connection closed before CONNECT".into()));
            }
        }
    })
    .await
    .map_err(|_| MqttError::Timeout)??;

    match packet {
        Packet::Connect(c) => Ok(c),
        other => Err(MqttError::MalformedPacket(format!(
            "expected CONNECT, got {other:?}"
        ))),
    }
}

async fn read_next_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut BytesMut,
    version: MqttVersion,
    keep_alive_secs: u16,
    max_packet_size: usize,
) -> mqtt_client::error::MqttResult<Packet> {
    let body = async {
        loop {
            if let Some(packet) = Packet::decode_with_limit(buf, version, max_packet_size)? {
                return Ok(packet);
            }
            let n = reader.read_buf(buf).await?;
            if n == 0 {
                return Err(MqttError::Io("connection closed".into()));
            }
        }
    };
    if keep_alive_secs == 0 {
        return body.await;
    }
    // MQTT-3.1.2-24 / MQTT-5.0: a server MAY disconnect a client that fails
    // to send any control packet within 1.5x the keep-alive interval.
    let timeout =
        Duration::from_secs((u64::from(keep_alive_secs) * 3) / 2).max(Duration::from_secs(1));
    tokio::time::timeout(timeout, body)
        .await
        .map_err(|_| MqttError::Timeout)?
}

/// Process one packet received from `client_id`. Returns `false` if the
/// connection should be torn down (protocol violation).
async fn handle_packet(state: &Arc<BrokerState>, client_id: &str, packet: Packet) -> bool {
    match packet {
        Packet::Publish(p) => handle_publish(state, client_id, p),
        Packet::PubRel(ack) => {
            if let Some(message) = state.sessions.take_incoming_qos2(client_id, ack.packet_id) {
                dispatch_publish(state, client_id, &message);
            }
            state.sessions.send_to(
                client_id,
                &Packet::PubComp(SimpleAck::success(ack.packet_id)),
            );
            true
        }
        // A subscriber acking (QoS 1) or completing (QoS 2) a PUBLISH *we* sent.
        Packet::PubAck(ack) | Packet::PubComp(ack) => {
            state
                .sessions
                .clear_pending_redelivery(client_id, ack.packet_id);
            true
        }
        Packet::PubRec(ack) => {
            // A subscriber acking a QoS 2 PUBLISH *we* sent — complete the
            // handshake by sending PUBREL (MQTT-5.0 §4.3.3). Ignored if the
            // packet id is unknown (e.g. a stray/duplicate PUBREC).
            if state
                .sessions
                .complete_outgoing_qos2(client_id, ack.packet_id)
            {
                state.sessions.send_to(
                    client_id,
                    &Packet::PubRel(SimpleAck::success(ack.packet_id)),
                );
            }
            true
        }
        Packet::Subscribe(p) => {
            handle_subscribe(state, client_id, p);
            true
        }
        Packet::Unsubscribe(p) => {
            handle_unsubscribe(state, client_id, p);
            true
        }
        Packet::PingReq => {
            state.sessions.send_to(client_id, &Packet::PingResp);
            true
        }
        Packet::Disconnect(_) => {
            state.sessions.discard_will(client_id);
            true
        }
        _ => false, // CONNECT/CONNACK/SUBACK/UNSUBACK/AUTH/PINGRESP are never valid from a client here
    }
}

fn handle_publish(state: &Arc<BrokerState>, client_id: &str, p: PublishPacket) -> bool {
    if !is_valid_topic_name(&p.topic) {
        return false;
    }

    if p.retain {
        state.retained.update(&p);
    }

    match (p.qos, p.packet_id) {
        (QoS::AtMostOnce, _) => dispatch_publish(state, client_id, &QueuedMessage::from(&p)),
        (QoS::AtLeastOnce, id) => {
            dispatch_publish(state, client_id, &QueuedMessage::from(&p));
            if let Some(id) = id {
                state
                    .sessions
                    .send_to(client_id, &Packet::PubAck(SimpleAck::success(id)));
            }
        }
        (QoS::ExactlyOnce, Some(id)) => {
            state.sessions.store_incoming_qos2(client_id, id, p);
            state
                .sessions
                .send_to(client_id, &Packet::PubRec(SimpleAck::success(id)));
        }
        (QoS::ExactlyOnce, None) => {}
    }
    true
}

/// Fan a message out to every matching subscriber (excluding `no_local`
/// subscribers publishing to their own topic), queueing it for offline
/// sessions.
fn dispatch_publish(state: &Arc<BrokerState>, publisher_id: &str, message: &QueuedMessage) {
    state
        .events
        .notify_message_published(publisher_id, &message.topic, message.qos);
    state.sessions.fan_out(publisher_id, message);
}

/// MQTT 5.0 shared subscriptions (`$share/<group>/<filter>`) aren't
/// implemented; refuse them explicitly rather than silently registering a
/// filter that can never match.
fn is_shared_subscription(filter: &str) -> bool {
    filter.starts_with("$share/")
}

fn handle_subscribe(state: &Arc<BrokerState>, client_id: &str, p: SubscribePacket) {
    let mut reason_codes = Vec::with_capacity(p.filters.len());
    let mut newly_subscribed = Vec::new();
    for filter in &p.filters {
        if is_shared_subscription(&filter.topic_filter) {
            reason_codes.push(SubAckReasonCode::SHARED_SUBSCRIPTIONS_NOT_SUPPORTED);
            continue;
        }
        if !is_valid_filter(&filter.topic_filter) {
            reason_codes.push(SubAckReasonCode::FAILURE);
            continue;
        }
        let granted_qos = filter.qos.min(state.config.max_qos);
        let is_new = state.sessions.add_subscription(
            client_id,
            &filter.topic_filter,
            Subscription {
                qos: granted_qos,
                no_local: filter.no_local,
                retain_as_published: filter.retain_as_published,
            },
        );
        reason_codes.push(SubAckReasonCode::granted(granted_qos));
        newly_subscribed.push((
            filter.topic_filter.clone(),
            granted_qos,
            filter.retain_handling,
            is_new,
        ));
    }

    state.sessions.send_to(
        client_id,
        &Packet::SubAck(SubAckPacket {
            packet_id: p.packet_id,
            reason_codes,
            properties: Properties::new(),
        }),
    );

    for (filter, qos, retain_handling, is_new) in newly_subscribed {
        let should_send = match retain_handling {
            RetainHandling::SendAtSubscribe => true,
            RetainHandling::SendIfNewSubscription => is_new,
            RetainHandling::DoNotSend => false,
        };
        if should_send {
            state.send_matching_retained(client_id, &filter, qos);
        }
    }
}

fn handle_unsubscribe(state: &Arc<BrokerState>, client_id: &str, p: UnsubscribePacket) {
    let reason_codes = p
        .topic_filters
        .iter()
        .map(|filter| {
            if state.sessions.remove_subscription(client_id, filter) {
                0x00
            } else {
                UNSUB_NO_SUBSCRIPTION
            }
        })
        .collect();
    state.sessions.send_to(
        client_id,
        &Packet::UnsubAck(UnsubAckPacket {
            packet_id: p.packet_id,
            reason_codes,
            properties: Properties::new(),
        }),
    );
}

/// A broker-assigned client id for a client that connected with an empty
/// one. A process-wide counter guarantees uniqueness within this process;
/// the timestamp keeps ids from colliding across broker restarts (relevant
/// to persistent sessions) without pulling in a UUID dependency.
fn generate_client_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "anon-{nanos:x}-{:x}",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_client_ids_are_unique() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| generate_client_id()).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn shared_subscription_filters_detected() {
        assert!(is_shared_subscription("$share/group/a/b"));
        assert!(!is_shared_subscription("a/$share/b"));
    }
}
