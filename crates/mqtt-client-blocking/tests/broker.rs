//! The blocking client against a real `stem-mqtt-broker` on a loopback port: no mocks.

#![allow(clippy::unwrap_used)] // test harness

use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mqtt_broker::{MqttAuthProvider, MqttBroker, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion as AsyncVersion, QoS as AsyncQoS};
use mqtt_client_blocking::{Client, Error, Options, QoS, WillMessage};

async fn start_broker() -> (MqttBroker, u16) {
    let broker = MqttBroker::new(MqttBrokerConfig::new("127.0.0.1", 0));
    broker.start().await.unwrap();
    let port = broker.bound_port().unwrap();
    (broker, port)
}

fn connect(port: u16, opts: Options) -> Result<Client<TcpStream>, Error> {
    Client::connect(TcpStream::connect(("127.0.0.1", port)).unwrap(), opts)
}

/// Poll until a message arrives (or fail after `secs`).
fn next_message(c: &mut Client<TcpStream>, secs: u64) -> mqtt_client_blocking::Message {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(m) = c.poll(Duration::from_millis(100)).unwrap() {
            return m;
        }
    }
    panic!("no message within {secs} s");
}

/// Run blocking client code on a blocking thread so the broker keeps running on the runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_and_subscribe_at_qos0_and_qos1() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        let mut sub = connect(port, Options::new("sub")).unwrap();
        sub.subscribe("home/+/temp", QoS::AtLeastOnce).unwrap();
        let mut publ = connect(port, Options::new("pub")).unwrap();
        publ.publish("home/kitchen/temp", b"21.5", QoS::AtMostOnce, false)
            .unwrap();
        publ.publish("home/hall/temp", b"19.0", QoS::AtLeastOnce, false)
            .unwrap();
        publ.publish("home/hall/humidity", b"40", QoS::AtLeastOnce, false)
            .unwrap(); // not matched

        let a = next_message(&mut sub, 5);
        let b = next_message(&mut sub, 5);
        assert_eq!(
            (a.topic.as_str(), a.payload.as_slice()),
            ("home/kitchen/temp", &b"21.5"[..])
        );
        assert_eq!(
            (b.topic.as_str(), b.payload.as_slice()),
            ("home/hall/temp", &b"19.0"[..])
        );
        assert!(
            sub.poll(Duration::from_millis(300)).unwrap().is_none(),
            "the humidity topic is not subscribed"
        );
        assert_eq!(sub.stats().messages_rx, 2);
        assert_eq!(publ.stats().messages_tx, 3);
        assert!(
            publ.stats().bytes_tx > 0 && publ.stats().packets_rx >= 3,
            "CONNACK and two PUBACKs came back"
        );
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retained_message_reaches_a_late_subscriber() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        let mut publ = connect(port, Options::new("pub")).unwrap();
        publ.publish("dev/c6/status", b"online", QoS::AtLeastOnce, true)
            .unwrap();
        let mut late = connect(port, Options::new("late")).unwrap();
        late.subscribe("dev/#", QoS::AtMostOnce).unwrap();
        let m = next_message(&mut late, 5);
        assert_eq!(
            (m.topic.as_str(), m.payload.as_slice(), m.retain),
            ("dev/c6/status", &b"online"[..], true)
        );
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_will_is_published_when_the_connection_drops_without_a_goodbye() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        let mut watcher = connect(port, Options::new("watcher")).unwrap();
        watcher
            .subscribe("dev/c6/status", QoS::AtLeastOnce)
            .unwrap();
        let dying = connect(
            port,
            Options::new("c6").will(WillMessage {
                topic: "dev/c6/status".into(),
                payload: b"offline".to_vec(),
                qos: QoS::AtLeastOnce,
                retain: true,
            }),
        )
        .unwrap();
        drop(dying); // no DISCONNECT: the socket just closes
        let m = next_message(&mut watcher, 5);
        assert_eq!(
            (m.topic.as_str(), m.payload.as_slice()),
            ("dev/c6/status", &b"offline"[..])
        );
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_disconnect_sends_no_will() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        let mut watcher = connect(port, Options::new("watcher")).unwrap();
        watcher.subscribe("dev/+/status", QoS::AtMostOnce).unwrap();
        let polite = connect(
            port,
            Options::new("c6").will(WillMessage {
                topic: "dev/c6/status".into(),
                payload: b"offline".to_vec(),
                qos: QoS::AtMostOnce,
                retain: false,
            }),
        )
        .unwrap();
        polite.disconnect().unwrap();
        assert!(watcher.poll(Duration::from_millis(600)).unwrap().is_none());
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_async_client_and_a_blocking_client_talk_to_each_other() {
    let (broker, port) = start_broker().await;
    let sync = blocking(move || {
        let mut c = connect(port, Options::new("blocking")).unwrap();
        c.subscribe("x/#", QoS::AtLeastOnce).unwrap();
        c
    })
    .await;

    let asyncc = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        port,
        "async",
        AsyncVersion::V5,
    ));
    asyncc.connect().await.unwrap();
    asyncc
        .publish(
            "x/from-async".into(),
            b"hello blocking".to_vec(),
            AsyncQoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();

    let sync = blocking(move || {
        let mut c = sync;
        let m = next_message(&mut c, 5);
        assert_eq!(
            (m.topic.as_str(), m.payload.as_slice()),
            ("x/from-async", &b"hello blocking"[..])
        );
        c.publish("y/from-blocking", b"hi", QoS::AtLeastOnce, false)
            .unwrap();
        c
    })
    .await;
    drop(sync);
    asyncc.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

struct OnlyAlice;
impl MqttAuthProvider for OnlyAlice {
    fn authenticate(
        &self,
        _client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
    ) -> bool {
        username.as_deref() == Some("alice") && password.as_deref() == Some(b"secret".as_slice())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_are_checked_by_the_broker() {
    let broker = MqttBroker::new(MqttBrokerConfig::new("127.0.0.1", 0));
    broker.set_auth_provider(Arc::new(OnlyAlice));
    broker.start().await.unwrap();
    let port = broker.bound_port().unwrap();
    blocking(move || {
        assert!(
            matches!(connect(port, Options::new("a")), Err(Error::Refused(_))),
            "no credentials"
        );
        assert!(matches!(
            connect(port, Options::new("b").credentials("alice", "wrong")),
            Err(Error::Refused(_))
        ));
        connect(port, Options::new("c").credentials("alice", "secret")).unwrap();
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_keep_alive_keeps_an_idle_connection_open() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        // keep-alive 1 s: the broker drops a silent client after 1.5 s. We ping every 150 ms.
        let opts = Options {
            keep_alive: Duration::from_secs(1),
            ping_interval: Some(Duration::from_millis(150)),
            ..Options::new("idle")
        };
        let mut c = connect(port, opts).unwrap();
        let until = Instant::now() + Duration::from_millis(2600);
        while Instant::now() < until {
            c.poll(Duration::from_millis(100)).unwrap();
        }
        assert!(
            c.stats().packets_rx >= 3,
            "PINGRESPs came back: {:?}",
            c.stats()
        );
        // still usable after being idle for longer than the broker's limit
        c.publish("still/alive", b"1", QoS::AtLeastOnce, false)
            .unwrap();
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_payload_survives_the_round_trip() {
    let (broker, port) = start_broker().await;
    blocking(move || {
        let mut sub = connect(port, Options::new("sub")).unwrap();
        sub.subscribe("big", QoS::AtLeastOnce).unwrap();
        let mut publ = connect(port, Options::new("pub")).unwrap();
        let payload: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        publ.publish("big", &payload, QoS::AtLeastOnce, false)
            .unwrap();
        let m = next_message(&mut sub, 10);
        assert_eq!(m.payload, payload);
    })
    .await;
    broker.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_broker_going_away_is_an_error_not_a_hang() {
    let (broker, port) = start_broker().await;
    let mut c = blocking(move || connect(port, Options::new("victim")).unwrap()).await;
    broker.stop().await.unwrap();
    let result = blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Err(e) = c.poll(Duration::from_millis(100)) {
                return Some(e);
            }
        }
        None
    })
    .await;
    assert!(
        matches!(result, Some(Error::Closed | Error::Io(_))),
        "got {result:?}"
    );
}
