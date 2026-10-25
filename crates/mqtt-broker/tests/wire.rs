//! Wire-level broker tests: a raw TCP/WebSocket peer speaks (sometimes
//! deliberately wrong) MQTT at a real broker, covering protocol-violation,
//! reserved-topic, retain-handling, will and event paths that a well-behaved
//! `MqttClient` never triggers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use mqtt_broker::{
    EnhancedAuthOutcome, EnhancedAuthStep, MqttBroker, MqttBrokerConfig, MqttBrokerEventListener,
    MqttEnhancedAuthProvider,
};
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::{ConnectPacket, Will};
use mqtt_client::protocol::packet::{AuthPacket, Packet};
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::subscribe::{
    RetainHandling, SubscribeFilter, SubscribePacket, UnsubscribePacket,
};
use mqtt_client::{MqttVersion, QoS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const V: MqttVersion = MqttVersion::V5;

async fn start(config_fn: impl FnOnce(&mut MqttBrokerConfig)) -> (MqttBroker, u16) {
    let mut c = MqttBrokerConfig::new("127.0.0.1", 0);
    config_fn(&mut c);
    let b = MqttBroker::new(c);
    b.start().await.unwrap();
    let port = b.bound_port().unwrap();
    (b, port)
}

struct Raw {
    s: TcpStream,
    buf: BytesMut,
}

impl Raw {
    async fn open(port: u16) -> Raw {
        Raw {
            s: TcpStream::connect(("127.0.0.1", port)).await.unwrap(),
            buf: BytesMut::new(),
        }
    }

    async fn send(&mut self, p: Packet) {
        self.s.write_all(&p.encode(V).unwrap()).await.unwrap();
    }

    /// Next packet, or `None` when the broker closed the connection.
    async fn recv(&mut self) -> Option<Packet> {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(Some(p)) = Packet::decode(&mut self.buf, V) {
                    return Some(p);
                }
                match self.s.read_buf(&mut self.buf).await {
                    Ok(0) | Err(_) => return None,
                    Ok(_) => {}
                }
            }
        })
        .await
        .expect("timed out waiting for broker")
    }

    async fn connect(port: u16, id: &str) -> Raw {
        Self::connect_with(port, id, None).await
    }

    async fn connect_with(port: u16, id: &str, will: Option<Will>) -> Raw {
        let mut r = Raw::open(port).await;
        r.send(Packet::Connect(connect_packet(id, will))).await;
        assert!(matches!(r.recv().await, Some(Packet::ConnAck(_))));
        r
    }
}

fn connect_packet(id: &str, will: Option<Will>) -> ConnectPacket {
    ConnectPacket {
        version: V,
        client_id: id.into(),
        clean_start: true,
        keep_alive: 30,
        username: None,
        password: None,
        will,
        properties: Properties::new(),
    }
}

fn publish(topic: &str, qos: QoS, id: Option<u16>, retain: bool) -> Packet {
    Packet::Publish(PublishPacket {
        dup: false,
        qos,
        retain,
        topic: topic.into(),
        packet_id: id,
        payload: Bytes::from_static(b"x"),
        properties: Properties::new(),
    })
}

fn subscribe(id: u16, filters: Vec<SubscribeFilter>) -> Packet {
    Packet::Subscribe(SubscribePacket {
        packet_id: id,
        filters,
        properties: Properties::new(),
    })
}

async fn assert_closed(r: &mut Raw) {
    assert!(r.recv().await.is_none(), "broker must close the connection");
}

#[tokio::test]
async fn pingreq_is_answered_with_pingresp() {
    let (b, port) = start(|_| {}).await;
    let mut r = Raw::connect(port, "ping").await;
    r.send(Packet::PingReq).await;
    assert!(matches!(r.recv().await, Some(Packet::PingResp)));
    b.stop().await.unwrap();
}

#[tokio::test]
async fn first_packet_must_be_connect() {
    let (b, port) = start(|_| {}).await;
    let mut r = Raw::open(port).await;
    r.send(Packet::PingReq).await;
    assert_closed(&mut r).await;
    b.stop().await.unwrap();
}

#[tokio::test]
async fn server_only_packet_from_a_client_is_a_protocol_error() {
    let (b, port) = start(|_| {}).await;
    let mut r = Raw::connect(port, "bad").await;
    r.send(Packet::PingResp).await;
    assert_closed(&mut r).await;
    b.stop().await.unwrap();
}

#[tokio::test]
async fn reserved_dollar_topics_are_acked_but_never_delivered() {
    let (b, port) = start(|_| {}).await;
    let mut sub = Raw::connect(port, "sub").await;
    sub.send(subscribe(
        1,
        vec![SubscribeFilter::new("#", QoS::AtLeastOnce)],
    ))
    .await;
    assert!(matches!(sub.recv().await, Some(Packet::SubAck(_))));

    let mut p = Raw::connect(port, "pub").await;
    p.send(publish("$SYS/x", QoS::AtMostOnce, None, false))
        .await;
    p.send(publish("$SYS/x", QoS::AtLeastOnce, Some(7), false))
        .await;
    assert!(matches!(p.recv().await, Some(Packet::PubAck(a)) if a.packet_id == 7));
    p.send(publish("$SYS/x", QoS::ExactlyOnce, Some(8), false))
        .await;
    assert!(matches!(p.recv().await, Some(Packet::PubRec(a)) if a.packet_id == 8));

    // Nothing reached the subscriber.
    assert!(timeout(Duration::from_millis(300), sub.recv())
        .await
        .is_err());
    b.stop().await.unwrap();
}

#[tokio::test]
async fn qos2_publish_is_acknowledged_with_pubrec() {
    let (b, port) = start(|_| {}).await;
    let mut p = Raw::connect(port, "q2").await;
    p.send(publish("a/b", QoS::ExactlyOnce, Some(3), false))
        .await;
    assert!(matches!(p.recv().await, Some(Packet::PubRec(a)) if a.packet_id == 3));
    p.send(Packet::PubRel(SimpleAck::success(3))).await;
    assert!(matches!(p.recv().await, Some(Packet::PubComp(_))));
    b.stop().await.unwrap();
}

#[tokio::test]
async fn subscription_limit_is_reported_as_quota_exceeded() {
    let (b, port) = start(|_| {}).await;
    let mut r = Raw::connect(port, "many").await;
    let filters = (0..=10_000)
        .map(|i| SubscribeFilter::new(format!("t/{i}"), QoS::AtMostOnce))
        .collect();
    r.send(subscribe(1, filters)).await;
    let Some(Packet::SubAck(ack)) = r.recv().await else {
        panic!("expected SUBACK");
    };
    assert_eq!(ack.reason_codes.len(), 10_001);
    assert_eq!(ack.reason_codes.last().unwrap().0, 0x97);
    b.stop().await.unwrap();
}

#[tokio::test]
async fn retain_handling_controls_retained_delivery() {
    let (b, port) = start(|_| {}).await;
    let mut p = Raw::connect(port, "retainer").await;
    p.send(publish("r/1", QoS::AtLeastOnce, Some(1), true))
        .await;
    assert!(matches!(p.recv().await, Some(Packet::PubAck(_))));

    let mut s = Raw::connect(port, "rh").await;
    let mut f = SubscribeFilter::new("r/1", QoS::AtMostOnce);

    // DoNotSend: SUBACK only.
    f.retain_handling = RetainHandling::DoNotSend;
    s.send(subscribe(1, vec![f.clone()])).await;
    assert!(matches!(s.recv().await, Some(Packet::SubAck(_))));
    assert!(timeout(Duration::from_millis(300), s.recv()).await.is_err());

    // SendIfNewSubscription on an existing subscription: still nothing.
    f.retain_handling = RetainHandling::SendIfNewSubscription;
    s.send(subscribe(2, vec![f.clone()])).await;
    assert!(matches!(s.recv().await, Some(Packet::SubAck(_))));
    assert!(timeout(Duration::from_millis(300), s.recv()).await.is_err());

    // SendAtSubscribe: retained message delivered.
    f.retain_handling = RetainHandling::SendAtSubscribe;
    s.send(subscribe(3, vec![f])).await;
    assert!(matches!(s.recv().await, Some(Packet::SubAck(_))));
    assert!(matches!(s.recv().await, Some(Packet::Publish(_))));
    b.stop().await.unwrap();
}

#[tokio::test]
async fn retained_will_is_published_on_ungraceful_disconnect() {
    let (b, port) = start(|_| {}).await;
    let will = Will {
        topic: "lwt/1".into(),
        payload: Bytes::from_static(b"gone"),
        qos: QoS::AtMostOnce,
        retain: true,
        properties: Properties::new(),
        delay_interval: 0,
    };
    let dying = Raw::connect_with(port, "dying", Some(will)).await;
    drop(dying); // no DISCONNECT => will fires

    let mut s = Raw::connect(port, "watcher").await;
    for _ in 0..50 {
        s.send(subscribe(
            1,
            vec![SubscribeFilter::new("lwt/1", QoS::AtMostOnce)],
        ))
        .await;
        assert!(matches!(s.recv().await, Some(Packet::SubAck(_))));
        if let Ok(Some(Packet::Publish(p))) = timeout(Duration::from_millis(100), s.recv()).await {
            assert_eq!(&p.payload[..], b"gone");
            assert!(b.is_running());
            b.stop().await.unwrap();
            return;
        }
        s.send(Packet::Unsubscribe(UnsubscribePacket {
            packet_id: 2,
            topic_filters: vec!["lwt/1".into()],
            properties: Properties::new(),
        }))
        .await;
        assert!(matches!(s.recv().await, Some(Packet::UnsubAck(_))));
    }
    panic!("retained will never became visible");
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);

impl MqttBrokerEventListener for Events {
    fn on_client_connected(&self, id: String) {
        self.0.lock().unwrap().push(format!("connected:{id}"));
    }
    fn on_client_disconnected(&self, id: String, _reason: String) {
        self.0.lock().unwrap().push(format!("disconnected:{id}"));
    }
    fn on_message_published(&self, id: String, topic: String, _qos: mqtt_broker::QoS) {
        self.0
            .lock()
            .unwrap()
            .push(format!("published:{id}:{topic}"));
    }
}

#[tokio::test]
async fn event_listener_sees_connect_publish_and_disconnect() {
    let (b, port) = start(|_| {}).await;
    let events = Arc::new(Events::default());
    b.set_event_listener(events.clone());
    let mut r = Raw::connect(port, "ev").await;
    r.send(publish("e/1", QoS::AtMostOnce, None, false)).await;
    r.send(Packet::Disconnect(Default::default())).await;
    drop(r);
    timeout(Duration::from_secs(3), async {
        loop {
            if events.0.lock().unwrap().len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("events");
    let got = events.0.lock().unwrap().clone();
    assert!(got.contains(&"connected:ev".to_string()), "{got:?}");
    assert!(got.contains(&"published:ev:e/1".to_string()), "{got:?}");
    assert!(got.contains(&"disconnected:ev".to_string()), "{got:?}");
    b.stop().await.unwrap();
}

// ── Enhanced auth edge cases ────────────────────────────────────────────────

struct Forever;
impl MqttEnhancedAuthProvider for Forever {
    fn step(&self, _: String, _: String, _: Option<Vec<u8>>, _: u32) -> EnhancedAuthStep {
        EnhancedAuthStep {
            outcome: EnhancedAuthOutcome::Continue,
            data: b"again".to_vec(),
        }
    }
}

async fn auth_connect(port: u16) -> Raw {
    let mut r = Raw::open(port).await;
    let mut c = connect_packet("auth", None);
    c.properties = Properties::with_auth("X", None);
    r.send(Packet::Connect(c)).await;
    assert!(matches!(r.recv().await, Some(Packet::Auth(_))));
    r
}

#[tokio::test]
async fn non_auth_reply_to_a_challenge_is_malformed() {
    let (b, port) = start(|_| {}).await;
    b.set_enhanced_auth_provider(Arc::new(Forever));
    let mut r = auth_connect(port).await;
    r.send(Packet::PingReq).await;
    match r.recv().await {
        Some(Packet::ConnAck(a)) => assert!(!a.reason_code.is_success()),
        None => {}
        other => panic!("unexpected {other:?}"),
    }
    b.stop().await.unwrap();
}

#[tokio::test]
async fn endless_challenges_never_converge_and_are_refused() {
    let (b, port) = start(|_| {}).await;
    b.set_enhanced_auth_provider(Arc::new(Forever));
    let mut r = auth_connect(port).await;
    let mut refused = false;
    for _ in 0..200 {
        r.send(Packet::Auth(AuthPacket::continue_with(
            "X",
            Bytes::from("r"),
        )))
        .await;
        match r.recv().await {
            Some(Packet::Auth(_)) => {}
            Some(Packet::ConnAck(a)) => {
                assert!(!a.reason_code.is_success());
                refused = true;
                break;
            }
            None => {
                refused = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(refused, "broker must cut off a non-converging exchange");
    b.stop().await.unwrap();
}

// ── WebSocket handshake failures ────────────────────────────────────────────

#[tokio::test]
async fn websocket_without_mqtt_subprotocol_is_rejected() {
    let mut c = MqttBrokerConfig::new("127.0.0.1", 0);
    c.ws_port = Some(0);
    let b = MqttBroker::new(c);
    b.start().await.unwrap();
    let ws = b.bound_ws_port().unwrap();
    let r = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{ws}/")).await;
    assert!(
        r.is_err(),
        "handshake lacking the mqtt subprotocol must fail"
    );
    b.stop().await.unwrap();
}

#[tokio::test]
async fn garbage_instead_of_a_websocket_handshake_is_dropped() {
    let mut c = MqttBrokerConfig::new("127.0.0.1", 0);
    c.ws_port = Some(0);
    let b = MqttBroker::new(c);
    b.start().await.unwrap();
    let ws = b.bound_ws_port().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", ws)).await.unwrap();
    s.write_all(b"this is not http\r\n\r\n").await.unwrap();
    let mut sink = Vec::new();
    let _ = timeout(Duration::from_secs(3), s.read_to_end(&mut sink))
        .await
        .expect("broker must close a bad handshake");
    // Broker still serves real clients afterwards.
    let (mut w, _) = tokio_tungstenite::connect_async(
        tokio_tungstenite::tungstenite::handshake::client::Request::builder()
            .uri(format!("ws://127.0.0.1:{ws}/"))
            .header("Sec-WebSocket-Protocol", "mqtt")
            .header("Host", format!("127.0.0.1:{ws}"))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .unwrap(),
    )
    .await
    .unwrap();
    w.close(None).await.ok();
    let _ = w.next().await;
    b.stop().await.unwrap();
}
