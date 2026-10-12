//! `cargo bench -p stem-mqtt-broker --features bench-internals` — the
//! publish hot path: subscription-index lookup and fan-out to N subscribers.
use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mqtt_broker::internals::*;
use mqtt_client::{MqttVersion, QoS};
use tokio::sync::{mpsc, oneshot};

struct Fixture {
    registry: SessionRegistry,
    /// Each connection's writer-channel receiver; kept alive (a dropped one
    /// would make sends fail) and drained between iterations.
    peers: Vec<mpsc::Receiver<Bytes>>,
    ids: Vec<String>,
    /// Packet id the next QoS 1 delivery will carry (ids count up from 1).
    next_packet_id: u16,
}

fn message(topic: &str, qos: QoS) -> QueuedMessage {
    QueuedMessage {
        topic: topic.into(),
        payload: Bytes::from(vec![0u8; 256]),
        qos,
        retain: false,
    }
}

/// `n` connected subscribers, each on `filter`.
fn fixture(n: usize, filter: &str, sub_qos: QoS) -> Fixture {
    let registry = SessionRegistry::new();
    let mut peers = Vec::new();
    let mut ids = Vec::new();
    for i in 0..n {
        let id = format!("c{i}");
        let (tx, rx) = mpsc::channel(1 << 16);
        let (shutdown, _) = oneshot::channel(); // never fired here
        registry
            .attach(AttachRequest {
                client_id: id.clone(),
                version: MqttVersion::V5,
                clean_start: true,
                will: None,
                max_queued: 0,
                max_clients: 0,
                sender: tx,
                budget: OutboundBudget::new(usize::MAX / 2),
                shutdown,
            })
            .unwrap();
        registry.add_subscription(
            &id,
            filter,
            Subscription {
                qos: sub_qos,
                no_local: false,
                retain_as_published: false,
            },
        );
        peers.push(rx);
        ids.push(id);
    }
    Fixture {
        registry,
        peers,
        ids,
        next_packet_id: 1,
    }
}

/// Empty every connection's channel and "ack" the QoS 1 packet each one just
/// received — otherwise unacked packets pile up toward the 65 535-id limit and
/// the benchmark would measure packet-id exhaustion instead of fan-out.
/// (Ack cost is part of the measured iteration for QoS 1.)
fn drain(f: &mut Fixture, acks: bool) {
    for rx in &mut f.peers {
        while rx.try_recv().is_ok() {}
    }
    if acks {
        for id in &f.ids {
            f.registry.clear_pending_redelivery(id, f.next_packet_id);
        }
        f.next_packet_id = f.next_packet_id.checked_add(1).unwrap_or(1);
    }
}

fn fan_out(c: &mut Criterion) {
    let mut group = c.benchmark_group("fan_out");
    for n in [1usize, 10, 100, 1000] {
        group.throughput(Throughput::Elements(n as u64));
        for (name, qos) in [("qos0", QoS::AtMostOnce), ("qos1", QoS::AtLeastOnce)] {
            let mut f = fixture(n, "sensors/+/temperature", qos);
            let msg = message("sensors/room-1/temperature", qos);
            group.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
                b.iter(|| {
                    f.registry.fan_out("publisher", black_box(&msg));
                    drain(&mut f, qos != QoS::AtMostOnce);
                });
            });
        }
    }
    group.finish();
}

fn index_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_lookup");
    for subs in [100usize, 10_000, 100_000] {
        let mut index = SubscriptionIndex::default();
        for i in 0..subs {
            let key = format!("fleet/{}/vehicle-{i}/telemetry", i % 50);
            index.insert(IndexEntry::from_key(&format!("c{i}"), &key).unwrap());
        }
        index.insert(IndexEntry::from_key("watcher", "fleet/#").unwrap());
        group.bench_with_input(
            BenchmarkId::new("one_hit_plus_wildcard", subs),
            &subs,
            |b, _| {
                b.iter(|| black_box(index.matching("fleet/7/vehicle-7/telemetry")));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, fan_out, index_lookup);
criterion_main!(benches);
