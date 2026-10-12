//! `cargo bench -p stem-mqtt-client` — packet codec and topic matching.
use bytes::{Bytes, BytesMut};
use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::topic::topic_matches;
use mqtt_client::{MqttVersion, QoS};

fn publish(payload: usize, qos: QoS) -> Packet {
    Packet::Publish(PublishPacket {
        dup: false,
        qos,
        retain: false,
        topic: "sensors/building-7/floor-3/room-12/temperature".into(),
        packet_id: (qos != QoS::AtMostOnce).then_some(1),
        payload: Bytes::from(vec![0xAB; payload]),
        properties: Properties::new(),
    })
}

fn codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("publish");
    for size in [16usize, 1024, 64 * 1024] {
        let packet = publish(size, QoS::AtLeastOnce);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("encode", size), &packet, |b, p| {
            b.iter(|| black_box(p.encode(MqttVersion::V5).unwrap()));
        });
        let wire = packet.encode(MqttVersion::V5).unwrap();
        group.bench_with_input(BenchmarkId::new("decode", size), &wire, |b, wire| {
            b.iter_batched(
                || BytesMut::from(&wire[..]),
                |mut buf| black_box(Packet::decode(&mut buf, MqttVersion::V5).unwrap()),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn topics(c: &mut Criterion) {
    let mut group = c.benchmark_group("topic_matches");
    let topic = "sensors/building-7/floor-3/room-12/temperature";
    for (name, filter) in [
        ("exact", topic),
        ("plus", "sensors/+/floor-3/+/temperature"),
        ("hash", "sensors/#"),
        ("miss", "sensors/building-8/#"),
    ] {
        group.bench_function(name, |b| b.iter(|| black_box(topic_matches(filter, topic))));
    }
    group.finish();
}

criterion_group!(benches, codec, topics);
criterion_main!(benches);
