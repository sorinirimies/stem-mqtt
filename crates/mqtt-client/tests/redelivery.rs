//! Client-side redelivery: `MqttClient::publish` must retry an unacked
//! QoS 1/2 step (with DUP=1) instead of giving up on the first timeout.
//!
//! This can't be exercised against the real `mqtt-broker` (it always acks
//! correctly and promptly) — hence a minimal fake TCP peer here that
//! deliberately withholds its first ack, so we can observe the client's
//! retry directly.

use bytes::BytesMut;
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::ConnAckPacket;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

/// Reads one packet off `stream`, using `buf` as the (persistent, across
/// calls) decode buffer.
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

#[tokio::test]
async fn client_retries_unacked_qos1_publish_with_dup_flag() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let fake_broker = tokio::spawn(async move {
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

        // First PUBLISH: must not have DUP set. Deliberately don't ack it.
        let first = match read_packet(&mut stream, &mut buf).await {
            Packet::Publish(p) => p,
            other => panic!("expected PUBLISH, got {other:?}"),
        };
        assert!(!first.dup, "the initial publish must not have DUP set");

        // The redelivered PUBLISH: same packet id/payload, DUP=1 this time.
        let second = match read_packet(&mut stream, &mut buf).await {
            Packet::Publish(p) => p,
            other => panic!("expected a redelivered PUBLISH, got {other:?}"),
        };
        assert!(second.dup, "the redelivered publish must have DUP set");
        assert_eq!(second.packet_id, first.packet_id);
        assert_eq!(second.payload, first.payload);

        send_packet(
            &mut stream,
            Packet::PubAck(SimpleAck::success(second.packet_id.unwrap())),
        )
        .await;
    });

    let mut options = ConnectOptions::new("127.0.0.1", port, "retry-test-client", MqttVersion::V5);
    // Short per-attempt timeout so this test doesn't take 15s+ to run —
    // the client retries 3 times by default, so worst case is
    // 4x this value.
    options.operation_timeout_secs = 1;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();

    // Must succeed once the fake broker acks the *second* attempt — proof
    // the retry loop actually completes the publish rather than just
    // timing out on the first unacked attempt.
    client
        .publish(
            "t/retry".into(),
            b"redeliver me".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .expect("publish should succeed once the redelivered attempt is acked");

    fake_broker.await.unwrap();
}

#[tokio::test]
async fn keepalive_does_not_fire_immediately_on_connect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let fake_broker = tokio::spawn(async move {
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

        // `tokio::time::interval`'s first tick resolves immediately, not
        // after the configured interval — without accounting for that, a
        // PINGREQ would arrive here almost instantly instead of after
        // ~0.8s (0.8 * the 1s keep_alive_secs below).
        let mut buf2 = BytesMut::with_capacity(1024);
        let immediate = timeout(
            Duration::from_millis(400),
            read_packet(&mut stream, &mut buf2),
        )
        .await;
        assert!(
            immediate.is_err(),
            "no PINGREQ should arrive this soon after CONNECT"
        );

        // But keep-alive must still actually work — a PINGREQ must arrive
        // well before the full 1s keep_alive_secs elapses.
        match read_packet(&mut stream, &mut buf2).await {
            Packet::PingReq => {}
            other => panic!("expected PINGREQ, got {other:?}"),
        }
    });

    let mut options =
        ConnectOptions::new("127.0.0.1", port, "keepalive-test-client", MqttVersion::V5);
    options.keep_alive_secs = 1;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();

    fake_broker.await.unwrap();
}
