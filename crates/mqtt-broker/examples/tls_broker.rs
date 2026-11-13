//! A broker with a TLS listener and a throwaway self-signed certificate, to try TLS clients.
//!
//! ```sh
//! cargo run -p stem-mqtt-broker --example tls_broker
//! # then, in another terminal, trust the certificate it wrote:
//! cargo run -p stem-mqtt-client-blocking --features rustls --example mqtts -- \
//!     pub localhost 8883 home/light on "${TMPDIR:-/tmp}/stem-mqtt-tls/ca.pem"
//! ```
//!
//! The certificate is valid for `localhost` and `127.0.0.1` and is regenerated on every start. It
//! is written to `<tmp>/stem-mqtt-tls/ca.pem`, the file a client passes as its trusted CA.

use mqtt_broker::{BrokerTlsConfig, MqttBroker, MqttBrokerConfig};

const TLS_PORT: u16 = 8883;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let names = vec!["localhost".to_owned(), "127.0.0.1".to_owned()];
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(names)?;
    let (cert_pem, key_pem) = (cert.pem(), signing_key.serialize_pem());

    let dir = std::env::temp_dir().join("stem-mqtt-tls");
    std::fs::create_dir_all(&dir)?;
    let ca_path = dir.join("ca.pem");
    std::fs::write(&ca_path, &cert_pem)?;

    let config = MqttBrokerConfig {
        tls: Some(BrokerTlsConfig {
            port: TLS_PORT,
            cert_pem: cert_pem.into_bytes(),
            key_pem: key_pem.into_bytes(),
            client_ca_pem: None,
        }),
        ..MqttBrokerConfig::new("127.0.0.1", 1883)
    };
    let broker = MqttBroker::new(config);
    broker
        .start()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "TLS on localhost:{} (plain on {}); CA certificate: {} — Ctrl-C to stop",
        broker.bound_tls_port().unwrap_or(TLS_PORT),
        broker.bound_port().unwrap_or(1883),
        ca_path.display()
    );

    tokio::signal::ctrl_c().await?;
    broker
        .stop()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("stopped.");
    Ok(())
}
