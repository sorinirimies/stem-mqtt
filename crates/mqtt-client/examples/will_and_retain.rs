//! Demonstrates a Last-Will-and-Testament message and a retained PUBLISH.
//!
//! Start a broker first: `cargo run -p stem-mqtt-broker --bin mqtt-broker`
//!
//! Then: `cargo run -p stem-mqtt-client --example will_and_retain`
//!
//! What to expect: a first client connects with a will message and a
//! second client subscribes to the will topic *before* the first client
//! is killed (simulated here by dropping the connection without sending
//! DISCONNECT) — the broker publishes the will on its behalf. Separately,
//! a retained message on another topic is delivered immediately to any
//! *new* subscriber, even one that subscribes after the message was sent.

use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS, WillOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let retained_topic = "stem-mqtt/examples/retained";
    let will_topic = "stem-mqtt/examples/will";

    // ── 1. Publish a retained message, then disconnect ───────────────────
    {
        let publisher = MqttClient::new(ConnectOptions::new(
            "127.0.0.1",
            1883,
            "retain-publisher",
            MqttVersion::V5,
        ));
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
        println!("published a retained message to {retained_topic:?}");
    }

    // ── 2. A brand-new subscriber gets the retained message immediately ──
    {
        let subscriber = MqttClient::new(ConnectOptions::new(
            "127.0.0.1",
            1883,
            "retain-subscriber",
            MqttVersion::V5,
        ));
        subscriber.connect().await?;
        subscriber
            .subscribe(retained_topic.into(), QoS::AtLeastOnce)
            .await?;
        println!("subscribed to {retained_topic:?} — retained message should arrive now");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        subscriber.disconnect().await?;
    }

    // ── 3. Connect with a will, then drop the connection uncleanly ───────
    {
        let mut opts = ConnectOptions::new("127.0.0.1", 1883, "will-demo-client", MqttVersion::V5);
        opts.will = Some(WillOptions {
            topic: will_topic.into(),
            payload: b"will-demo-client vanished".to_vec(),
            qos: QoS::AtLeastOnce,
            retain: false,
        });
        let client = MqttClient::new(opts);
        client.connect().await?;
        println!("connected with a will on {will_topic:?}; dropping without DISCONNECT...");
        // Dropping the client (rather than calling `disconnect()`) closes
        // the socket without a clean DISCONNECT, so the broker publishes
        // the will message.
        drop(client);
    }

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    println!("done.");
    Ok(())
}
