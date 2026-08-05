//! Auto-reconnect: when `ConnectOptions::auto_reconnect` is set,
//! `MqttClient` must notice a dropped connection and reconnect on its own,
//! replaying whatever it was subscribed to. Exercised against a minimal
//! fake broker (not the real `mqtt-broker`) that deliberately drops the
//! connection once, so the test controls exactly when the "network loss"
//! happens.

use bytes::BytesMut;
use mqtt_client::protocol::connect::ConnAckPacket;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::subscribe::SubAckPacket;
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

async fn read_packet(stream: &mut TcpStream, buf: &mut BytesMut) -> Packet {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(packet) =
                Packet::decode(buf, MqttVersion::V5).expect("packet should decode")
            {
                return packet;
            }
            let n = stream.read_buf(buf).await.expect("read should succeed");
            assert!(n > 0, "connection closed unexpectedly");
        }
    })
    .await
    .expect("timed out waiting for a packet")
}

async fn send_packet(stream: &mut TcpStream, packet: Packet) {
    let bytes = packet
        .encode(MqttVersion::V5)
        .expect("packet should encode");
    stream
        .write_all(&bytes)
        .await
        .expect("write should succeed");
}

async fn accept_and_handshake(listener: &TcpListener) -> TcpStream {
    let (mut stream, _peer) = listener.accept().await.unwrap();
    let mut buf = BytesMut::with_capacity(1024);
    match read_packet(&mut stream, &mut buf).await {
        Packet::Connect(_) => {}
        other => panic!("expected CONNECT, got {other:?}"),
    }
    send_packet(
        &mut stream,
        Packet::ConnAck(ConnAckPacket {
            session_present: false,
            reason_code: mqtt_client::protocol::connect::ConnectReasonCode::SUCCESS,
            properties: Properties::new(),
        }),
    )
    .await;
    stream
}

#[tokio::test]
async fn client_auto_reconnects_and_replays_subscriptions() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let fake_broker = tokio::spawn(async move {
        // ── First connection: handshake, one SUBSCRIBE, then drop ──────
        let mut stream = accept_and_handshake(&listener).await;
        let mut buf = BytesMut::with_capacity(1024);
        let first_subscribe = match read_packet(&mut stream, &mut buf).await {
            Packet::Subscribe(p) => p,
            other => panic!("expected SUBSCRIBE, got {other:?}"),
        };
        send_packet(
            &mut stream,
            Packet::SubAck(SubAckPacket {
                packet_id: first_subscribe.packet_id,
                reason_codes: vec![mqtt_client::protocol::subscribe::SubAckReasonCode::granted(
                    first_subscribe.filters[0].qos,
                )],
                properties: Properties::new(),
            }),
        )
        .await;
        // Simulate a network drop: just close the socket.
        drop(stream);

        // ── Second connection: the client's own auto-reconnect ─────────
        let mut stream = accept_and_handshake(&listener).await;
        let mut buf = BytesMut::with_capacity(1024);
        let replayed_subscribe = match read_packet(&mut stream, &mut buf).await {
            Packet::Subscribe(p) => p,
            other => panic!("expected a replayed SUBSCRIBE, got {other:?}"),
        };
        assert_eq!(
            replayed_subscribe.filters[0].topic_filter, first_subscribe.filters[0].topic_filter,
            "the reconnect must replay the same topic filter the client was subscribed to"
        );
        send_packet(
            &mut stream,
            Packet::SubAck(SubAckPacket {
                packet_id: replayed_subscribe.packet_id,
                reason_codes: vec![mqtt_client::protocol::subscribe::SubAckReasonCode::granted(
                    replayed_subscribe.filters[0].qos,
                )],
                properties: Properties::new(),
            }),
        )
        .await;

        // Keep the second connection open long enough for the test to
        // observe `is_connected() == true` before this task (and the
        // listener/stream it owns) is dropped.
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    let mut options = ConnectOptions::new(
        "127.0.0.1",
        port,
        "auto-reconnect-test-client",
        MqttVersion::V5,
    );
    options.auto_reconnect = true;
    options.reconnect_backoff_secs = 1;
    options.reconnect_max_backoff_secs = 1;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();
    client
        .subscribe("t/reconnect".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    // The fake broker drops the connection right after acking the
    // subscribe; give the client's read loop a moment to notice, then
    // wait past the 1s reconnect backoff for it to reconnect and replay.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !client.is_connected(),
        "the client should have noticed the dropped connection by now"
    );

    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        client.is_connected(),
        "the client should have auto-reconnected by now"
    );

    fake_broker.await.unwrap();
}
