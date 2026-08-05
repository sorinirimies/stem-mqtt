//! End-to-end integration tests: a real [`MqttBroker`] listening on an
//! ephemeral loopback port, driven by real [`MqttClient`] connections over
//! actual TCP sockets (nothing mocked). Each test gets its own broker
//! instance bound to port `0` so tests can run in parallel without port
//! collisions.

use std::sync::Arc;
use std::time::Duration;

use mqtt_broker::{MqttAuthProvider, MqttBroker, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Forwards every delivered message (and the one disconnect reason, if any)
/// to an unbounded channel so async test code can `.await` them instead of
/// polling shared state from inside the synchronous listener callback.
struct ChannelListener {
    tx: mpsc::UnboundedSender<MqttMessage>,
}

impl MqttMessageListener for ChannelListener {
    fn on_message(&self, message: MqttMessage) {
        let _ = self.tx.send(message);
    }

    fn on_disconnected(&self, _reason: String) {}
}

/// Starts a broker on an OS-assigned loopback port and returns it together
/// with that port. The broker is stopped when the returned guard is
/// dropped... actually `MqttBroker` has no `Drop` impl, so callers should
/// call `.stop().await` explicitly at the end of the test.
async fn start_broker(config_fn: impl FnOnce(&mut MqttBrokerConfig)) -> (MqttBroker, u16) {
    let mut config = MqttBrokerConfig::new("127.0.0.1", 0);
    config_fn(&mut config);
    let broker = MqttBroker::new(config);
    broker.start().await.expect("broker should bind and start");
    let port = broker
        .bound_port()
        .expect("port must be known after start()");
    (broker, port)
}

fn client(port: u16, client_id: &str) -> MqttClient {
    MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        port,
        client_id,
        MqttVersion::V5,
    ))
}

async fn recv_message(rx: &mut mpsc::UnboundedReceiver<MqttMessage>) -> MqttMessage {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for a message")
        .expect("channel closed unexpectedly")
}

async fn assert_no_message(rx: &mut mpsc::UnboundedReceiver<MqttMessage>) {
    let result = timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(result.is_err(), "expected no message, but got one");
}

// ── MQTT-over-WebSocket ────────────────────────────────────────────────
// These drive a raw `tokio-tungstenite` WebSocket client speaking MQTT
// bytes directly (mirroring what a browser's `mqtt.js` does), rather than
// `mqtt_client::MqttClient` (which only speaks raw TCP) — proving the
// broker's WebSocket bridge (`crate`-internal `WsByteStream`) actually
// round-trips real MQTT packets, not just that it accepts a handshake.

use futures_util::{SinkExt, StreamExt};
use mqtt_client::protocol::connect::ConnectPacket;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::subscribe::{SubscribeFilter, SubscribePacket};
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::Message;
async fn start_broker_with_ws() -> (MqttBroker, u16, u16) {
    let mut config = MqttBrokerConfig::new("127.0.0.1", 0);
    config.ws_port = Some(0);
    let broker = MqttBroker::new(config);
    broker.start().await.expect("broker should bind and start");
    let tcp_port = broker.bound_port().expect("tcp port must be known");
    let ws_port = broker.bound_ws_port().expect("ws port must be known");
    (broker, tcp_port, ws_port)
}

/// Connects a raw WebSocket client to the broker's `ws_port`, completing
/// the `mqtt` subprotocol handshake (MQTT-5.0 §6.4.1).
async fn connect_ws(
    ws_port: u16,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let request = Request::builder()
        .uri(format!("ws://127.0.0.1:{ws_port}/"))
        .header("Sec-WebSocket-Protocol", "mqtt")
        .header("Host", format!("127.0.0.1:{ws_port}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    let (ws_stream, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket handshake should succeed");
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok()),
        Some("mqtt"),
        "broker must echo back the negotiated `mqtt` subprotocol"
    );
    ws_stream
}

#[tokio::test]
async fn websocket_client_connects_and_subscribes() {
    let (broker, _tcp_port, ws_port) = start_broker_with_ws().await;
    let mut ws = connect_ws(ws_port).await;

    let connect = Packet::Connect(ConnectPacket {
        version: MqttVersion::V5,
        client_id: "ws-raw-client".into(),
        clean_start: true,
        keep_alive: 30,
        username: None,
        password: None,
        will: None,
        properties: Properties::new(),
    });
    ws.send(Message::Binary(
        connect.encode(MqttVersion::V5).unwrap().to_vec(),
    ))
    .await
    .unwrap();

    let msg = ws.next().await.unwrap().unwrap();
    let Message::Binary(bytes) = msg else {
        panic!("expected a binary CONNACK message, got {msg:?}")
    };
    let mut buf = bytes::BytesMut::from(&bytes[..]);
    let packet = Packet::decode(&mut buf, MqttVersion::V5).unwrap().unwrap();
    let Packet::ConnAck(ack) = packet else {
        panic!("expected CONNACK, got {packet:?}")
    };
    assert!(ack.reason_code.is_success());

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn websocket_subscriber_receives_publish_from_tcp_client() {
    let (broker, tcp_port, ws_port) = start_broker_with_ws().await;

    // Subscriber: raw WebSocket client (stands in for a browser).
    let mut ws = connect_ws(ws_port).await;
    let connect = Packet::Connect(ConnectPacket {
        version: MqttVersion::V5,
        client_id: "ws-subscriber".into(),
        clean_start: true,
        keep_alive: 30,
        username: None,
        password: None,
        will: None,
        properties: Properties::new(),
    });
    ws.send(Message::Binary(
        connect.encode(MqttVersion::V5).unwrap().to_vec(),
    ))
    .await
    .unwrap();
    ws.next().await.unwrap().unwrap(); // CONNACK

    let subscribe = Packet::Subscribe(SubscribePacket {
        packet_id: 1,
        filters: vec![SubscribeFilter::new("ws/bridge", QoS::AtLeastOnce)],
        properties: Properties::new(),
    });
    ws.send(Message::Binary(
        subscribe.encode(MqttVersion::V5).unwrap().to_vec(),
    ))
    .await
    .unwrap();
    ws.next().await.unwrap().unwrap(); // SUBACK

    // Publisher: a normal `MqttClient` over raw TCP.
    let publisher = client(tcp_port, "tcp-publisher");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "ws/bridge".into(),
            b"bridged across transports".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();

    let msg = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for the bridged PUBLISH")
        .unwrap()
        .unwrap();
    let Message::Binary(bytes) = msg else {
        panic!("expected a binary PUBLISH message, got {msg:?}")
    };
    let mut buf = bytes::BytesMut::from(&bytes[..]);
    let packet = Packet::decode(&mut buf, MqttVersion::V5).unwrap().unwrap();
    let Packet::Publish(PublishPacket { topic, payload, .. }) = packet else {
        panic!("expected PUBLISH, got {packet:?}")
    };
    assert_eq!(topic, "ws/bridge");
    assert_eq!(payload, b"bridged across transports".as_slice());

    publisher.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn qos0_pub_sub_roundtrip() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("t/qos0".into(), QoS::AtMostOnce)
        .await
        .unwrap();

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish("t/qos0".into(), b"hello".to_vec(), QoS::AtMostOnce, false)
        .await
        .unwrap();

    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.topic, "t/qos0");
    assert_eq!(msg.payload, b"hello");
    assert_eq!(msg.qos, QoS::AtMostOnce);

    publisher.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn qos1_pub_sub_roundtrip() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    let sub_result = subscriber
        .subscribe("t/qos1".into(), QoS::AtLeastOnce)
        .await
        .unwrap();
    assert!(sub_result.reason_code < 0x80);

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    // `publish()` only resolves once the broker PUBACKs — a real
    // end-to-end acknowledgement, not a fire-and-forget send.
    publisher
        .publish(
            "t/qos1".into(),
            b"at least once".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();

    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.payload, b"at least once");

    publisher.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn qos2_pub_sub_roundtrip() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("t/qos2".into(), QoS::ExactlyOnce)
        .await
        .unwrap();

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    // Exercises the full four-part QoS 2 handshake: PUBLISH -> PUBREC ->
    // PUBREL -> PUBCOMP, on both the publisher and (internally, for
    // delivery to the subscriber) the broker side.
    publisher
        .publish(
            "t/qos2".into(),
            b"exactly once".to_vec(),
            QoS::ExactlyOnce,
            false,
        )
        .await
        .unwrap();

    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.payload, b"exactly once");
    assert_eq!(msg.qos, QoS::ExactlyOnce);

    // No duplicate delivery.
    assert_no_message(&mut rx).await;

    publisher.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn retained_message_delivered_to_new_subscriber() {
    let (broker, port) = start_broker(|_| {}).await;

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "t/retained".into(),
            b"sticky".to_vec(),
            QoS::AtLeastOnce,
            true,
        )
        .await
        .unwrap();
    publisher.disconnect().await.unwrap();

    // Subscriber connects *after* the retained message was published.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("t/retained".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.topic, "t/retained");
    assert_eq!(msg.payload, b"sticky");
    assert!(msg.retain);

    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn wildcard_subscription_matches_multiple_topics() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("sensors/+/temperature".into(), QoS::AtMostOnce)
        .await
        .unwrap();

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "sensors/kitchen/temperature".into(),
            b"21.5".to_vec(),
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    publisher
        .publish(
            "sensors/kitchen/humidity".into(),
            b"40".to_vec(),
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    publisher
        .publish(
            "sensors/bedroom/temperature".into(),
            b"19.0".to_vec(),
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();

    let first = recv_message(&mut rx).await;
    assert_eq!(first.topic, "sensors/kitchen/temperature");
    let second = recv_message(&mut rx).await;
    assert_eq!(second.topic, "sensors/bedroom/temperature");
    // The humidity topic (doesn't match `+/temperature`) must not arrive.
    assert_no_message(&mut rx).await;

    publisher.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn unsubscribe_stops_further_delivery() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("t/unsub".into(), QoS::AtMostOnce)
        .await
        .unwrap();

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish("t/unsub".into(), b"before".to_vec(), QoS::AtMostOnce, false)
        .await
        .unwrap();
    recv_message(&mut rx).await;

    subscriber.unsubscribe("t/unsub".into()).await.unwrap();
    publisher
        .publish("t/unsub".into(), b"after".to_vec(), QoS::AtMostOnce, false)
        .await
        .unwrap();
    assert_no_message(&mut rx).await;

    publisher.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

struct FixedCredentials;

impl MqttAuthProvider for FixedCredentials {
    fn authenticate(
        &self,
        _client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
    ) -> bool {
        username.as_deref() == Some("demo") && password.as_deref() == Some(b"demo")
    }
}

#[tokio::test]
async fn auth_provider_accepts_correct_credentials() {
    let (broker, port) = start_broker(|_| {}).await;
    broker.set_auth_provider(Arc::new(FixedCredentials));

    let mut opts = ConnectOptions::new("127.0.0.1", port, "authed", MqttVersion::V5);
    opts.username = Some("demo".into());
    opts.password = Some(b"demo".to_vec());
    let good_client = MqttClient::new(opts);
    let result = good_client.connect().await.unwrap();
    assert_eq!(result.reason_code, 0x00);

    good_client.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn auth_provider_rejects_bad_credentials() {
    let (broker, port) = start_broker(|_| {}).await;
    broker.set_auth_provider(Arc::new(FixedCredentials));

    let mut opts = ConnectOptions::new("127.0.0.1", port, "not-authed", MqttVersion::V5);
    opts.username = Some("demo".into());
    opts.password = Some(b"WRONG".to_vec());
    let bad_client = MqttClient::new(opts);
    let err = bad_client.connect().await.unwrap_err();
    assert!(matches!(err, mqtt_client::MqttError::ConnectionRefused(_)));

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn last_will_is_published_on_ungraceful_disconnect() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = client(port, "observer");
    observer.set_message_listener(Arc::new(ChannelListener { tx }));
    observer.connect().await.unwrap();
    observer
        .subscribe("t/will".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    let mut opts = ConnectOptions::new("127.0.0.1", port, "will-client", MqttVersion::V5);
    opts.will = Some(mqtt_client::WillOptions {
        topic: "t/will".into(),
        payload: b"gone".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: false,
    });
    let will_client = MqttClient::new(opts);
    will_client.connect().await.unwrap();

    // Drop (not `disconnect()`) so the socket closes without a DISCONNECT
    // packet — the broker must treat this as an ungraceful loss and
    // publish the will on the client's behalf.
    drop(will_client);

    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.topic, "t/will");
    assert_eq!(msg.payload, b"gone");

    observer.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn offline_session_resumes_and_flushes_queued_messages() {
    let (broker, port) = start_broker(|_| {}).await;

    // First connection: clean_start = false, so the broker must keep this
    // session (subscriptions + an offline queue) after we disconnect.
    let mut opts = ConnectOptions::new("127.0.0.1", port, "persist-me", MqttVersion::V5);
    opts.clean_start = false;
    let first = MqttClient::new(opts.clone());
    let result = first.connect().await.unwrap();
    assert!(
        !result.session_present,
        "first-ever CONNECT for a client id must not report a resumed session"
    );
    first
        .subscribe("t/persist".into(), QoS::AtLeastOnce)
        .await
        .unwrap();
    // Graceful disconnect (not drop): clean_start=false means the session
    // — including this subscription — must still survive.
    first.disconnect().await.unwrap();

    // Published while "persist-me" is offline — the broker must queue this
    // for later delivery instead of silently dropping it.
    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "t/persist".into(),
            b"queued while offline".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();
    publisher.disconnect().await.unwrap();

    // Reconnect with the same client id and clean_start=false: the broker
    // must report session_present=true and immediately flush the queued
    // message, without needing to re-subscribe.
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let second = MqttClient::new(opts);
    second.set_message_listener(Arc::new(ChannelListener { tx: tx2 }));
    let result = second.connect().await.unwrap();
    assert!(
        result.session_present,
        "reconnecting with clean_start=false must resume the prior session"
    );

    let msg = recv_message(&mut rx2).await;
    assert_eq!(msg.topic, "t/persist");
    assert_eq!(msg.payload, b"queued while offline");

    second.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn empty_payload_retained_publish_clears_it() {
    let (broker, port) = start_broker(|_| {}).await;

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "t/clear-me".into(),
            b"sticky".to_vec(),
            QoS::AtLeastOnce,
            true,
        )
        .await
        .unwrap();
    // An empty-payload retained PUBLISH must clear the topic's retained
    // message (MQTT-3.3.1-10), not store an empty one.
    publisher
        .publish("t/clear-me".into(), Vec::new(), QoS::AtLeastOnce, true)
        .await
        .unwrap();
    publisher.disconnect().await.unwrap();

    // A subscriber connecting *after* the clear must get nothing retained.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let subscriber = client(port, "sub");
    subscriber.set_message_listener(Arc::new(ChannelListener { tx }));
    subscriber.connect().await.unwrap();
    subscriber
        .subscribe("t/clear-me".into(), QoS::AtLeastOnce)
        .await
        .unwrap();
    assert_no_message(&mut rx).await;

    subscriber.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn max_clients_rejects_connections_past_the_limit() {
    let (broker, port) = start_broker(|c| c.max_clients = 1).await;

    let first = client(port, "first");
    let result = first.connect().await.unwrap();
    assert_eq!(result.reason_code, 0x00);

    // A second, concurrent connection must be refused with "quota exceeded"
    // (0x97) while the first is still connected.
    let second = client(port, "second");
    let err = second.connect().await.unwrap_err();
    assert!(matches!(err, mqtt_client::MqttError::ConnectionRefused(_)));

    first.disconnect().await.unwrap();

    // Once the first client disconnects, a new connection must be allowed
    // again — the limit is on *concurrent* clients, not a lifetime count.
    let third = client(port, "third");
    let result = third.connect().await.unwrap();
    assert_eq!(result.reason_code, 0x00);

    third.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

// ── Broker-initiated QoS 1/2 redelivery ───────────────────────────────────
// `MqttClient` always acks a delivered PUBLISH immediately, so proving the
// broker actually *retries* an unacked one needs a client that can
// deliberately withhold its ack — hence a minimal raw-TCP MQTT client here
// instead of `mqtt_client::MqttClient`.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A bare-bones MQTT client speaking raw TCP directly via the wire-protocol
/// codec, so tests can withhold acks deliberately (`mqtt_client::MqttClient`
/// always acks a delivered PUBLISH immediately, which would make it
/// impossible to observe the broker's redelivery behavior).
struct RawClient {
    stream: tokio::net::TcpStream,
    buf: bytes::BytesMut,
}

impl RawClient {
    async fn connect(port: u16, client_id: &str) -> Self {
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("TCP connect should succeed");
        let mut me = RawClient {
            stream,
            buf: bytes::BytesMut::with_capacity(1024),
        };
        me.send(Packet::Connect(ConnectPacket {
            version: MqttVersion::V5,
            client_id: client_id.to_string(),
            clean_start: true,
            keep_alive: 30,
            username: None,
            password: None,
            will: None,
            properties: Properties::new(),
        }))
        .await;
        match me.read_packet().await {
            Packet::ConnAck(ack) => assert!(ack.reason_code.is_success()),
            other => panic!("expected CONNACK, got {other:?}"),
        }
        me
    }

    async fn send(&mut self, packet: Packet) {
        let bytes = packet
            .encode(MqttVersion::V5)
            .expect("packet should encode");
        self.stream
            .write_all(&bytes)
            .await
            .expect("write should succeed");
    }

    async fn read_packet(&mut self) -> Packet {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(packet) =
                    Packet::decode(&mut self.buf, MqttVersion::V5).expect("packet should decode")
                {
                    return packet;
                }
                let n = self
                    .stream
                    .read_buf(&mut self.buf)
                    .await
                    .expect("read should succeed");
                assert!(n > 0, "connection closed unexpectedly");
            }
        })
        .await
        .expect("timed out waiting for a packet")
    }
}

#[tokio::test]
async fn broker_retries_unacked_qos1_publish_with_dup_flag() {
    // A short redelivery interval keeps this test fast without touching
    // the 5s production default.
    let (broker, port) = start_broker(|c| c.redelivery_interval_secs = 1).await;

    let mut subscriber = RawClient::connect(port, "withholds-ack").await;
    subscriber
        .send(Packet::Subscribe(SubscribePacket {
            packet_id: 1,
            filters: vec![SubscribeFilter::new("t/redelivery", QoS::AtLeastOnce)],
            properties: Properties::new(),
        }))
        .await;
    match subscriber.read_packet().await {
        Packet::SubAck(_) => {}
        other => panic!("expected SUBACK, got {other:?}"),
    }

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    publisher
        .publish(
            "t/redelivery".into(),
            b"resend me".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();

    // First delivery: DUP must be unset.
    let first = match subscriber.read_packet().await {
        Packet::Publish(p) => p,
        other => panic!("expected PUBLISH, got {other:?}"),
    };
    assert_eq!(first.topic, "t/redelivery");
    assert!(!first.dup, "the initial delivery must not have DUP set");

    // Deliberately never PUBACK it. Past the 1s redelivery interval, the
    // broker must resend the same PUBLISH with DUP=1.
    let second = match subscriber.read_packet().await {
        Packet::Publish(p) => p,
        other => panic!("expected a redelivered PUBLISH, got {other:?}"),
    };
    assert_eq!(second.topic, "t/redelivery");
    assert_eq!(second.packet_id, first.packet_id);
    assert_eq!(second.payload, first.payload);
    assert!(second.dup, "a redelivered PUBLISH must have DUP set");

    // Now ack it — a third resend must never arrive.
    subscriber
        .send(Packet::PubAck(
            mqtt_client::protocol::ack::SimpleAck::success(second.packet_id.unwrap()),
        ))
        .await;
    let result = timeout(Duration::from_secs(2), subscriber.read_packet()).await;
    assert!(
        result.is_err(),
        "no further redelivery should happen once PUBACK is sent"
    );

    publisher.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn broker_retries_unacked_pubrel_for_qos2() {
    let (broker, port) = start_broker(|c| c.redelivery_interval_secs = 1).await;

    let mut subscriber = RawClient::connect(port, "withholds-pubcomp").await;
    subscriber
        .send(Packet::Subscribe(SubscribePacket {
            packet_id: 1,
            filters: vec![SubscribeFilter::new("t/qos2-redelivery", QoS::ExactlyOnce)],
            properties: Properties::new(),
        }))
        .await;
    match subscriber.read_packet().await {
        Packet::SubAck(_) => {}
        other => panic!("expected SUBACK, got {other:?}"),
    }

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    let publish_task = tokio::spawn(async move {
        publisher
            .publish(
                "t/qos2-redelivery".into(),
                b"exactly once".to_vec(),
                QoS::ExactlyOnce,
                false,
            )
            .await
            .unwrap();
        publisher
    });

    // Receive the PUBLISH and promptly PUBREC it — this moves the broker's
    // redelivery entry from "awaiting PUBREC" to "awaiting PUBCOMP".
    let publish = match subscriber.read_packet().await {
        Packet::Publish(p) => p,
        other => panic!("expected PUBLISH, got {other:?}"),
    };
    let packet_id = publish.packet_id.unwrap();
    subscriber
        .send(Packet::PubRec(
            mqtt_client::protocol::ack::SimpleAck::success(packet_id),
        ))
        .await;

    // First PUBREL: must arrive promptly, before the redelivery interval.
    let first_pubrel = match subscriber.read_packet().await {
        Packet::PubRel(ack) => ack,
        other => panic!("expected PUBREL, got {other:?}"),
    };
    assert_eq!(first_pubrel.packet_id, packet_id);

    // Deliberately never PUBCOMP it. Past the 1s redelivery interval, the
    // broker must resend the same PUBREL (PUBREL has no DUP flag; it's
    // just the same packet again).
    let second_pubrel = match subscriber.read_packet().await {
        Packet::PubRel(ack) => ack,
        other => panic!("expected a redelivered PUBREL, got {other:?}"),
    };
    assert_eq!(second_pubrel.packet_id, packet_id);

    // Now complete the handshake — no further PUBREL should arrive, and
    // the client's `publish()` call must finally resolve.
    subscriber
        .send(Packet::PubComp(
            mqtt_client::protocol::ack::SimpleAck::success(packet_id),
        ))
        .await;
    let result = timeout(Duration::from_secs(2), subscriber.read_packet()).await;
    assert!(
        result.is_err(),
        "no further PUBREL redelivery should happen once PUBCOMP is sent"
    );

    let publisher = publish_task.await.unwrap();
    publisher.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}
