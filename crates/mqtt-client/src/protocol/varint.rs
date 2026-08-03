//! Wire-format primitives shared by every MQTT packet: the Variable Byte
//! Integer encoding, length-prefixed UTF-8 strings, and length-prefixed
//! binary data, as defined by the MQTT 3.1.1 and MQTT 5.0 specifications.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::{MqttError, MqttResult};

/// Maximum value representable by a 4-byte Variable Byte Integer.
pub const MAX_VARINT: u32 = 268_435_455;

/// Encode `value` as an MQTT Variable Byte Integer (1-4 bytes, 7 bits of
/// payload per byte, continuation bit in the MSB).
pub fn encode_varint(mut value: u32, out: &mut BytesMut) -> MqttResult<()> {
    if value > MAX_VARINT {
        return Err(MqttError::Protocol(format!(
            "value {value} exceeds maximum variable byte integer {MAX_VARINT}"
        )));
    }
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.put_u8(byte);
        if value == 0 {
            break;
        }
    }
    Ok(())
}

/// Decode an MQTT Variable Byte Integer from the front of `buf`, returning
/// the decoded value. Returns `Ok(None)` if `buf` does not yet contain a
/// complete encoding (caller should read more bytes and retry).
pub fn decode_varint(buf: &mut Bytes) -> MqttResult<Option<u32>> {
    let mut multiplier: u32 = 1;
    let mut value: u32 = 0;
    let mut consumed = 0usize;
    let snapshot = buf.clone();
    loop {
        if !buf.has_remaining() {
            *buf = snapshot;
            return Ok(None);
        }
        let byte = buf.get_u8();
        consumed += 1;
        value += (byte & 0x7F) as u32 * multiplier;
        if multiplier > 128 * 128 * 128 {
            return Err(MqttError::Protocol(
                "malformed variable byte integer".into(),
            ));
        }
        multiplier *= 128;
        if byte & 0x80 == 0 {
            break;
        }
        if consumed > 4 {
            return Err(MqttError::Protocol("variable byte integer too long".into()));
        }
    }
    Ok(Some(value))
}

/// Number of bytes required to encode `value` as a Variable Byte Integer.
pub fn varint_len(value: u32) -> usize {
    match value {
        0..=127 => 1,
        128..=16_383 => 2,
        16_384..=2_097_151 => 3,
        _ => 4,
    }
}

/// Encode a length-prefixed UTF-8 string (2-byte big-endian length prefix).
pub fn encode_utf8_string(s: &str, out: &mut BytesMut) -> MqttResult<()> {
    if s.len() > u16::MAX as usize {
        return Err(MqttError::Protocol(
            "string too long for MQTT encoding".into(),
        ));
    }
    out.put_u16(s.len() as u16);
    out.put_slice(s.as_bytes());
    Ok(())
}

/// Decode a length-prefixed UTF-8 string from the front of `buf`.
pub fn decode_utf8_string(buf: &mut Bytes) -> MqttResult<String> {
    if buf.remaining() < 2 {
        return Err(MqttError::MalformedPacket("truncated string length".into()));
    }
    let len = buf.get_u16() as usize;
    if buf.remaining() < len {
        return Err(MqttError::MalformedPacket("truncated string body".into()));
    }
    let raw = buf.copy_to_bytes(len);
    String::from_utf8(raw.to_vec())
        .map_err(|e| MqttError::MalformedPacket(format!("invalid UTF-8 string: {e}")))
}

/// Encode length-prefixed binary data (2-byte big-endian length prefix).
pub fn encode_binary(data: &[u8], out: &mut BytesMut) -> MqttResult<()> {
    if data.len() > u16::MAX as usize {
        return Err(MqttError::Protocol(
            "binary data too long for MQTT encoding".into(),
        ));
    }
    out.put_u16(data.len() as u16);
    out.put_slice(data);
    Ok(())
}

/// Decode length-prefixed binary data from the front of `buf`.
pub fn decode_binary(buf: &mut Bytes) -> MqttResult<Bytes> {
    if buf.remaining() < 2 {
        return Err(MqttError::MalformedPacket("truncated binary length".into()));
    }
    let len = buf.get_u16() as usize;
    if buf.remaining() < len {
        return Err(MqttError::MalformedPacket("truncated binary body".into()));
    }
    Ok(buf.copy_to_bytes(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_boundaries() {
        for value in [
            0u32, 1, 127, 128, 16_383, 16_384, 2_097_151, 2_097_152, MAX_VARINT,
        ] {
            let mut out = BytesMut::new();
            encode_varint(value, &mut out).unwrap();
            assert_eq!(out.len(), varint_len(value));
            let mut bytes = out.freeze();
            let decoded = decode_varint(&mut bytes).unwrap().unwrap();
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn varint_rejects_overflow() {
        let mut out = BytesMut::new();
        assert!(encode_varint(MAX_VARINT + 1, &mut out).is_err());
    }

    #[test]
    fn varint_incomplete_returns_none() {
        let mut buf = Bytes::from_static(&[0x80]);
        assert_eq!(decode_varint(&mut buf).unwrap(), None);
    }

    #[test]
    fn utf8_string_roundtrip() {
        let mut out = BytesMut::new();
        encode_utf8_string("hello/mqtt", &mut out).unwrap();
        let mut bytes = out.freeze();
        assert_eq!(decode_utf8_string(&mut bytes).unwrap(), "hello/mqtt");
    }

    #[test]
    fn binary_roundtrip() {
        let mut out = BytesMut::new();
        encode_binary(&[1, 2, 3, 4], &mut out).unwrap();
        let mut bytes = out.freeze();
        assert_eq!(&decode_binary(&mut bytes).unwrap()[..], &[1, 2, 3, 4]);
    }
}
