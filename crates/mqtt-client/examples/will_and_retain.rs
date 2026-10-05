//! Retained messages and Last Will and Testament, with the deliveries printed.
//!
//! Start a broker first: `cargo run -p stem-mqtt-broker --bin mqtt-broker`
//!
//! Then: `cargo run -p stem-mqtt-client --example will_and_retain`
//!
//! 1. **Retained message** — a publisher sends a message with `retain = true` and disconnects.
//!    A subscriber that arrives *afterwards* still receives it immediately (flagged `retained`).
//! 2. **Last Will** — a watcher subscribes to a topic. A second client connects with a will on
//!    that topic and then vanishes without sending DISCONNECT (its connection is simply
//!    dropped). The broker publishes the will on the dead client's behalf and the watcher
//!    prints it.

use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{
    ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS, WillOptions,
};

/// Prints every delivery with a label.
struct Printer(&'static str);

impl MqttMessageListener for Printer {
    fn on_message(&self, message: MqttMessage) {
        println!(
            "  [{}] {:?} on {:?}{}",
            self.0,
            String::from_utf8_lossy(&message.payload),
            message.topic,
            if message.retain { "  (retained)" } else { "" }
        );
    }

    fn on_disconnected(&self, _reason: String) {}
}

fn client(id: &str) -> MqttClient {
    MqttClient::new(ConnectOptions::new("127.0.0.1", 1883, id, MqttVersion::V5))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let retained_topic = "stem-mqtt/examples/retained";
    let will_topic = "stem-mqtt/examples/will";

    // ── 1. Retained message: published first, delivered to a later subscriber ──
    println!("1. retained message");
    let publisher = client("retain-publisher");
    publisher.connect().await?;
    publisher
        .publish(
            retained_topic.into(),
            b"this sticks around".to_vec(),
            QoS::AtLeastOnce,
            /* retain = */ true,
        )
        .await?;
    publisher.disconnect().await?;
    println!("  published (retain=true), publisher is gone");

    let late = client("late-subscriber");
    late.set_message_listener(Arc::new(Printer("late subscriber")));
    late.connect().await?;
    println!("  a new subscriber arrives and subscribes ...");
    late.subscribe(retained_topic.into(), QoS::AtLeastOnce)
        .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    late.disconnect().await?;

    // ── 2. Last Will: the broker speaks for a client that vanished ──────────
    println!("\n2. last will and testament");
    let watcher = client("watcher");
    watcher.set_message_listener(Arc::new(Printer("watcher")));
    watcher.connect().await?;
    watcher
        .subscribe(will_topic.into(), QoS::AtLeastOnce)
        .await?;
    println!("  watcher is subscribed to {will_topic:?}");

    let mut options = ConnectOptions::new("127.0.0.1", 1883, "doomed-client", MqttVersion::V5);
    options.will = Some(WillOptions {
        topic: will_topic.into(),
        payload: b"doomed-client vanished".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: false,
    });
    let doomed = MqttClient::new(options);
    doomed.connect().await?;
    println!("  doomed-client connected with a will; now it drops without DISCONNECT ...");
    // Dropping the client (instead of `disconnect()`) closes the socket without a DISCONNECT
    // packet, so the broker publishes the will.
    drop(doomed);

    tokio::time::sleep(Duration::from_millis(500)).await;
    watcher.disconnect().await?;
    println!("done.");
    Ok(())
}
