//! Client robustness against a misbehaving peer, driven by a minimal fake
//! TCP broker (the real `mqtt-broker` always behaves, so it can't exercise
//! these paths).

use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use mqtt_client::protocol::connect::{ConnAckPacket, ConnectReasonCode};
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::{
    ConnectOptions, MqttClient, MqttError, MqttMessage, MqttMessageListener, MqttVersion, QoS,
    TlsOptions,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration, Instant};

async fn read_packet(stream: &mut TcpStream, buf: &mut BytesMut) -> Packet {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(p) = Packet::decode(buf, MqttVersion::V5).unwrap() {
                return p;
            }
            assert!(stream.read_buf(buf).await.unwrap() > 0, "peer closed");
        }
    })
    .await
    .expect("timed out waiting for a packet")
}

/// Accepts one connection, completes the CONNECT/CONNACK handshake, then
/// hands the raw stream to `then`.
async fn fake_broker<F, Fut>(then: F) -> u16
where
    F: FnOnce(TcpStream, BytesMut) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        assert!(matches!(
            read_packet(&mut stream, &mut buf).await,
            Packet::Connect(_)
        ));
        let ack = Packet::ConnAck(ConnAckPacket {
            session_present: false,
            reason_code: ConnectReasonCode::SUCCESS,
            properties: Properties::new(),
        });
        stream
            .write_all(&ack.encode(MqttVersion::V5).unwrap())
            .await
            .unwrap();
        then(stream, buf).await;
    });
    port
}

#[derive(Default)]
struct Recorder {
    disconnects: Mutex<Vec<String>>,
}

impl MqttMessageListener for Recorder {
    fn on_message(&self, _message: MqttMessage) {}
    fn on_disconnected(&self, reason: String) {
        self.disconnects.lock().unwrap().push(reason);
    }
}

/// A broker that accepts the connection and then goes silent (a half-open
/// socket) must be detected via the missing PINGRESP — and reported once.
#[tokio::test]
async fn silent_broker_is_detected_by_keepalive_timeout() {
    let port = fake_broker(|stream, _buf| async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    })
    .await;

    let mut options = ConnectOptions::new("127.0.0.1", port, "ka", MqttVersion::V5);
    options.keep_alive_secs = 1;
    let client = MqttClient::new(options);
    let recorder = Arc::new(Recorder::default());
    client.set_message_listener(recorder.clone());
    client.connect().await.unwrap();

    let started = Instant::now();
    timeout(Duration::from_secs(6), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("keep-alive must declare a silent broker dead");
    assert!(
        started.elapsed() >= Duration::from_millis(1200),
        "must not give up before ~1.5x keep-alive"
    );
    let reasons = recorder.disconnects.lock().unwrap().clone();
    assert_eq!(reasons.len(), 1, "exactly one notification: {reasons:?}");
    assert!(reasons[0].contains("keep-alive"), "{reasons:?}");
}

/// When the connection dies, in-flight requests must fail immediately, not
/// sit out their whole operation timeout.
#[tokio::test]
async fn in_flight_publish_fails_fast_when_connection_drops() {
    let port = fake_broker(|mut stream, mut buf| async move {
        // Read the PUBLISH, never ack it, then slam the socket shut.
        assert!(matches!(
            read_packet(&mut stream, &mut buf).await,
            Packet::Publish(_)
        ));
        drop(stream);
    })
    .await;

    let mut options = ConnectOptions::new("127.0.0.1", port, "ff", MqttVersion::V5);
    options.operation_timeout_secs = 30;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();

    let started = Instant::now();
    let err = client
        .publish("t".into(), b"x".to_vec(), QoS::AtLeastOnce, false)
        .await
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "must fail fast, took {:?} ({err:?})",
        started.elapsed()
    );
    assert!(matches!(err, MqttError::Session(_)), "{err:?}");
}

/// A packet bigger than `max_packet_size` is a protocol violation, not
/// something to buffer.
#[tokio::test]
async fn oversized_incoming_packet_drops_the_connection() {
    let port = fake_broker(|mut stream, _buf| async move {
        // PUBLISH header announcing a 10 MiB body, then nothing.
        stream
            .write_all(&[0x30, 0x80, 0x80, 0xC0, 0x04])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    })
    .await;

    let mut options = ConnectOptions::new("127.0.0.1", port, "big", MqttVersion::V5);
    options.max_packet_size = 64 * 1024;
    let client = MqttClient::new(options);
    let recorder = Arc::new(Recorder::default());
    client.set_message_listener(recorder.clone());
    client.connect().await.unwrap();

    timeout(Duration::from_secs(5), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("oversized packet must end the connection");
    assert!(recorder.disconnects.lock().unwrap()[0].contains("too large"));
}

#[tokio::test]
async fn invalid_options_fail_before_touching_the_network() {
    let mut options = ConnectOptions::new("", 1883, "c", MqttVersion::V5);
    assert!(matches!(
        MqttClient::new(options.clone()).connect().await,
        Err(MqttError::Protocol(m)) if m.contains("host")
    ));
    options.host = "127.0.0.1".into();
    options.will = Some(mqtt_client::WillOptions {
        topic: "bad/+/topic".into(),
        payload: vec![],
        qos: QoS::AtMostOnce,
        retain: false,
    });
    assert!(matches!(
        MqttClient::new(options).connect().await,
        Err(MqttError::Protocol(m)) if m.contains("will topic")
    ));
}

#[test]
fn debug_output_never_contains_credentials() {
    let mut options = ConnectOptions::new("h", 1883, "c", MqttVersion::V5);
    options.username = Some("alice".into());
    options.password = Some(b"hunter2-password".to_vec());
    options.tls = Some(TlsOptions {
        client_key_pem: Some(b"-----BEGIN PRIVATE KEY-----SECRETKEY".to_vec()),
        ..TlsOptions::new()
    });
    let shown = format!("{options:?}");
    assert!(shown.contains("alice"), "non-secret fields stay visible");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("SECRETKEY"), "{shown}");
    assert!(
        !shown.contains("104, 117"),
        "no raw byte dump either: {shown}"
    );
}

/// A broker that answers CONNECT with endless AUTH challenges must not trap the
/// client in an unbounded loop.
#[tokio::test]
async fn endless_auth_challenges_are_cut_off() {
    use mqtt_client::protocol::packet::AuthPacket;
    use mqtt_client::MqttAuthHandler;

    struct Echo;
    impl MqttAuthHandler for Echo {
        fn respond(&self, _method: String, challenge: Vec<u8>) -> Option<Vec<u8>> {
            Some(challenge)
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        assert!(matches!(
            read_packet(&mut stream, &mut buf).await,
            Packet::Connect(_)
        ));
        for _ in 0..40 {
            let challenge = Packet::Auth(AuthPacket::continue_with(
                "X",
                bytes::Bytes::from_static(b"c"),
            ));
            if stream
                .write_all(&challenge.encode(MqttVersion::V5).unwrap())
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });

    let mut options = ConnectOptions::new("127.0.0.1", port, "loop", MqttVersion::V5);
    options.auth_method = Some("X".into());
    let client = MqttClient::new(options);
    client.set_auth_handler(Arc::new(Echo));
    let err = timeout(Duration::from_secs(10), client.connect())
        .await
        .expect("must give up instead of looping")
        .unwrap_err();
    assert!(
        matches!(&err, MqttError::ConnectionRefused(m) if m.contains("challenges")),
        "{err:?}"
    );
}

/// A broker-sent DISCONNECT must surface its reason once and mark the
/// client disconnected.
#[tokio::test]
async fn server_disconnect_is_reported_with_its_reason_code() {
    use mqtt_client::protocol::packet::DisconnectPacket;
    let port = fake_broker(|mut stream, _buf| async move {
        let pkt = Packet::Disconnect(DisconnectPacket {
            reason_code: 0x8B,
            properties: Properties::new(),
        });
        stream
            .write_all(&pkt.encode(MqttVersion::V5).unwrap())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    })
    .await;

    let client = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        port,
        "sd",
        MqttVersion::V5,
    ));
    let recorder = Arc::new(Recorder::default());
    client.set_message_listener(recorder.clone());
    client.connect().await.unwrap();

    timeout(Duration::from_secs(3), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("server DISCONNECT must end the session");
    let reasons = recorder.disconnects.lock().unwrap().clone();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("0x8B"), "{reasons:?}");
}

/// Every operation on a never-connected client fails cleanly with
/// `NotConnected` instead of hanging or panicking.
#[tokio::test]
async fn operations_before_connect_report_not_connected() {
    let client = MqttClient::new(ConnectOptions::new("127.0.0.1", 1, "nc", MqttVersion::V5));
    assert!(!client.is_connected());
    assert!(matches!(
        client.subscribe("a/b".into(), QoS::AtLeastOnce).await,
        Err(MqttError::NotConnected)
    ));
    assert!(matches!(
        client.unsubscribe("a/b".into()).await,
        Err(MqttError::NotConnected)
    ));
    assert!(client.next_message(10).await.is_none());
}

// ── Deterministic failure-path coverage ─────────────────────────────────────

use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::subscribe::{SubAckPacket, SubAckReasonCode};

async fn connected_client(port: u16, id: &str) -> MqttClient {
    let mut options = ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5);
    options.operation_timeout_secs = 2;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();
    client
}

async fn send(stream: &mut TcpStream, p: Packet) {
    stream
        .write_all(&p.encode(MqttVersion::V5).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn malformed_filters_are_rejected_client_side() {
    let port = fake_broker(|s, _b| async move {
        let _keep_open = s;
        tokio::time::sleep(Duration::from_secs(3)).await;
    })
    .await;
    let client = connected_client(port, "badfilter").await;
    assert!(matches!(
        client.subscribe("a/#/b".into(), QoS::AtMostOnce).await,
        Err(MqttError::Protocol(_))
    ));
    assert!(matches!(
        client.unsubscribe("a/#/b".into()).await,
        Err(MqttError::Protocol(_))
    ));
}

#[tokio::test]
async fn wrong_ack_type_is_reported_as_unexpected() {
    let port = fake_broker(|mut s, mut b| async move {
        let Packet::Subscribe(sub) = read_packet(&mut s, &mut b).await else {
            panic!("expected SUBSCRIBE");
        };
        send(&mut s, Packet::PubAck(SimpleAck::success(sub.packet_id))).await;
        let Packet::Unsubscribe(u) = read_packet(&mut s, &mut b).await else {
            panic!("expected UNSUBSCRIBE");
        };
        send(&mut s, Packet::PubAck(SimpleAck::success(u.packet_id))).await;
        let Packet::Publish(p) = read_packet(&mut s, &mut b).await else {
            panic!("expected PUBLISH");
        };
        send(
            &mut s,
            Packet::PubRec(SimpleAck::success(p.packet_id.unwrap())),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    })
    .await;
    let client = connected_client(port, "unexpected").await;
    assert!(matches!(
        client.subscribe("a/b".into(), QoS::AtMostOnce).await,
        Err(MqttError::MalformedPacket(_))
    ));
    assert!(matches!(
        client.unsubscribe("a/b".into()).await,
        Err(MqttError::MalformedPacket(_))
    ));
    // QoS 1 PUBLISH answered with PUBREC instead of PUBACK.
    assert!(matches!(
        client
            .publish("a/b".into(), b"x".to_vec(), QoS::AtLeastOnce, false)
            .await,
        Err(MqttError::MalformedPacket(_))
    ));
}

#[tokio::test]
async fn suback_failure_is_not_remembered_and_puback_failure_is_an_error() {
    let port = fake_broker(|mut s, mut b| async move {
        let Packet::Subscribe(sub) = read_packet(&mut s, &mut b).await else {
            panic!("expected SUBSCRIBE");
        };
        send(
            &mut s,
            Packet::SubAck(SubAckPacket {
                packet_id: sub.packet_id,
                reason_codes: vec![SubAckReasonCode(0x80)],
                properties: Properties::new(),
            }),
        )
        .await;
        let Packet::Publish(p) = read_packet(&mut s, &mut b).await else {
            panic!("expected PUBLISH");
        };
        let mut ack = SimpleAck::success(p.packet_id.unwrap());
        ack.reason_code = 0x87; // Not authorized
        send(&mut s, Packet::PubAck(ack)).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    })
    .await;
    let client = connected_client(port, "rejected").await;
    let r = client
        .subscribe("a/b".into(), QoS::AtMostOnce)
        .await
        .unwrap();
    assert_eq!(r.reason_code, 0x80);
    let err = client
        .publish("a/b".into(), b"x".to_vec(), QoS::AtLeastOnce, false)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, MqttError::Session(m) if m.contains("0x87")),
        "{err:?}"
    );
}

#[tokio::test]
async fn handshake_answered_with_a_non_connack_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = BytesMut::new();
        let _ = read_packet(&mut s, &mut b).await;
        send(&mut s, Packet::PingResp).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let client = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        port,
        "nc",
        MqttVersion::V5,
    ));
    assert!(matches!(
        client.connect().await,
        Err(MqttError::MalformedPacket(_))
    ));
}

#[tokio::test]
async fn second_connect_and_repeat_disconnect_are_benign() {
    let port = fake_broker(|s, _b| async move {
        let _keep_open = s;
        tokio::time::sleep(Duration::from_secs(3)).await;
    })
    .await;
    let client = connected_client(port, "twice").await;
    assert!(matches!(
        client.connect().await,
        Err(MqttError::AlreadyConnected)
    ));
    client.disconnect().await.unwrap();
    client.disconnect().await.unwrap();
    assert!(!client.is_connected());
}

/// A peer that stops reading must not wedge `publish`: the write times out
/// and the connection is declared dead.
#[tokio::test]
async fn stalled_write_times_out_and_kills_the_connection() {
    let port = fake_broker(|s, _b| async move {
        let _keep_open = s;
        // Never read again: socket buffers fill up.
        tokio::time::sleep(Duration::from_secs(20)).await;
    })
    .await;
    let mut options = ConnectOptions::new("127.0.0.1", port, "stall", MqttVersion::V5);
    options.operation_timeout_secs = 1;
    let client = MqttClient::new(options);
    let recorder = Arc::new(Recorder::default());
    client.set_message_listener(recorder.clone());
    client.connect().await.unwrap();

    let big = vec![0u8; 64 * 1024 * 1024];
    let err = client
        .publish("big".into(), big, QoS::AtMostOnce, false)
        .await
        .unwrap_err();
    assert!(matches!(err, MqttError::Timeout), "{err:?}");
    assert!(!client.is_connected());
    let reasons = recorder.disconnects.lock().unwrap().clone();
    assert!(
        reasons.iter().any(|r| r.contains("timed out")),
        "{reasons:?}"
    );
}

/// While the broker is unreachable the supervisor must keep retrying with
/// backoff, and a deliberate disconnect must stop it.
#[tokio::test]
async fn reconnect_supervisor_retries_then_stops_on_disconnect() {
    let port = fake_broker(|stream, _b| async move {
        drop(stream); // and the listener goes away with this task
    })
    .await;
    let mut options = ConnectOptions::new("127.0.0.1", port, "retry", MqttVersion::V5);
    options.auto_reconnect = true;
    options.reconnect_backoff_secs = 1;
    options.reconnect_max_backoff_secs = 1;
    let client = MqttClient::new(options);
    client.connect().await.unwrap();

    timeout(Duration::from_secs(3), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("drop must be noticed");
    // Let at least one (failing) reconnect attempt happen.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!client.is_connected());
    client.disconnect().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!client.is_connected(), "supervisor must stay stopped");
}

#[tokio::test]
async fn qos2_handshake_with_wrong_ack_types_is_reported() {
    let port = fake_broker(|mut s, mut b| async move {
        // 1st PUBLISH: answer PUBACK instead of PUBREC.
        let Packet::Publish(p) = read_packet(&mut s, &mut b).await else {
            panic!("expected PUBLISH");
        };
        send(
            &mut s,
            Packet::PubAck(SimpleAck::success(p.packet_id.unwrap())),
        )
        .await;
        // 2nd PUBLISH: PUBREC is right, but answer PUBREL with PUBACK.
        let Packet::Publish(p) = read_packet(&mut s, &mut b).await else {
            panic!("expected PUBLISH");
        };
        let id = p.packet_id.unwrap();
        send(&mut s, Packet::PubRec(SimpleAck::success(id))).await;
        assert!(matches!(
            read_packet(&mut s, &mut b).await,
            Packet::PubRel(_)
        ));
        send(&mut s, Packet::PubAck(SimpleAck::success(id))).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    })
    .await;
    let client = connected_client(port, "q2wrong").await;
    for _ in 0..2 {
        assert!(matches!(
            client
                .publish("a".into(), b"x".to_vec(), QoS::ExactlyOnce, false)
                .await,
            Err(MqttError::MalformedPacket(_))
        ));
    }
}
