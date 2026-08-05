//! Minimal embedded broker — the library equivalent of the `mqtt-broker`
//! CLI binary.
//!
//! ```sh
//! cargo run -p stem-mqtt-broker --example simple_broker
//! ```

use mqtt_broker::{MqttBroker, MqttBrokerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let config = MqttBrokerConfig::new("127.0.0.1", 1883);
    let broker = MqttBroker::new(config);

    broker
        .start()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "listening on 127.0.0.1:{} — Ctrl-C to stop",
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
