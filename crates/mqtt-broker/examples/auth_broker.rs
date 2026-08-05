//! A broker with pluggable authentication and connect/disconnect/publish
//! observability — the two extension points foreign (Kotlin/Swift/Python)
//! callers get through UniFFI's `with_foreign` callback interfaces.
//!
//! ```sh
//! cargo run -p stem-mqtt-broker --example auth_broker
//! ```
//!
//! Only `username = "demo"` / `password = "demo"` (or no credentials, since
//! `allow_anonymous` defaults to `true`) is accepted; every other login is
//! rejected. Every connect, disconnect, and publish is logged to stdout.

use std::sync::Arc;

use mqtt_broker::{MqttAuthProvider, MqttBroker, MqttBrokerConfig, MqttBrokerEventListener};
use mqtt_client::QoS;

struct FixedCredentials;

impl MqttAuthProvider for FixedCredentials {
    fn authenticate(
        &self,
        client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
    ) -> bool {
        let ok = matches!(
            (username.as_deref(), password.as_deref()),
            (Some("demo"), Some(b"demo")) | (None, None)
        );
        println!(
            "[auth] client_id={client_id:?} username={username:?} -> {}",
            if ok { "accepted" } else { "rejected" }
        );
        ok
    }
}

struct Logger;

impl MqttBrokerEventListener for Logger {
    fn on_client_connected(&self, client_id: String) {
        println!("[event] connected:    {client_id}");
    }

    fn on_client_disconnected(&self, client_id: String, reason: String) {
        println!("[event] disconnected: {client_id} ({reason})");
    }

    fn on_message_published(&self, client_id: String, topic: String, qos: QoS) {
        println!("[event] published:    {client_id} -> {topic} (qos {qos:?})");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let config = MqttBrokerConfig::new("127.0.0.1", 1883);
    let broker = MqttBroker::new(config);
    broker.set_auth_provider(Arc::new(FixedCredentials));
    broker.set_event_listener(Arc::new(Logger));

    broker
        .start()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "listening on 127.0.0.1:{} (username=\"demo\" password=\"demo\") — Ctrl-C to stop",
        broker.bound_port().unwrap_or(1883)
    );

    tokio::signal::ctrl_c().await?;

    broker
        .stop()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("stopped.");
    Ok(())
}
