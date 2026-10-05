//! The packet decoder is the broker's (and client's) main untrusted-input
//! surface. These deterministic, dependency-free tests hammer it with random
//! and mutated bytes and assert the properties that matter for a network
//! service: **it never panics, never loops, and never over-allocates**.
//!
//! (For coverage-guided fuzzing see `fuzz/` — this is the always-on,
//! no-nightly-required counterpart that runs in every CI build.)

use bytes::{Bytes, BytesMut};
use mqtt_client::protocol::ack::SimpleAck;
use mqtt_client::protocol::connect::{ConnAckPacket, ConnectPacket, ConnectReasonCode, Will};
use mqtt_client::protocol::packet::{DisconnectPacket, Packet};
use mqtt_client::protocol::properties::{Properties, Property};
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::protocol::subscribe::{
    SubAckPacket, SubAckReasonCode, SubscribeFilter, SubscribePacket, UnsubAckPacket,
    UnsubscribePacket,
};
use mqtt_client::{MqttVersion, QoS};

/// Tiny deterministic PRNG (xorshift64*), so failures reproduce exactly.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const LIMIT: usize = 4096;

/// The invariant every input must satisfy. Returns how many packets decoded.
fn check(bytes: &[u8], version: MqttVersion) -> usize {
    let mut buf = BytesMut::from(bytes);
    let mut decoded = 0;
    // A decoder that makes no progress would spin here; the cap proves it can't.
    for _ in 0..(bytes.len() + 2) {
        let before = buf.len();
        match Packet::decode_with_limit(&mut buf, version, LIMIT) {
            Ok(Some(packet)) => {
                assert!(
                    buf.len() < before,
                    "decoded a packet without consuming input"
                );
                // Whatever decoded must also re-encode without panicking.
                let _ = packet.encode(version);
                decoded += 1;
            }
            Ok(None) => {
                assert_eq!(buf.len(), before, "an incomplete read must not consume");
                break;
            }
            Err(_) => break, // malformed input is an error, never a panic
        }
    }
    decoded
}

fn corpus() -> Vec<(MqttVersion, Vec<u8>)> {
    let mut props = Properties::new();
    props.push(Property::ReasonString("why".into()));
    props.push(Property::UserProperty(("k".into(), "v".into())));
    let publish = |qos, id| {
        Packet::Publish(PublishPacket {
            dup: false,
            qos,
            retain: true,
            topic: "a/b/c".into(),
            packet_id: id,
            payload: Bytes::from_static(b"payload"),
            properties: props.clone(),
        })
    };
    let packets = vec![
        Packet::Connect(ConnectPacket {
            version: MqttVersion::V5,
            client_id: "cid".into(),
            clean_start: true,
            keep_alive: 30,
            username: Some("user".into()),
            password: Some(Bytes::from_static(b"secret")),
            will: Some(Will {
                topic: "w".into(),
                payload: Bytes::from_static(b"bye"),
                qos: QoS::AtLeastOnce,
                retain: false,
                properties: Properties::new(),
                delay_interval: 0,
            }),
            properties: props.clone(),
        }),
        Packet::ConnAck(ConnAckPacket {
            session_present: true,
            reason_code: ConnectReasonCode::SUCCESS,
            properties: props.clone(),
        }),
        publish(QoS::AtMostOnce, None),
        publish(QoS::AtLeastOnce, Some(7)),
        publish(QoS::ExactlyOnce, Some(8)),
        Packet::PubAck(SimpleAck::success(1)),
        Packet::PubRec(SimpleAck::success(2)),
        Packet::PubRel(SimpleAck::success(3)),
        Packet::PubComp(SimpleAck::success(4)),
        Packet::Subscribe(SubscribePacket {
            packet_id: 5,
            filters: vec![SubscribeFilter::new("x/+/#", QoS::ExactlyOnce)],
            properties: Properties::new(),
        }),
        Packet::SubAck(SubAckPacket {
            packet_id: 5,
            reason_codes: vec![SubAckReasonCode::granted(QoS::AtLeastOnce)],
            properties: Properties::new(),
        }),
        Packet::Unsubscribe(UnsubscribePacket {
            packet_id: 6,
            topic_filters: vec!["x".into()],
            properties: Properties::new(),
        }),
        Packet::UnsubAck(UnsubAckPacket {
            packet_id: 6,
            reason_codes: vec![0],
            properties: Properties::new(),
        }),
        Packet::PingReq,
        Packet::PingResp,
        Packet::Disconnect(DisconnectPacket::normal()),
    ];
    let mut out = Vec::new();
    for version in [MqttVersion::V311, MqttVersion::V5] {
        for p in &packets {
            if let Ok(bytes) = p.encode(version) {
                out.push((version, bytes.to_vec()));
            }
        }
    }
    out
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0xDEADBEEFCAFEF00D);
    for _ in 0..200_000 {
        let len = rng.below(48);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        check(&bytes, MqttVersion::V311);
        check(&bytes, MqttVersion::V5);
    }
}

#[test]
fn mutated_valid_packets_never_panic() {
    let mut rng = Rng(0x0123456789ABCDEF);
    let corpus = corpus();
    assert!(corpus.len() >= 20, "corpus should cover every packet type");
    for (version, original) in &corpus {
        assert!(check(original, *version) >= 1, "valid corpus must decode");
        for _ in 0..4_000 {
            let mut bytes = original.clone();
            for _ in 0..(1 + rng.below(4)) {
                match rng.below(4) {
                    0 if !bytes.is_empty() => {
                        let i = rng.below(bytes.len());
                        bytes[i] ^= 1 << rng.below(8);
                    }
                    1 if !bytes.is_empty() => {
                        let i = rng.below(bytes.len());
                        bytes[i] = rng.next() as u8;
                    }
                    2 => {
                        let cut = rng.below(bytes.len() + 1);
                        bytes.truncate(cut);
                    }
                    _ => bytes.push(rng.next() as u8),
                }
            }
            check(&bytes, *version);
        }
    }
}

#[test]
fn a_valid_stream_decodes_identically_however_it_is_chunked() {
    let corpus = corpus();
    for version in [MqttVersion::V311, MqttVersion::V5] {
        let stream: Vec<u8> = corpus
            .iter()
            .filter(|(v, _)| *v == version)
            .flat_map(|(_, b)| b.clone())
            .collect();
        let whole = check(&stream, version);

        // Feed it a few bytes at a time, as a slow socket would.
        for chunk_size in [1usize, 2, 3, 7, 64] {
            let mut buf = BytesMut::new();
            let mut got = 0;
            for chunk in stream.chunks(chunk_size) {
                buf.extend_from_slice(chunk);
                while let Some(_p) = Packet::decode_with_limit(&mut buf, version, LIMIT).unwrap() {
                    got += 1;
                }
            }
            assert_eq!(got, whole, "chunk size {chunk_size}");
            assert!(buf.is_empty(), "nothing may be left over");
        }
    }
}

#[test]
fn huge_declared_lengths_are_rejected_without_buffering() {
    // Every packet type announcing a ~256 MiB body, with no body bytes sent.
    for type_byte in 0u8..=255 {
        let mut buf = BytesMut::from(&[type_byte, 0xFF, 0xFF, 0xFF, 0x7F][..]);
        let decoded = Packet::decode_with_limit(&mut buf, MqttVersion::V5, LIMIT);
        assert!(
            !matches!(decoded, Ok(Some(_))),
            "cannot decode a packet from a header alone"
        );
        assert!(
            buf.capacity() < 1 << 20,
            "must not reserve the announced size"
        );
    }
}
