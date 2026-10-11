//! The decoder must be insensitive to how the stream is split into reads:
//! feeding bytes in arbitrary chunks yields the same packets as one big read,
//! and never panics or consumes a partial packet.
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::packet::Packet;
use mqtt_client::MqttVersion;

fn drain(buf: &mut BytesMut, version: MqttVersion, out: &mut Vec<Packet>) -> bool {
    loop {
        match Packet::decode_with_limit(buf, version, 1 << 20) {
            Ok(Some(p)) => out.push(p),
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

fuzz_target!(|input: (u8, Vec<u8>, Vec<u8>)| {
    let (v, data, sizes) = input;
    let version = if v & 1 == 0 { MqttVersion::V311 } else { MqttVersion::V5 };

    let mut whole = Vec::new();
    let mut buf = BytesMut::from(&data[..]);
    let whole_ok = drain(&mut buf, version, &mut whole);

    let mut chunked = Vec::new();
    let mut buf = BytesMut::new();
    let mut rest = &data[..];
    let mut sizes = sizes.iter().map(|s| usize::from(*s).max(1)).cycle();
    let mut chunked_ok = true;
    while !rest.is_empty() && chunked_ok {
        let n = sizes.next().unwrap_or(1).min(rest.len());
        buf.extend_from_slice(&rest[..n]);
        rest = &rest[n..];
        chunked_ok = drain(&mut buf, version, &mut chunked);
    }

    // Errors may surface at different offsets, but every packet decoded
    // before one must agree, and a clean run must agree entirely.
    let common = whole.len().min(chunked.len());
    assert_eq!(whole[..common], chunked[..common]);
    if whole_ok && chunked_ok {
        assert_eq!(whole, chunked);
    }
});
