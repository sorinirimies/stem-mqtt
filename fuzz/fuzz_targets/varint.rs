//! Variable-byte-integer and length-prefixed string/binary primitives.
#![no_main]

use bytes::{Bytes, BytesMut};
use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::varint::{
    decode_binary, decode_utf8_string, decode_varint, encode_binary, encode_utf8_string,
    encode_varint, peek_varint, varint_len,
};

fuzz_target!(|input: (u32, &str, Vec<u8>, Vec<u8>)| {
    let (value, text, blob, raw) = input;

    // Encode → decode identity, and the length helper agrees with the encoder.
    let mut out = BytesMut::new();
    if encode_varint(value, &mut out).is_ok() {
        assert_eq!(out.len(), varint_len(value));
        assert_eq!(peek_varint(&out).unwrap(), Some((value, out.len())));
        let mut bytes = out.freeze();
        assert_eq!(decode_varint(&mut bytes).unwrap(), Some(value));
        assert!(bytes.is_empty());
    }

    let mut out = BytesMut::new();
    if encode_utf8_string(text, &mut out).is_ok() {
        let mut bytes = out.freeze();
        assert_eq!(decode_utf8_string(&mut bytes).unwrap(), text);
    }
    let mut out = BytesMut::new();
    if encode_binary(&blob, &mut out).is_ok() {
        let mut bytes = out.freeze();
        assert_eq!(&decode_binary(&mut bytes).unwrap()[..], &blob[..]);
    }

    // Arbitrary bytes must never panic any decoder.
    let _ = peek_varint(&raw);
    let _ = decode_varint(&mut Bytes::from(raw.clone()));
    let _ = decode_utf8_string(&mut Bytes::from(raw.clone()));
    let _ = decode_binary(&mut Bytes::from(raw));
});
