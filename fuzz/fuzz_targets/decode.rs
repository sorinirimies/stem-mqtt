//! Coverage-guided fuzzing of the wire decoder:
//!
//! ```sh
//! cargo install cargo-fuzz
//! cargo +nightly fuzz run decode        # from the repo root's fuzz/ directory
//! ```
//!
//! The always-on, no-nightly counterpart is `crates/mqtt-client/tests/decode_robustness.rs`.
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::MqttVersion;

fuzz_target!(|data: &[u8]| {
    for version in [MqttVersion::V311, MqttVersion::V5] {
        let mut buf = BytesMut::from(data);
        // Drain every packet the input contains; any panic is a finding.
        while let Ok(Some(packet)) = Packet::decode_with_limit(&mut buf, version, 1 << 20) {
            let _ = packet.encode(version);
        }
    }
});
