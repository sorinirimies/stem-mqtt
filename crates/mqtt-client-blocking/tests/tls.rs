//! The blocking client over TLS (feature `rustls`) against a real `stem-mqtt-broker` TLS listener.

#![cfg(feature = "rustls")]
#![allow(clippy::unwrap_used)] // test harness

use std::time::{Duration, Instant};

use mqtt_broker::{BrokerTlsConfig, MqttBroker, MqttBrokerConfig};
use mqtt_client_blocking::tls::{self, ClientIdentity};
use mqtt_client_blocking::{Client, Options, QoS, WillMessage};

struct Pem {
    cert: Vec<u8>,
    key: Vec<u8>,
}

fn self_signed(names: &[&str]) -> Pem {
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(
        names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>(),
    )
    .unwrap();
    Pem {
        cert: cert.pem().into_bytes(),
        key: signing_key.serialize_pem().into_bytes(),
    }
}

async fn start_tls_broker(server: &Pem, client_ca: Option<Vec<u8>>) -> (MqttBroker, u16) {
    let config = MqttBrokerConfig {
        tls: Some(BrokerTlsConfig {
            port: 0,
            cert_pem: server.cert.clone(),
            key_pem: server.key.clone(),
            client_ca_pem: client_ca,
        }),
        ..MqttBrokerConfig::new("127.0.0.1", 0)
    };
    let broker = MqttBroker::new(config);
    broker.start().await.unwrap();
    let port = broker.bound_tls_port().unwrap();
    (broker, port)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_and_receive_over_tls_with_a_private_ca() {
    let server = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let ca = server.cert.clone();
    blocking(move || {
        let config = tls::client_config(Some(&ca), None).unwrap();
        let mut sub = Client::connect(
            tls::connect("127.0.0.1", port, config.clone()).unwrap(),
            Options::new("tls-sub"),
        )
        .unwrap();
        sub.subscribe("home/+/temp", QoS::AtLeastOnce).unwrap();
        let mut publ = Client::connect(
            tls::connect("127.0.0.1", port, config).unwrap(),
            Options::new("tls-pub"),
        )
        .unwrap();
        publ.publish("home/hall/temp", b"19.0", QoS::AtLeastOnce, false)
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // A short timeout exercises the read timeout on the TLS stream.
            if let Some(m) = sub.poll(Duration::from_millis(50)).unwrap() {
                assert_eq!(m.topic, "home/hall/temp");
                assert_eq!(m.payload, b"19.0");
                break;
            }
            assert!(Instant::now() < deadline, "no message over TLS");
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn large_payload_survives_tls_record_splitting() {
    let server = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let ca = server.cert.clone();
    blocking(move || {
        let config = tls::client_config(Some(&ca), None).unwrap();
        let connect = |id: &str| {
            Client::connect(
                tls::connect("127.0.0.1", port, config.clone()).unwrap(),
                Options::new(id),
            )
            .unwrap()
        };
        let mut sub = connect("big-sub");
        sub.subscribe("big", QoS::AtLeastOnce).unwrap();
        let payload = vec![0xAB; 150_000];
        connect("big-pub")
            .publish("big", &payload, QoS::AtLeastOnce, false)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(m) = sub.poll(Duration::from_millis(50)).unwrap() {
                assert_eq!(m.payload, payload);
                break;
            }
            assert!(Instant::now() < deadline, "no large message over TLS");
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_untrusted_certificate_is_rejected() {
    let server = self_signed(&["127.0.0.1"]);
    let stranger = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let err = blocking(move || {
        let config = tls::client_config(Some(&stranger.cert), None).unwrap();
        let stream = tls::connect("127.0.0.1", port, config).unwrap();
        Client::connect(stream, Options::new("untrusted")).err()
    })
    .await;
    assert!(
        err.is_some(),
        "a certificate from another CA must be refused"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_for_another_host_is_rejected() {
    let server = self_signed(&["other.example"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let ca = server.cert.clone();
    let err = blocking(move || {
        let config = tls::client_config(Some(&ca), None).unwrap();
        let stream = tls::connect("127.0.0.1", port, config).unwrap();
        Client::connect(stream, Options::new("wrong-host")).err()
    })
    .await;
    assert!(
        err.is_some(),
        "a certificate for another name must be refused"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mutual_tls_needs_the_client_certificate() {
    let server = self_signed(&["127.0.0.1"]);
    let client = self_signed(&["device"]);
    let (_broker, port) = start_tls_broker(&server, Some(client.cert.clone())).await;
    let ca = server.cert.clone();
    blocking(move || {
        // With the identity: accepted.
        let with = tls::client_config(
            Some(&ca),
            Some(ClientIdentity {
                cert_pem: &client.cert,
                key_pem: &client.key,
            }),
        )
        .unwrap();
        let stream = tls::connect("127.0.0.1", port, with).unwrap();
        let mut ok = Client::connect(stream, Options::new("device")).unwrap();
        ok.publish("t", b"x", QoS::AtLeastOnce, false).unwrap();

        // Without: the broker drops the handshake.
        let without = tls::client_config(Some(&ca), None).unwrap();
        let stream = tls::connect("127.0.0.1", port, without).unwrap();
        assert!(Client::connect(stream, Options::new("anon")).is_err());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ca_bundle_trusts_any_certificate_in_it() {
    let server = self_signed(&["127.0.0.1"]);
    let other = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let bundle = [other.cert.as_slice(), b"\n", server.cert.as_slice()].concat();
    blocking(move || {
        let config = tls::client_config(Some(&bundle), None).unwrap();
        let stream = tls::connect("127.0.0.1", port, config).unwrap();
        Client::connect(stream, Options::new("bundle")).unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_web_roots_refuse_a_private_certificate() {
    let server = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let err = blocking(move || {
        let config = tls::client_config(None, None).unwrap();
        let stream = tls::connect("127.0.0.1", port, config).unwrap();
        Client::connect(stream, Options::new("web-roots")).err()
    })
    .await;
    assert!(
        err.is_some(),
        "a private certificate is not in the web roots"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_alive_pings_work_over_an_idle_tls_connection() {
    let server = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let ca = server.cert.clone();
    blocking(move || {
        let config = tls::client_config(Some(&ca), None).unwrap();
        let mut opts = Options::new("idle");
        opts.keep_alive = Duration::from_secs(1);
        opts.ping_interval = Some(Duration::from_millis(300));
        let mut client =
            Client::connect(tls::connect("127.0.0.1", port, config).unwrap(), opts).unwrap();
        // Idle for several keep-alive periods: the broker would drop a silent client.
        let until = Instant::now() + Duration::from_millis(2500);
        while Instant::now() < until {
            client.poll(Duration::from_millis(100)).unwrap();
        }
        client
            .publish("idle/ok", b"1", QoS::AtLeastOnce, false)
            .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_will_is_sent_when_a_tls_client_drops() {
    let server = self_signed(&["127.0.0.1"]);
    let (_broker, port) = start_tls_broker(&server, None).await;
    let ca = server.cert.clone();
    blocking(move || {
        let config = tls::client_config(Some(&ca), None).unwrap();
        let connect = |opts: Options| {
            Client::connect(
                tls::connect("127.0.0.1", port, config.clone()).unwrap(),
                opts,
            )
            .unwrap()
        };
        let mut watcher = connect(Options::new("watcher"));
        watcher.subscribe("status/dev", QoS::AtLeastOnce).unwrap();
        let mut opts = Options::new("dev");
        opts.will = Some(WillMessage {
            topic: "status/dev".into(),
            payload: b"offline".to_vec(),
            qos: QoS::AtLeastOnce,
            retain: false,
        });
        drop(connect(opts)); // no DISCONNECT: the TLS stream is just closed

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(m) = watcher.poll(Duration::from_millis(100)).unwrap() {
                assert_eq!(m.payload, b"offline");
                break;
            }
            assert!(Instant::now() < deadline, "no will over TLS");
        }
    })
    .await;
}

#[test]
fn a_pem_without_certificates_is_invalid_data() {
    let err = tls::client_config(Some(b""), None).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn a_mismatched_or_missing_client_key_is_invalid_data() {
    let a = self_signed(&["device"]);
    let b = self_signed(&["device"]);
    let ca = self_signed(&["127.0.0.1"]).cert;
    let mismatched = ClientIdentity {
        cert_pem: &a.cert,
        key_pem: &b.key,
    };
    let err = tls::client_config(Some(&ca), Some(mismatched)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let no_key = ClientIdentity {
        cert_pem: &a.cert,
        key_pem: b"",
    };
    let err = tls::client_config(Some(&ca), Some(no_key)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn bad_pem_is_invalid_data() {
    let err = tls::client_config(Some(b"not a certificate"), None).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let id = ClientIdentity {
        cert_pem: b"",
        key_pem: b"-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----",
    };
    assert!(!format!("{id:?}").contains("secret"));
}

#[test]
fn an_invalid_server_name_is_invalid_data() {
    let config = tls::client_config(None, None).unwrap();
    let err = tls::connect("not a host!", 8883, config).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}
