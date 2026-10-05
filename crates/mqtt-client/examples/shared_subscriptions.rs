//! Shared subscriptions: several workers subscribe to `$share/<group>/<filter>` and the broker
//! delivers each message to exactly **one** member of the group, round-robin — a work queue
//! on plain MQTT, no extra infrastructure.
//!
//! Start a broker first: `cargo run -p stem-mqtt-broker --bin mqtt-broker`
//!
//! Then: `cargo run -p stem-mqtt-client --example shared_subscriptions`
//!
//! Three workers join the group `workers` on `jobs/#`; a producer publishes nine jobs. Every
//! job is printed exactly once, and each worker ends up with three of them. A separate
//! ordinary subscriber on the same filter still gets *every* job (shared subscriptions only
//! split delivery *within* a group).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};

struct Worker {
    name: &'static str,
    handled: AtomicUsize,
}

impl MqttMessageListener for Worker {
    fn on_message(&self, message: MqttMessage) {
        self.handled.fetch_add(1, Ordering::Relaxed);
        println!(
            "  {} handled {}",
            self.name,
            String::from_utf8_lossy(&message.payload)
        );
    }

    fn on_disconnected(&self, _reason: String) {}
}

struct Audit(AtomicUsize);

impl MqttMessageListener for Audit {
    fn on_message(&self, _message: MqttMessage) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn on_disconnected(&self, _reason: String) {}
}

async fn connect(id: &str) -> anyhow::Result<MqttClient> {
    let client = MqttClient::new(ConnectOptions::new("127.0.0.1", 1883, id, MqttVersion::V5));
    client.connect().await?;
    Ok(client)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let group_filter = "$share/workers/jobs/#";

    let mut workers = Vec::new();
    let mut clients = Vec::new();
    for name in ["worker-1", "worker-2", "worker-3"] {
        let client = connect(name).await?;
        let worker = Arc::new(Worker {
            name,
            handled: AtomicUsize::new(0),
        });
        client.set_message_listener(worker.clone());
        client
            .subscribe(group_filter.into(), QoS::AtLeastOnce)
            .await?;
        println!("{name} joined {group_filter:?}");
        workers.push(worker);
        clients.push(client);
    }

    // An ordinary (non-shared) subscriber sees every job.
    let audit_client = connect("auditor").await?;
    let audit = Arc::new(Audit(AtomicUsize::new(0)));
    audit_client.set_message_listener(audit.clone());
    audit_client
        .subscribe("jobs/#".into(), QoS::AtLeastOnce)
        .await?;

    let producer = connect("producer").await?;
    println!("\nproducer publishes 9 jobs:");
    for i in 1..=9 {
        producer
            .publish(
                format!("jobs/{i}"),
                format!("job-{i}").into_bytes(),
                QoS::AtLeastOnce,
                false,
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    println!("\nper worker:");
    for w in &workers {
        println!("  {} -> {} jobs", w.name, w.handled.load(Ordering::Relaxed));
    }
    println!(
        "  auditor (plain subscription) -> {} jobs",
        audit.0.load(Ordering::Relaxed)
    );

    producer.disconnect().await?;
    audit_client.disconnect().await?;
    for c in clients {
        c.disconnect().await?;
    }
    Ok(())
}
