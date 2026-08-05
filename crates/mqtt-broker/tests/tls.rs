//! TLS end-to-end: a real [`MqttBroker`] with its TLS listener enabled,
//! driven by a real [`MqttClient`] connecting over TLS — nothing mocked,
//! using a self-signed certificate generated on the fly via `rcgen`.

use mqtt_broker::{BrokerTlsConfig, MqttBroker, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS, TlsOptions};

/// A self-signed certificate + key for `127.0.0.1`, PEM-encoded, plus the
/// same certificate again as the trusted root a client would use to
/// verify it (self-signed certs are their own trust anchor).
struct SelfSignedCert {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
}

fn generate_self_signed(subject_alt_names: Vec<String>) -> SelfSignedCert {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(subject_alt_names)
            .expect("cert generation should succeed");
    SelfSignedCert {
        cert_pem: cert.pem().into_bytes(),
        key_pem: signing_key.serialize_pem().into_bytes(),
    }
}

#[tokio::test]
async fn client_connects_to_broker_over_tls() {
    let server_cert = generate_self_signed(vec!["127.0.0.1".to_string()]);

    let config = MqttBrokerConfig {
        tls: Some(BrokerTlsConfig {
            port: 0,
            cert_pem: server_cert.cert_pem.clone(),
            key_pem: server_cert.key_pem,
            client_ca_pem: None,
        }),
        ..MqttBrokerConfig::new("127.0.0.1", 0)
    };
    let broker = MqttBroker::new(config);
    broker.start().await.expect("broker should bind and start");
    let tls_port = broker
        .bound_tls_port()
        .expect("TLS port must be known after start()");

    let mut options = ConnectOptions::new("127.0.0.1", tls_port, "tls-client", MqttVersion::V5);
    options.tls = Some(TlsOptions {
        // Trust the server's own (self-signed) certificate directly,
        // rather than skipping verification entirely — this exercises the
        // real certificate-verification path, not just "TLS handshake
        // happens at all".
        ca_cert_pem: Some(server_cert.cert_pem),
        client_cert_pem: None,
        client_key_pem: None,
        insecure_skip_certificate_verification: false,
    });
    let client = MqttClient::new(options);
    let result = client
        .connect()
        .await
        .expect("TLS handshake + CONNECT should succeed");
    assert_eq!(result.reason_code, 0x00);

    client
        .publish("t/tls".into(), b"over tls".to_vec(), QoS::AtMostOnce, false)
        .await
        .expect("publish over TLS should succeed");

    client.disconnect().await.unwrap();
    broker.stop().await.unwrap();
}

#[tokio::test]
async fn client_rejects_untrusted_server_certificate() {
    let server_cert = generate_self_signed(vec!["127.0.0.1".to_string()]);
    // A *different* self-signed cert the client will trust instead of the
    // server's real one — the handshake must fail.
    let unrelated_cert = generate_self_signed(vec!["127.0.0.1".to_string()]);

    let config = MqttBrokerConfig {
        tls: Some(BrokerTlsConfig {
            port: 0,
            cert_pem: server_cert.cert_pem,
            key_pem: server_cert.key_pem,
            client_ca_pem: None,
        }),
        ..MqttBrokerConfig::new("127.0.0.1", 0)
    };
    let broker = MqttBroker::new(config);
    broker.start().await.expect("broker should bind and start");
    let tls_port = broker.bound_tls_port().unwrap();

    let mut options = ConnectOptions::new("127.0.0.1", tls_port, "tls-client", MqttVersion::V5);
    options.tls = Some(TlsOptions {
        ca_cert_pem: Some(unrelated_cert.cert_pem),
        client_cert_pem: None,
        client_key_pem: None,
        insecure_skip_certificate_verification: false,
    });
    let client = MqttClient::new(options);
    let err = client.connect().await.unwrap_err();
    assert!(
        matches!(err, mqtt_client::MqttError::Io(_)),
        "expected a TLS handshake failure, got {err:?}"
    );

    broker.stop().await.unwrap();
}

#[tokio::test]
async fn mutual_tls_requires_a_trusted_client_certificate() {
    let server_cert = generate_self_signed(vec!["127.0.0.1".to_string()]);
    let client_ca = generate_self_signed(vec!["stem-mqtt-test-client-ca".to_string()]);

    let config = MqttBrokerConfig {
        tls: Some(BrokerTlsConfig {
            port: 0,
            cert_pem: server_cert.cert_pem.clone(),
            key_pem: server_cert.key_pem,
            // Requires every connecting client to present a certificate
            // signed by this CA.
            client_ca_pem: Some(client_ca.cert_pem.clone()),
        }),
        ..MqttBrokerConfig::new("127.0.0.1", 0)
    };
    let broker = MqttBroker::new(config);
    broker.start().await.expect("broker should bind and start");
    let tls_port = broker.bound_tls_port().unwrap();

    // A client presenting the trusted CA's own cert as its client
    // certificate (self-signed-as-its-own-CA, same trick as the server
    // side) must be accepted.
    let mut authed_options =
        ConnectOptions::new("127.0.0.1", tls_port, "mtls-client-good", MqttVersion::V5);
    authed_options.tls = Some(TlsOptions {
        ca_cert_pem: Some(server_cert.cert_pem.clone()),
        client_cert_pem: Some(client_ca.cert_pem.clone()),
        client_key_pem: Some(client_ca.key_pem.clone()),
        insecure_skip_certificate_verification: false,
    });
    let authed_client = MqttClient::new(authed_options);
    authed_client
        .connect()
        .await
        .expect("a client presenting a cert signed by the trusted CA should be accepted");
    authed_client.disconnect().await.unwrap();

    // A client with no certificate at all must be rejected.
    let mut anon_options =
        ConnectOptions::new("127.0.0.1", tls_port, "mtls-client-bad", MqttVersion::V5);
    anon_options.tls = Some(TlsOptions {
        ca_cert_pem: Some(server_cert.cert_pem),
        client_cert_pem: None,
        client_key_pem: None,
        insecure_skip_certificate_verification: false,
    });
    let anon_client = MqttClient::new(anon_options);
    let err = anon_client.connect().await.unwrap_err();
    assert!(
        matches!(err, mqtt_client::MqttError::Io(_)),
        "expected a TLS handshake failure (no client cert presented), got {err:?}"
    );

    broker.stop().await.unwrap();
}
