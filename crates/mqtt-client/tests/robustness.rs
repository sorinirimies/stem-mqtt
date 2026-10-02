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
