//! Minimal publish/subscribe round trip over a single connection.
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
//! cargo run -p mqtt-client --example pub_sub
//! ```

use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};

struct Printer;

impl MqttMessageListener for Printer {
    fn on_message(&self, message: MqttMessage) {
        println!(
            "  <- received on {:?}: {:?}",
            message.topic,
            String::from_utf8_lossy(&message.payload)
        );
    }

    fn on_disconnected(&self, reason: String) {
        println!("  disconnected: {reason}");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let topic = "stem-mqtt/examples/pub_sub";

    let client = MqttClient::new(ConnectOptions::new(
        "127.0.0.1",
        1883,
        "pub-sub-example",
        MqttVersion::V5,
    ));
    client.set_message_listener(Arc::new(Printer));

    println!("connecting...");
    let result = client.connect().await?;
    println!("connected (session_present={})", result.session_present);

    println!("subscribing to {topic:?}...");
    client.subscribe(topic.into(), QoS::AtLeastOnce).await?;

    println!("publishing 3 messages...");
    for i in 0..3 {
        client
            .publish(
                topic.into(),
                format!("hello #{i}").into_bytes(),
                QoS::AtLeastOnce,
                false,
            )
            .await?;
    }

    // Give the read loop a moment to deliver the messages before we
    // disconnect and tear down the runtime.
    tokio::time::sleep(Duration::from_millis(200)).await;

    client.disconnect().await?;
    println!("done.");
    Ok(())
}
