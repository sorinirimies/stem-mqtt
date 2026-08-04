//! Per-connection task: CONNECT handshake, the read loop, and packet
//! dispatch into shared broker state.

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::{ConnAckPacket, ConnectReasonCode};
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::subscribe::{SubAckPacket, SubAckReasonCode, UnsubAckPacket};
use mqtt_client::{MqttVersion, QoS};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::broker::BrokerState;
use crate::session::{QueuedMessage, Subscription};
use crate::topic::{is_valid_filter, is_valid_topic_name};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Handle one accepted connection end-to-end — CONNECT handshake, session
/// (re)attachment, message loop, and teardown — generic over the byte
/// transport so the exact same logic drives both raw TCP
/// ([`crate::broker::MqttBroker::start`]) and MQTT-over-WebSocket
/// ([`crate::ws::WsByteStream`]) connections.
pub(crate) async fn handle_connection<S>(state: Arc<BrokerState>, stream: S, peer: String)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);
    let mut buf = BytesMut::with_capacity(1024);

    let connect = match read_connect(&mut reader, &mut buf).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(%peer, error = %e, "connection closed before valid CONNECT");
            return;
        }
    };

    let version = connect.version;
    let client_id = if connect.client_id.is_empty() {
        format!("anon-{}", uuid_like())
    } else {
        connect.client_id.clone()
    };

    // ── Authentication ───────────────────────────────────────────────────
    let authorized = {
        let provider = state.auth_provider.lock().unwrap().clone();
        match provider {
            Some(p) => p.authenticate(
                client_id.clone(),
                connect.username.clone(),
                connect.password.as_ref().map(|b| b.to_vec()),
            ),
            None => state.config.allow_anonymous || connect.username.is_some(),
        }
    };
    if !authorized {
        let mut writer = writer;
        let ack = Packet::ConnAck(ConnAckPacket {
            session_present: false,
            reason_code: ConnectReasonCode::BAD_USERNAME_OR_PASSWORD,
            properties: Properties::new(),
        });
        if let Ok(bytes) = ack.encode(version) {
            let _ = writer.write_all(&bytes).await;
        }
        return;
    }

    // ── Client-id capacity check ─────────────────────────────────────────
    if state.config.max_clients > 0 && state.client_count() >= state.config.max_clients {
        let ack = Packet::ConnAck(ConnAckPacket {
            session_present: false,
            reason_code: ConnectReasonCode(0x97), // Quota exceeded
            properties: Properties::new(),
        });
        let mut writer = writer;
        if let Ok(bytes) = ack.encode(version) {
            let _ = writer.write_all(&bytes).await;
        }
        return;
    }

    // ── Take over / create the session ───────────────────────────────────
    let (session_present, shutdown_prev) = state.sessions.attach(
        &client_id,
        version,
        connect.clean_start,
        connect.will.clone(),
        state.config.max_queued_per_client as usize,
    );
    if let Some(prev_shutdown) = shutdown_prev {
        let _ = prev_shutdown.send(());
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    state.sessions.set_sender(&client_id, tx);

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    state.sessions.set_shutdown_handle(&client_id, shutdown_tx);

    // Writer task: drains the channel to the socket.
    let mut writer = writer;
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });

    let connack = Packet::ConnAck(ConnAckPacket {
        session_present,
        reason_code: ConnectReasonCode::SUCCESS,
        properties: Properties::new(),
    });
    if !state.sessions.send_to(&client_id, &connack) {
        writer_task.abort();
        return;
    }

    state.sessions.flush_offline_queue(&client_id);
    state.events.notify_connected(&client_id);
    tracing::info!(%client_id, %peer, ?version, "client connected");

    let keep_alive = connect.keep_alive;
    let disconnect_reason;
    let mut graceful = false;

    loop {
        let read_fut = read_next_packet(&mut reader, &mut buf, version, keep_alive);
        tokio::select! {
            _ = &mut shutdown_rx => {
                disconnect_reason = "replaced by a new connection with the same client id".into();
                break;
            }
            result = read_fut => {
                match result {
                    Ok(packet) => {
                        if matches!(packet, Packet::Disconnect(_)) {
                            graceful = true;
                            disconnect_reason = "client disconnected".into();
                            handle_packet(&state, &client_id, version, packet).await;
                            break;
                        }
                        if !handle_packet(&state, &client_id, version, packet).await {
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
    state.detach_session(&client_id, graceful);
    state
        .events
        .notify_disconnected(&client_id, &disconnect_reason);
    tracing::info!(%client_id, reason = %disconnect_reason, "client disconnected");
}

async fn read_connect<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut BytesMut,
) -> mqtt_client::error::MqttResult<mqtt_client::protocol::connect::ConnectPacket> {
    // The version passed here is irrelevant: CONNECT decodes its own
    // protocol level from the packet body.
    let packet = tokio::time::timeout(CONNECT_TIMEOUT, async {
        loop {
            if let Some(packet) = Packet::decode(buf, MqttVersion::V311)? {
                return Ok(packet);
            }
            let n = reader.read_buf(buf).await?;
            if n == 0 {
                return Err(mqtt_client::MqttError::Io(
                    "connection closed before CONNECT".into(),
                ));
            }
        }
    })
    .await
    .map_err(|_| mqtt_client::MqttError::Timeout)??;

    match packet {
        Packet::Connect(c) => Ok(c),
        other => Err(mqtt_client::MqttError::MalformedPacket(format!(
            "expected CONNECT, got {other:?}"
        ))),
    }
}

async fn read_next_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut BytesMut,
    version: MqttVersion,
    keep_alive_secs: u16,
) -> mqtt_client::error::MqttResult<Packet> {
    let body = async {
        loop {
            if let Some(packet) = Packet::decode(buf, version)? {
                return Ok(packet);
            }
            let n = reader.read_buf(buf).await?;
            if n == 0 {
                return Err(mqtt_client::MqttError::Io("connection closed".into()));
            }
        }
    };
    if keep_alive_secs == 0 {
        return body.await;
    }
    // MQTT-3.1.2-24 / MQTT-5.0: a server MAY disconnect a client that fails
    // to send any control packet within 1.5x the keep-alive interval.
    let timeout = Duration::from_secs((keep_alive_secs as u64 * 3) / 2).max(Duration::from_secs(1));
    tokio::time::timeout(timeout, body)
        .await
        .map_err(|_| mqtt_client::MqttError::Timeout)?
}

/// Process one packet received from `client_id`. Returns `false` if the
/// connection should be torn down (protocol violation).
async fn handle_packet(
    state: &Arc<BrokerState>,
    client_id: &str,
    version: MqttVersion,
    packet: Packet,
) -> bool {
    match packet {
        Packet::Publish(p) => handle_publish(state, client_id, version, p).await,
        Packet::PubRel(ack) => {
            if let Some(message) = state.sessions.take_incoming_qos2(client_id, ack.packet_id) {
                dispatch_publish(state, client_id, &message).await;
            }
            state.sessions.send_to(
                client_id,
                &Packet::PubComp(SimpleAck::success(ack.packet_id)),
            );
            true
        }
        Packet::PubAck(_) => true, // no redelivery tracking for QoS 1 in this implementation
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
        Packet::PubComp(_) => true, // QoS 2 handshake we initiated is now complete
        Packet::Subscribe(p) => {
            handle_subscribe(state, client_id, version, p).await;
            true
        }
        Packet::Unsubscribe(p) => {
            handle_unsubscribe(state, client_id, version, p).await;
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

async fn handle_publish(
    state: &Arc<BrokerState>,
    client_id: &str,
    version: MqttVersion,
    p: PublishPacket,
) -> bool {
    if !is_valid_topic_name(&p.topic) {
        return false;
    }

    if p.retain {
        state.retained.update(&p);
    }

    match p.qos {
        QoS::AtMostOnce => {
            dispatch_publish(state, client_id, &to_queued(&p)).await;
        }
        QoS::AtLeastOnce => {
            dispatch_publish(state, client_id, &to_queued(&p)).await;
            if let Some(id) = p.packet_id {
                state
                    .sessions
                    .send_to(client_id, &Packet::PubAck(SimpleAck::success(id)));
            }
        }
        QoS::ExactlyOnce => {
            if let Some(id) = p.packet_id {
                state.sessions.store_incoming_qos2(client_id, id, p);
                state
                    .sessions
                    .send_to(client_id, &Packet::PubRec(SimpleAck::success(id)));
            }
        }
    }
    let _ = version;
    true
}

fn to_queued(p: &PublishPacket) -> QueuedMessage {
    QueuedMessage {
        topic: p.topic.clone(),
        payload: p.payload.clone(),
        qos: p.qos,
        retain: p.retain,
    }
}

/// Fan a message out to every matching subscriber (excluding `no_local`
/// subscribers publishing to their own topic), queueing it for offline
/// sessions.
async fn dispatch_publish(state: &Arc<BrokerState>, publisher_id: &str, message: &QueuedMessage) {
    state
        .events
        .notify_message_published(publisher_id, &message.topic, message.qos);
    state.sessions.fan_out(publisher_id, message);
}

async fn handle_subscribe(
    state: &Arc<BrokerState>,
    client_id: &str,
    version: MqttVersion,
    p: mqtt_client::protocol::subscribe::SubscribePacket,
) {
    let mut reason_codes = Vec::with_capacity(p.filters.len());
    let mut newly_subscribed = Vec::new();
    for filter in &p.filters {
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
            mqtt_client::protocol::subscribe::RetainHandling::SendAtSubscribe => true,
            mqtt_client::protocol::subscribe::RetainHandling::SendIfNewSubscription => is_new,
            mqtt_client::protocol::subscribe::RetainHandling::DoNotSend => false,
        };
        if should_send {
            state.send_matching_retained(client_id, &filter, qos);
        }
    }
    let _ = version;
}

async fn handle_unsubscribe(
    state: &Arc<BrokerState>,
    client_id: &str,
    _version: MqttVersion,
    p: mqtt_client::protocol::subscribe::UnsubscribePacket,
) {
    for filter in &p.topic_filters {
        state.sessions.remove_subscription(client_id, filter);
    }
    state.sessions.send_to(
        client_id,
        &Packet::UnsubAck(UnsubAckPacket {
            packet_id: p.packet_id,
            reason_codes: p.topic_filters.iter().map(|_| 0u8).collect(),
            properties: Properties::new(),
        }),
    );
}

// Best-effort unique suffix for anonymous client ids, without pulling in a
// UUID dependency.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}
