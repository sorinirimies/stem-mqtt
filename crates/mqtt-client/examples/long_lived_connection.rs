//! A long-lived connection: connect once, stay connected for a while
//! (well past the keep-alive interval, so idle periods are actually
//! exercised), and publish periodic heartbeats — the shape a real
//! background service or IoT device holds an MQTT connection in, as
//! opposed to the connect-publish-disconnect pattern the other examples
//! use.
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
//! cargo run -p mqtt-client --example long_lived_connection
//! ```
//!
//! Runs for ~20 seconds. `keep_alive_secs` is set low (3s, vs. the default
//! 30s) purely so this demo doesn't have to run for minutes to show more
//! than one PINGREQ/PINGRESP cycle — [`MqttClient`] sends these
//! automatically in the background the whole time; nothing here does it
//! manually. Watch the broker's own logs (`RUST_LOG=mqtt_broker=debug`)
//! if you want to see the pings arriving.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};

struct Counter(AtomicU64);

impl MqttMessageListener for Counter {
    fn on_message(&self, message: MqttMessage) {
        let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        println!(
            "  <- [{n}] {:?}: {:?}",
            message.topic,
            String::from_utf8_lossy(&message.payload)
        );
    }

    fn on_disconnected(&self, reason: String) {
        // In a real long-running service this is where you'd trigger a
        // reconnect-with-backoff loop; see the module docs above for why
        // this example doesn't need one.
        println!("  !! connection lost: {reason}");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let topic = "stem-mqtt/examples/heartbeat";
    let total_runtime = Duration::from_secs(20);
    let publish_every = Duration::from_secs(2);

    let mut options = ConnectOptions::new(
        "127.0.0.1",
        1883,
        "long-lived-connection-example",
        MqttVersion::V5,
    );
    options.keep_alive_secs = 3;
    // A `clean_start = false` session would additionally survive a brief
    // disconnect (messages published while we're offline get queued by
    // the broker and flushed on reconnect) — left as the default `true`
    // here since this example focuses on staying connected, not resuming
    // a session. See `will_and_retain` for session-adjacent behavior.

    let client = Arc::new(MqttClient::new(options));
    client.set_message_listener(Arc::new(Counter(AtomicU64::new(0))));

    client.connect().await?;
    client.subscribe(topic.into(), QoS::AtLeastOnce).await?;
    println!(
        "connected; publishing a heartbeat every {}s for {}s total (keep_alive={}s)...\n",
        publish_every.as_secs(),
        total_runtime.as_secs(),
        3
    );

    let start = tokio::time::Instant::now();
    let mut n: u64 = 0;
    while start.elapsed() < total_runtime {
        tokio::time::sleep(publish_every).await;
        n += 1;
        let payload = format!("heartbeat #{n} at {:.1}s", start.elapsed().as_secs_f32());
        client
            .publish(
                topic.into(),
                payload.clone().into_bytes(),
                QoS::AtLeastOnce,
                false,
            )
            .await?;
        println!("  -> {payload} (connected={})", client.is_connected());
    }

    // Give the last heartbeat's delivery a moment to land before tearing
    // down, same as the other examples.
    tokio::time::sleep(Duration::from_millis(200)).await;

    client.disconnect().await?;
    println!(
        "\ndone — connection held for {:.1}s.",
        start.elapsed().as_secs_f32()
    );
    Ok(())
}
