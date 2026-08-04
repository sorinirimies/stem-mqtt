//! Hierarchical topics and the two subscription wildcards:
//! `+` (single level) and `#` (multi level, must be last).
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
//! cargo run -p mqtt-client --example topics
//! ```
//!
//! What to expect: three subscriptions with different specificity —
//! an exact topic, a `+` wildcard one level deep, and a `#` wildcard
//! covering everything under `home/` — then four publishes to topics of
//! varying depth, so you can see exactly which subscriptions catch which
//! messages and why.

use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};

/// Tags every delivered message with which subscription filter it arrived
/// through, so one listener can demonstrate all three at once.
struct Tagged {
    filter: &'static str,
}

impl MqttMessageListener for Tagged {
    fn on_message(&self, message: MqttMessage) {
        println!(
            "  matched {:>24} <- {:<32} {:?}",
            format!("{:?}", self.filter),
            message.topic,
            String::from_utf8_lossy(&message.payload)
        );
    }

    fn on_disconnected(&self, _reason: String) {}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    // One client per filter makes it obvious in the output which filter
    // caught which message — a single client with three subscriptions
    // would work identically, just harder to read here.
    let exact = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        1883,
        "topics-exact",
        MqttVersion::V5,
    ));
    let single_level = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        1883,
        "topics-plus",
        MqttVersion::V5,
    ));
    let multi_level = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        1883,
        "topics-hash",
        MqttVersion::V5,
    ));

    exact.set_message_listener(Arc::new(Tagged {
        filter: "home/kitchen/temperature",
    }));
    single_level.set_message_listener(Arc::new(Tagged {
        filter: "home/+/temperature",
    }));
    multi_level.set_message_listener(Arc::new(Tagged { filter: "home/#" }));

    for c in [&exact, &single_level, &multi_level] {
        c.connect().await?;
    }

    // Exact match: only "home/kitchen/temperature" itself.
    exact
        .subscribe("home/kitchen/temperature".into(), QoS::AtLeastOnce)
        .await?;
    // `+` matches exactly one level in that position — catches
    // "home/kitchen/temperature" and "home/bedroom/temperature" but *not*
    // "home/kitchen/humidity" (wrong last level) or
    // "home/kitchen/temperature/detail" (`+` never matches multiple
    // levels or zero levels).
    single_level
        .subscribe("home/+/temperature".into(), QoS::AtLeastOnce)
        .await?;
    // `#` matches its position and everything below it — catches every
    // topic below "home/", any depth.
    multi_level
        .subscribe("home/#".into(), QoS::AtLeastOnce)
        .await?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    let publisher = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        1883,
        "topics-publisher",
        MqttVersion::V5,
    ));
    publisher.connect().await?;

    let messages: &[(&str, &[u8])] = &[
        ("home/kitchen/temperature", b"21.5C"),
        ("home/bedroom/temperature", b"19.0C"),
        ("home/kitchen/humidity", b"48%"),
        ("home/kitchen/temperature/detail", b"21.5C +/-0.1"),
    ];

    println!("publishing {} messages:\n", messages.len());
    for (topic, payload) in messages {
        println!("  -> {topic}");
        publisher
            .publish(topic.to_string(), payload.to_vec(), QoS::AtLeastOnce, false)
            .await?;
    }

    println!();
    tokio::time::sleep(Duration::from_millis(300)).await;

    for c in [&exact, &single_level, &multi_level, &publisher] {
        c.disconnect().await?;
    }
    println!("done.");
    Ok(())
}
