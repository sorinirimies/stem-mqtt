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

// ── Robustness regressions ─────────────────────────────────────────────

/// A second CONNECT with the same client id evicts the first connection.
/// The evicted connection's teardown must neither disturb its replacement
/// nor publish the evicted connection's Last Will.
#[tokio::test]
async fn session_takeover_leaves_replacement_connected_and_publishes_no_will() {
    let (broker, port) = start_broker(|_| {}).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = client(port, "observer");
    observer.set_message_listener(Arc::new(ChannelListener { tx }));
    observer.connect().await.unwrap();
    observer
        .subscribe("t/#".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    let mut opts = ConnectOptions::new("127.0.0.1", port, "same-id", MqttVersion::V5);
    opts.will = Some(mqtt_client::WillOptions {
        topic: "t/will".into(),
        payload: b"should-not-fire".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: false,
    });
    let first = MqttClient::new(opts.clone());
    first.connect().await.unwrap();

    let second = MqttClient::new(opts);
    second.connect().await.unwrap();

    // Give the evicted connection time to tear itself down.
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        broker.client_count(),
        2,
        "observer + replacement must both still be counted"
    );
    second
        .publish(
            "t/alive".into(),
            b"still here".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await
        .expect("replacement connection must remain fully usable");
    let msg = recv_message(&mut rx).await;
    assert_eq!(msg.topic, "t/alive", "only the live publish, never a will");
    assert_no_message(&mut rx).await;

    second.disconnect().await.unwrap();
    observer.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn oversized_packet_disconnects_only_the_offender() {
    let (broker, port) = start_broker(|c| c.max_packet_size = 1024).await;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let bystander = client(port, "bystander");
    bystander.set_message_listener(Arc::new(ChannelListener { tx }));
    bystander.connect().await.unwrap();
    bystander
        .subscribe("t/ok".into(), QoS::AtMostOnce)
        .await
        .unwrap();

    let offender = client(port, "offender");
    offender.connect().await.unwrap();
    // QoS 0 so there's no ack to wait on: the broker just drops us.
    let _ = offender
        .publish("t/big".into(), vec![0u8; 8 * 1024], QoS::AtMostOnce, false)
        .await;
    timeout(Duration::from_secs(5), async {
        while offender.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("broker must drop a client that exceeds max_packet_size");

    // Small messages from others still flow.
    let ok = client(port, "ok");
    ok.connect().await.unwrap();
    ok.publish("t/ok".into(), b"small".to_vec(), QoS::AtMostOnce, false)
        .await
        .unwrap();
    assert_eq!(recv_message(&mut rx).await.payload, b"small");

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_the_broker_disconnects_clients() {
    let (broker, port) = start_broker(|_| {}).await;
    let c = client(port, "will-be-kicked");
    c.connect().await.unwrap();
    assert!(c.is_connected());

    broker.stop().await.unwrap();
    assert!(!broker.is_running());

    timeout(Duration::from_secs(5), async {
        while c.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("stop() must close live client connections");
}

#[tokio::test]
async fn start_is_all_or_nothing_when_a_listener_cannot_bind() {
    let blocker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let taken = blocker.local_addr().unwrap().port();

    let mut config = MqttBrokerConfig::new("127.0.0.1", 0);
    config.ws_port = Some(taken);
    let broker = MqttBroker::new(config);
    assert!(broker.start().await.is_err(), "ws port is already in use");
    assert!(
        !broker.is_running(),
        "a failed start must not leave a half-running broker"
    );
    assert_eq!(broker.bound_port(), None);

    // Once the port is free, the very same broker starts fine.
    drop(blocker);
    broker
        .start()
        .await
        .expect("restart after fixing the conflict");
    assert!(broker.is_running());
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn concurrent_start_calls_do_not_double_start() {
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::new("127.0.0.1", 0)));
    let results = futures_util::future::join_all((0..4).map(|_| {
        let broker = broker.clone();
        async move { broker.start().await }
    }))
    .await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn empty_client_id_with_persistent_session_is_refused() {
    let (broker, port) = start_broker(|_| {}).await;
    let mut opts = ConnectOptions::new("127.0.0.1", port, "", MqttVersion::V311);
    opts.clean_start = false;
    let err = MqttClient::new(opts).connect().await.unwrap_err();
    assert!(
        matches!(err, mqtt_client::MqttError::ConnectionRefused(_)),
        "{err:?}"
    );

    // ...while an empty id with a clean session gets one assigned.
    let mut opts = ConnectOptions::new("127.0.0.1", port, "", MqttVersion::V311);
    opts.clean_start = true;
    let ok = MqttClient::new(opts);
    ok.connect().await.expect("clean + empty id is allowed");
    ok.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn client_rejects_invalid_topics_locally() {
    let (broker, port) = start_broker(|_| {}).await;
    let c = client(port, "validator");
    c.connect().await.unwrap();

    for bad in ["", "a/+/b", "a/#"] {
        let err = c
            .publish(bad.into(), vec![], QoS::AtMostOnce, false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, mqtt_client::MqttError::Protocol(_)),
            "{bad:?}: {err:?}"
        );
    }
    assert!(c.subscribe("a/#/b".into(), QoS::AtMostOnce).await.is_err());
    assert!(
        c.is_connected(),
        "local validation must not cost us the connection"
    );

    c.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn unsubscribe_reports_unknown_filter_on_v5() {
    let (broker, port) = start_broker(|_| {}).await;
    let c = client(port, "unsub");
    c.connect().await.unwrap();
    // Not an error: UNSUBACK (reason 0x11) still completes the exchange.
    c.unsubscribe("never/subscribed".into()).await.unwrap();
    c.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

/// `$share/<group>/<filter>`: each message goes to exactly ONE member of the
/// group (round-robin), while ordinary subscribers still get every message.
#[tokio::test]
async fn shared_subscription_delivers_each_message_to_one_group_member() {
    let (broker, port) = start_broker(|_| {}).await;

    let mut inboxes = Vec::new();
    let mut members = Vec::new();
    for name in ["worker-a", "worker-b"] {
        let (tx, rx) = mpsc::unbounded_channel();
        let c = client(port, name);
        c.set_message_listener(Arc::new(ChannelListener { tx }));
        c.connect().await.unwrap();
        let granted = c
            .subscribe("$share/workers/jobs/#".into(), QoS::AtLeastOnce)
            .await
            .unwrap();
        assert!(
            granted.reason_code < 0x80,
            "shared subscribe must be granted"
        );
        inboxes.push(rx);
        members.push(c);
    }
    let (tx, mut observer_rx) = mpsc::unbounded_channel();
    let observer = client(port, "observer");
    observer.set_message_listener(Arc::new(ChannelListener { tx }));
    observer.connect().await.unwrap();
    observer
        .subscribe("jobs/#".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    let publisher = client(port, "dispatcher");
    publisher.connect().await.unwrap();
    for i in 0..6 {
        publisher
            .publish(format!("jobs/{i}"), vec![i as u8], QoS::AtLeastOnce, false)
            .await
            .unwrap();
    }

    for _ in 0..6 {
        recv_message(&mut observer_rx).await; // the plain subscriber sees all 6
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let counts: Vec<usize> = inboxes
        .iter_mut()
        .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).count())
        .collect();
    assert_eq!(
        counts.iter().sum::<usize>(),
        6,
        "no duplicates, no losses: {counts:?}"
    );
    assert!(
        counts.iter().all(|&c| c > 0),
        "load is spread across members: {counts:?}"
    );

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn malformed_shared_subscription_is_rejected() {
    let (broker, port) = start_broker(|_| {}).await;
    let c = client(port, "shared-bad");
    c.connect().await.unwrap();
    for bad in ["$share/onlygroup", "$share//a", "$share/g/"] {
        let result = c.subscribe(bad.into(), QoS::AtMostOnce).await.unwrap();
        assert!(result.reason_code >= 0x80, "{bad:?} must be refused");
    }
    c.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

/// Persistent sessions that stay offline past `session_expiry_secs` are
/// discarded, so clients that never return can't leak memory forever.
#[tokio::test]
async fn offline_sessions_expire() {
    let (broker, port) = start_broker(|c| {
        c.session_expiry_secs = 1;
        c.redelivery_interval_secs = 1; // the sweep runs on this tick
    })
    .await;

    let mut opts = ConnectOptions::new("127.0.0.1", port, "forgetful", MqttVersion::V5);
    opts.clean_start = false;
    let first = MqttClient::new(opts.clone());
    first.connect().await.unwrap();
    first
        .subscribe("t/expire".into(), QoS::AtLeastOnce)
        .await
        .unwrap();
    first.disconnect().await.unwrap();

    // Still resumable right after disconnecting...
    let again = MqttClient::new(opts.clone());
    assert!(again.connect().await.unwrap().session_present);
    again.disconnect().await.unwrap();

    // ...but gone once the expiry has passed.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let later = MqttClient::new(opts);
    assert!(
        !later.connect().await.unwrap().session_present,
        "the expired session must not be resumed"
    );
    later.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

/// A panicking listener (an exception thrown by Kotlin/Swift/Python code
/// surfaces as a Rust panic) must not kill the connection's read loop.
#[tokio::test]
async fn panicking_listener_does_not_kill_the_connection() {
    struct PanicOnce(
        std::sync::atomic::AtomicBool,
        mpsc::UnboundedSender<MqttMessage>,
    );
    impl MqttMessageListener for PanicOnce {
        fn on_message(&self, message: MqttMessage) {
            if !self.0.swap(true, std::sync::atomic::Ordering::SeqCst) {
                panic!("foreign listener blew up");
            }
            let _ = self.1.send(message);
        }
        fn on_disconnected(&self, _reason: String) {}
    }

    let (broker, port) = start_broker(|_| {}).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sub = client(port, "fragile");
    sub.set_message_listener(Arc::new(PanicOnce(false.into(), tx)));
    sub.connect().await.unwrap();
    sub.subscribe("t/p".into(), QoS::AtMostOnce).await.unwrap();

    let publisher = client(port, "pub");
    publisher.connect().await.unwrap();
    for payload in [b"first".as_slice(), b"second".as_slice()] {
        publisher
            .publish("t/p".into(), payload.to_vec(), QoS::AtMostOnce, false)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(recv_message(&mut rx).await.payload, b"second");
    assert!(sub.is_connected());
    broker.stop().await.unwrap();
}

// ── Pull-style delivery (for runtimes without callbacks: Dart, Haskell) ──

#[tokio::test]
async fn polled_messages_and_events_work_without_any_callback() {
    use mqtt_broker::BrokerEvent;

    let (broker, port) = start_broker(|_| {}).await;
    broker.enable_event_queue(16);

    let sub = client(port, "poll-sub");
    sub.enable_message_queue(16); // no listener registered anywhere
    sub.connect().await.unwrap();
    sub.subscribe("poll/#".into(), QoS::AtLeastOnce)
        .await
        .unwrap();

    let publisher = client(port, "poll-pub");
    publisher.connect().await.unwrap();
    publisher
        .publish("poll/x".into(), b"pulled".to_vec(), QoS::AtLeastOnce, false)
        .await
        .unwrap();

    let msg = sub
        .next_message(5_000)
        .await
        .expect("a queued message must arrive");
    assert_eq!(msg.topic, "poll/x");
    assert_eq!(msg.payload, b"pulled");
    assert!(
        sub.next_message(100).await.is_none(),
        "queue is drained; a poll with nothing pending times out"
    );

    // The broker saw both connections and the publish, in order.
    let mut seen = Vec::new();
    while let Some(event) = broker.next_event(300).await {
        seen.push(event);
    }
    assert!(seen.contains(&BrokerEvent::ClientConnected {
        client_id: "poll-sub".into()
    }));
    assert!(seen.contains(&BrokerEvent::ClientConnected {
        client_id: "poll-pub".into()
    }));
    assert!(seen.iter().any(|e| matches!(
        e,
        BrokerEvent::MessagePublished { topic, .. } if topic == "poll/x"
    )));

    // A lost connection is observable by polling too.
    broker.stop().await.unwrap();
    timeout(Duration::from_secs(5), async {
        while sub.is_connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(sub.last_disconnect_reason().is_some());
}

// ── MQTT 5.0 enhanced authentication (challenge/response) ────────────────

mod enhanced_auth {
    use super::*;
    use mqtt_broker::{EnhancedAuthStep, MqttEnhancedAuthProvider};
    use mqtt_client::MqttAuthHandler;

    /// Round 0: challenge with [7]. Round 1: the response must be [8].
    struct AddOne;
    impl MqttEnhancedAuthProvider for AddOne {
        fn step(
            &self,
            _client_id: String,
            method: String,
            data: Option<Vec<u8>>,
            round: u32,
        ) -> EnhancedAuthStep {
            assert_eq!(method, "X-ADD-ONE");
            match (round, data.as_deref()) {
                (0, Some(b"hello")) => EnhancedAuthStep::proceed(vec![7]),
                (1, Some([8])) => EnhancedAuthStep::success(b"welcome".to_vec()),
                _ => EnhancedAuthStep::failure(),
            }
        }
    }

    struct Answer(Option<Vec<u8>>);
    impl MqttAuthHandler for Answer {
        fn respond(&self, _method: String, challenge: Vec<u8>) -> Option<Vec<u8>> {
            self.0
                .clone()
                .or_else(|| Some(challenge.iter().map(|b| b + 1).collect()))
        }
    }

    fn options(port: u16, id: &str) -> ConnectOptions {
        let mut o = ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5);
        o.auth_method = Some("X-ADD-ONE".into());
        o.auth_data = Some(b"hello".to_vec());
        o
    }

    #[tokio::test]
    async fn correct_response_authenticates() {
        let (broker, port) = start_broker(|c| c.allow_anonymous = false).await;
        broker.set_enhanced_auth_provider(Arc::new(AddOne));
        let c = MqttClient::new(options(port, "eauth-ok"));
        c.set_auth_handler(Arc::new(Answer(None)));
        c.connect()
            .await
            .expect("challenge/response should succeed");
        assert!(c.is_connected());
        // A full session works afterwards.
        c.subscribe("x".into(), QoS::AtMostOnce).await.unwrap();
        c.disconnect().await.unwrap();
        broker.stop().await.unwrap();
    }

    #[tokio::test]
    async fn wrong_response_is_refused() {
        let (broker, port) = start_broker(|_| {}).await;
        broker.set_enhanced_auth_provider(Arc::new(AddOne));
        let c = MqttClient::new(options(port, "eauth-bad"));
        c.set_auth_handler(Arc::new(Answer(Some(vec![99]))));
        let err = c.connect().await.unwrap_err();
        assert!(
            matches!(err, mqtt_client::MqttError::ConnectionRefused(_)),
            "{err:?}"
        );
        assert_eq!(broker.client_count(), 0);
        broker.stop().await.unwrap();
    }

    #[tokio::test]
    async fn method_without_a_provider_is_refused() {
        let (broker, port) = start_broker(|_| {}).await;
        let c = MqttClient::new(options(port, "eauth-none"));
        c.set_auth_handler(Arc::new(Answer(None)));
        let err = c.connect().await.unwrap_err();
        assert!(
            matches!(err, mqtt_client::MqttError::ConnectionRefused(_)),
            "{err:?}"
        );
        broker.stop().await.unwrap();
    }

    #[tokio::test]
    async fn challenge_without_a_client_handler_is_refused() {
        let (broker, port) = start_broker(|_| {}).await;
        broker.set_enhanced_auth_provider(Arc::new(AddOne));
        let c = MqttClient::new(options(port, "eauth-nohandler"));
        let err = c.connect().await.unwrap_err();
        assert!(
            matches!(&err, mqtt_client::MqttError::ConnectionRefused(m) if m.contains("MqttAuthHandler")),
            "{err:?}"
        );
        broker.stop().await.unwrap();
    }

    #[tokio::test]
    async fn auth_method_requires_mqtt5() {
        let mut o = ConnectOptions::new("127.0.0.1", 1883, "c", MqttVersion::V311);
        o.auth_method = Some("X".into());
        let err = MqttClient::new(o).connect().await.unwrap_err();
        assert!(matches!(err, mqtt_client::MqttError::Protocol(m) if m.contains("MQTT 5.0")));
    }
}
