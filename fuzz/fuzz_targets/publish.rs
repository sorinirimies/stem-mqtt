//! Structured fuzzing: build a PUBLISH from arbitrary fields and require a
//! lossless encode → decode round trip for every valid combination.
#![no_main]

use bytes::{Bytes, BytesMut};
use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::protocol::properties::Properties;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::topic::is_valid_topic_name;
use mqtt_client::{MqttVersion, QoS};

fuzz_target!(|input: (u8, bool, bool, &str, Vec<u8>, u16, bool)| {
    let (q, dup, retain, topic, payload, id, v5) = input;
    let qos = match q % 3 {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        _ => QoS::ExactlyOnce,
    };
    if !is_valid_topic_name(topic) || (qos != QoS::AtMostOnce && id == 0) {
        return;
    }
    let version = if v5 { MqttVersion::V5 } else { MqttVersion::V311 };
    let packet = Packet::Publish(PublishPacket {
        dup: dup && qos != QoS::AtMostOnce, // DUP must be 0 for QoS 0
        qos,
        retain,
        topic: topic.to_string(),
        packet_id: (qos != QoS::AtMostOnce).then_some(id),
        payload: Bytes::from(payload),
        properties: Properties::new(),
    });
    let encoded = packet.encode(version).expect("valid PUBLISH must encode");
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = Packet::decode(&mut buf, version).unwrap().unwrap();
    assert_eq!(decoded, packet);
    assert!(buf.is_empty());
});
