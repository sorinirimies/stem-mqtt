//! Same broker, two MQTT protocol versions — negotiated independently per
//! connection, not per broker. One client speaks MQTT 3.1.1, the other
//! MQTT 5.0, and both publish/subscribe against each other just fine: the
//! wire-level differences (property lists, extra reason codes, "Clean
//! Session" vs "Clean Start" naming, ...) are entirely handled by the
//! codec — nothing about the pub/sub API changes based on version.
//!
//! Start a broker first (in another terminal):
//!
//! ```sh
//! cargo run -p mqtt-broker --bin mqtt-broker -- --port 1883
//! ```
//!
//! Then run this example:
//!
//! ```sh
//! cargo run -p mqtt-client --example mqtt_versions
//! ```

use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};

struct Printer(&'static str);

impl MqttMessageListener for Printer {
    fn on_message(&self, message: MqttMessage) {
        println!(
            "  [{}] <- received on {:?}: {:?}",
            self.0,
            message.topic,
            String::from_utf8_lossy(&message.payload)
        );
    }

    fn on_disconnected(&self, reason: String) {
        println!("  [{}] disconnected: {reason}", self.0);
    }
}

async fn connect(
    client_id: &str,
    version: MqttVersion,
    label: &'static str,
) -> anyhow::Result<MqttClient> {
    let client = MqttClient::new(ConnectOptions::new("127.0.0.1", 1883, client_id, version));
    client.set_message_listener(Arc::new(Printer(label)));

    let result = client.connect().await?;
    println!(
        "[{label}] connected as {version:?} (session_present={}, reason_code=0x{:02X})",
        result.session_present, result.reason_code
    );
    Ok(client)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let topic = "stem-mqtt/examples/versions";

    // MQTT-3.1.1: `clean_start` here maps onto the wire as CONNECT's
    // "Clean Session" flag (the 3.1.1 name for the same bit) — the codec
    // and `ConnectOptions` field are identical either way.
    let v311 = connect("versions-v311", MqttVersion::V311, "3.1.1").await?;
    // MQTT 5.0: the same flag is called "Clean Start", and CONNACK can
    // additionally carry MQTT 5-only properties/reason codes the 3.1.1
    // client never sees — none of which this example needs to care about.
    let v5 = connect("versions-v5", MqttVersion::V5, "5.0  ").await?;

    v311.subscribe(topic.into(), QoS::AtLeastOnce).await?;
    v5.subscribe(topic.into(), QoS::AtLeastOnce).await?;

    println!("\n[3.1.1] publishing...");
    v311.publish(
        topic.into(),
        b"hello from a 3.1.1 client".to_vec(),
        QoS::AtLeastOnce,
        false,
    )
    .await?;

    println!("[5.0]   publishing...");
    v5.publish(
        topic.into(),
        b"hello from a 5.0 client".to_vec(),
        QoS::AtLeastOnce,
        false,
    )
    .await?;

    // Each client is also subscribed to the topic it just published on,
    // so both should see both messages — proof the broker treats
    // version-3.1.1 and version-5.0 connections identically once CONNECT
    // is done.
    tokio::time::sleep(Duration::from_millis(300)).await;

    v311.disconnect().await?;
    v5.disconnect().await?;
    println!("\ndone.");
    Ok(())
}
