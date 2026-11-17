//! Topic authorisation, certificate-bound client ids and the message observer, end to end: a real broker and real clients.

#![allow(clippy::unwrap_used)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mqtt_broker::{
    BrokerTlsConfig, CertClientId, MqttAuthorizer, MqttBroker, MqttBrokerConfig,
    MqttMessageObserver, ObservedMessage, TopicAction,
};
use mqtt_client::{
    ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS, TlsOptions,
    WillOptions,
};
use tokio::sync::mpsc;
use tokio::time::timeout;

struct Channel(mpsc::UnboundedSender<MqttMessage>);
impl MqttMessageListener for Channel {
    fn on_message(&self, message: MqttMessage) {
        let _ = self.0.send(message);
    }
    fn on_disconnected(&self, _reason: String) {}
}

async fn broker(config: impl FnOnce(&mut MqttBrokerConfig)) -> (MqttBroker, u16) {
    let mut c = MqttBrokerConfig::new("127.0.0.1", 0);
    config(&mut c);
    let b = MqttBroker::new(c);
    b.start().await.unwrap();
    let port = b.bound_port().unwrap();
    (b, port)
}

fn client(port: u16, id: &str) -> MqttClient {
    MqttClient::new(ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5))
}

async fn got(rx: &mut mpsc::UnboundedReceiver<MqttMessage>) -> MqttMessage {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a message")
        .expect("open")
}

async fn none(rx: &mut mpsc::UnboundedReceiver<MqttMessage>) {
    assert!(
        timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "expected nothing"
    );
}

/// A rig may use `tc/farm/rig/<its id>/…`; the hub (`hub`) may use everything; a Last Will on the rig's own status only.
struct Acl;
impl MqttAuthorizer for Acl {
    fn authorize(&self, client_id: String, _action: TopicAction, topic: String) -> bool {
        client_id == "hub" || topic.starts_with(&format!("tc/farm/rig/{client_id}/"))
    }
}

#[tokio::test]
async fn a_client_may_only_use_its_own_topics() {
    let (broker, port) = broker(|_| {}).await;
    broker.set_authorizer(Arc::new(Acl));

    let (tx, mut hub_rx) = mpsc::unbounded_channel();
    let hub = client(port, "hub");
    hub.set_message_listener(Arc::new(Channel(tx)));
    hub.connect().await.unwrap();
    assert_eq!(
        hub.subscribe("tc/farm/rig/#".into(), QoS::AtLeastOnce)
            .await
            .unwrap()
            .reason_code,
        0x01
    );

    let (tx, mut a_rx) = mpsc::unbounded_channel();
    let a = client(port, "a");
    a.set_message_listener(Arc::new(Channel(tx)));
    a.connect().await.unwrap();
    // its own command topic: granted; another rig's, and a wildcard that reaches it: refused (0x87)
    assert_eq!(
        a.subscribe("tc/farm/rig/a/cmd".into(), QoS::AtLeastOnce)
            .await
            .unwrap()
            .reason_code,
        0x01
    );
    assert_eq!(
        a.subscribe("tc/farm/rig/b/cmd".into(), QoS::AtLeastOnce)
            .await
            .unwrap()
            .reason_code,
        0x87
    );
    assert_eq!(
        a.subscribe("tc/farm/rig/#".into(), QoS::AtLeastOnce)
            .await
            .unwrap()
            .reason_code,
        0x87
    );

    // a publishes on its own topic: the hub sees it; on b's topic: nobody does
    a.publish(
        "tc/farm/rig/a/status".into(),
        b"up".to_vec(),
        QoS::AtMostOnce,
        false,
    )
    .await
    .unwrap();
    assert_eq!(got(&mut hub_rx).await.payload, b"up");
    a.publish(
        "tc/farm/rig/b/status".into(),
        b"spoof".to_vec(),
        QoS::AtMostOnce,
        false,
    )
    .await
    .unwrap();
    none(&mut hub_rx).await;
    // QoS 1: the denied publish is acknowledged (MQTT 5.0 carries 0x87), still nothing is delivered
    let _ = a
        .publish(
            "tc/farm/rig/b/status".into(),
            b"spoof".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await;
    none(&mut hub_rx).await;
    // nothing retained either
    a.publish(
        "tc/farm/rig/b/status".into(),
        b"spoof".to_vec(),
        QoS::AtMostOnce,
        true,
    )
    .await
    .unwrap();
    let (tx, mut late_rx) = mpsc::unbounded_channel();
    let late = client(port, "hub");
    drop(hub);
    late.set_message_listener(Arc::new(Channel(tx)));
    late.connect().await.unwrap();
    late.subscribe("tc/farm/rig/b/status".into(), QoS::AtMostOnce)
        .await
        .unwrap();
    none(&mut late_rx).await;

    // the hub commands a rig
    late.publish(
        "tc/farm/rig/a/cmd".into(),
        b"stop".to_vec(),
        QoS::AtLeastOnce,
        false,
    )
    .await
    .unwrap();
    assert_eq!(got(&mut a_rx).await.payload, b"stop");

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn a_last_will_needs_the_same_permission() {
    let (broker, port) = broker(|_| {}).await;
    broker.set_authorizer(Arc::new(Acl));
    let will = |topic: &str| WillOptions {
        topic: topic.into(),
        payload: b"gone".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: true,
    };
    let mut ok = ConnectOptions::new("127.0.0.1", port, "a", MqttVersion::V5);
    ok.will = Some(will("tc/farm/rig/a/status"));
    assert!(MqttClient::new(ok).connect().await.is_ok());
    let mut bad = ConnectOptions::new("127.0.0.1", port, "a2", MqttVersion::V5);
    bad.will = Some(will("tc/farm/rig/b/status"));
    assert!(
        MqttClient::new(bad).connect().await.is_err(),
        "a will on another rig's topic is refused"
    );
    broker.stop().await.unwrap();
}

struct Seen(Mutex<Vec<(String, ObservedMessage)>>);
impl MqttMessageObserver for Seen {
    fn on_message(&self, client_id: String, message: ObservedMessage) {
        self.0.lock().unwrap().push((client_id, message));
    }
}

#[tokio::test]
async fn the_observer_sees_payloads_and_the_last_will() {
    let (broker, port) = broker(|_| {}).await;
    let seen = Arc::new(Seen(Mutex::default()));
    broker.set_message_observer(seen.clone());
    let mut o = ConnectOptions::new("127.0.0.1", port, "rig", MqttVersion::V5);
    o.will = Some(WillOptions {
        topic: "tc/farm/rig/rig/status".into(),
        payload: b"gone".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: true,
    });
    let rig = MqttClient::new(o);
    rig.connect().await.unwrap();
    rig.publish(
        "tc/farm/rig/rig/telemetry".into(),
        b"{\"w\":212}".to_vec(),
        QoS::AtMostOnce,
        false,
    )
    .await
    .unwrap();
    // a connection that dies without DISCONNECT: use a second client dropped hard? the clean way: the will fires on abrupt close
    drop(rig);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let all = seen.0.lock().unwrap().clone();
    let t = all
        .iter()
        .find(|(_, m)| m.topic.ends_with("telemetry"))
        .expect("the telemetry");
    assert_eq!(
        (t.0.as_str(), t.1.payload.as_slice(), t.1.retain),
        ("rig", &b"{\"w\":212}"[..], false)
    );
    assert!(
        all.iter().any(|(c, m)| c == "rig"
            && m.topic.ends_with("status")
            && m.payload == b"gone"
            && m.retain),
        "the Last Will is observed too: {all:?}"
    );
    broker.stop().await.unwrap();
}

struct Pems {
    cert: Vec<u8>,
    key: Vec<u8>,
}

fn cert(common_name: &str, dns: &str) -> Pems {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec![dns.to_owned()]).unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, common_name);
    let c = params.self_signed(&key).unwrap();
    Pems {
        cert: c.pem().into_bytes(),
        key: key.serialize_pem().into_bytes(),
    }
}

async fn tls_broker(binding: CertClientId, ca: &Pems, server: &Pems) -> (MqttBroker, u16) {
    let (b, _) = broker(|c| {
        c.tls = Some(BrokerTlsConfig {
            port: 0,
            cert_pem: server.cert.clone(),
            key_pem: server.key.clone(),
            client_ca_pem: Some(ca.cert.clone()),
            bind_client_id: binding,
        });
    })
    .await;
    let p = b.bound_tls_port().unwrap();
    (b, p)
}

fn tls_client(port: u16, id: &str, own: &Pems, server: &Pems) -> MqttClient {
    let mut o = ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5);
    o.tls = Some(TlsOptions {
        ca_cert_pem: Some(server.cert.clone()),
        client_cert_pem: Some(own.cert.clone()),
        client_key_pem: Some(own.key.clone()),
        insecure_skip_certificate_verification: false,
    });
    MqttClient::new(o)
}

#[tokio::test]
async fn the_client_id_is_the_certificates_common_name() {
    let server = {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        Pems {
            cert: cert.pem().into_bytes(),
            key: signing_key.serialize_pem().into_bytes(),
        }
    };
    // one certificate is its own CA (as in the other TLS tests): the broker trusts exactly this one
    let rig = cert("rig-a1", "rig-a1.farm.local");
    let (broker, port) = tls_broker(CertClientId::CommonName, &rig, &server).await;

    // its own name: in
    let ok = tls_client(port, "rig-a1", &rig, &server);
    assert_eq!(ok.connect().await.unwrap().reason_code, 0x00);
    // another rig's name: refused, though the certificate is valid
    let spoof = tls_client(port, "rig-b2", &rig, &server);
    assert!(
        spoof.connect().await.is_err(),
        "a client cannot take another's id"
    );
    ok.disconnect().await.unwrap();
    // no id at all: the certificate gives it one (and the session is that client's, not a generated name)
    let blank = tls_client(port, "", &rig, &server);
    assert_eq!(blank.connect().await.unwrap().reason_code, 0x00);
    blank.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn a_dns_name_binding_and_a_client_that_sends_no_id() {
    let server = {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        Pems {
            cert: cert.pem().into_bytes(),
            key: signing_key.serialize_pem().into_bytes(),
        }
    };
    let rig = cert("whatever", "rig-a1.farm.local");
    let (broker, port) = tls_broker(CertClientId::DnsName, &rig, &server).await;
    assert!(
        tls_client(port, "whatever", &rig, &server)
            .connect()
            .await
            .is_err(),
        "the common name is not the id here"
    );
    assert_eq!(
        tls_client(port, "rig-a1.farm.local", &rig, &server)
            .connect()
            .await
            .unwrap()
            .reason_code,
        0x00
    );
    broker.stop().await.unwrap();

    // without a binding the id is the client's own, as before
    let (open, port) = tls_broker(CertClientId::None, &rig, &server).await;
    assert_eq!(
        tls_client(port, "anything", &rig, &server)
            .connect()
            .await
            .unwrap()
            .reason_code,
        0x00
    );
    open.stop().await.unwrap();
}
