//! Anything the decoder accepts must reach a fixed point under
//! encode → decode → encode: the codec's two halves must agree.
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::MqttVersion;

fuzz_target!(|data: &[u8]| {
    for version in [MqttVersion::V311, MqttVersion::V5] {
        let mut buf = BytesMut::from(data);
        let Ok(Some(packet)) = Packet::decode_with_limit(&mut buf, version, 1 << 20) else {
            continue;
        };
        let Ok(encoded) = packet.encode(version) else {
            continue;
        };
        // Lenient decoding may normalise (e.g. reserved 3.1.1 CONNACK codes),
        // so compare bytes, not structs: re-decoding our own output must
        // succeed and re-encode to exactly the same bytes (a fixed point).
        let mut again = BytesMut::from(&encoded[..]);
        let decoded = Packet::decode_with_limit(&mut again, version, 1 << 21)
            .expect("re-decoding our own encoding must not error")
            .expect("our own encoding is a complete packet");
        assert!(again.is_empty(), "encoder emitted trailing bytes");
        let re_encoded = decoded.encode(version).expect("fixed point must encode");
        assert_eq!(encoded, re_encoded, "encode/decode not idempotent ({version:?})");
    }
});
