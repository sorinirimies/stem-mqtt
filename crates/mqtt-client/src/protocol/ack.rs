//! PUBACK / PUBREC / PUBREL / PUBCOMP packets. All four share an identical
//! wire shape (MQTT-5.0 §3.4, §3.5, §3.6, §3.7): a packet id, and — in MQTT
//! 5.0, when there's anything interesting to say — a reason code and
//! properties.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::properties::Properties;
use super::MqttVersion;
use crate::error::MqttError;

/// Common body of PUBACK, PUBREC, PUBREL and PUBCOMP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleAck {
    pub packet_id: u16,
    /// `0x00` (Success) unless MQTT 5.0 reports a specific outcome.
    pub reason_code: u8,
    pub properties: Properties,
}

impl SimpleAck {
    pub fn success(packet_id: u16) -> Self {
        SimpleAck {
            packet_id,
            reason_code: 0,
            properties: Properties::new(),
        }
    }

    pub fn encode_body(
        &self,
        version: MqttVersion,
        out: &mut BytesMut,
    ) -> crate::error::MqttResult<()> {
        out.put_u16(self.packet_id);
        if version.is_v5() && (self.reason_code != 0 || !self.properties.0.is_empty()) {
            out.put_u8(self.reason_code);
            self.properties.encode(out)?;
        }
        Ok(())
    }

    pub fn encoded_len(&self, version: MqttVersion) -> usize {
        if version.is_v5() && (self.reason_code != 0 || !self.properties.0.is_empty()) {
            2 + 1 + self.properties.encoded_len()
        } else {
            2
        }
    }

    pub fn decode_body(version: MqttVersion, mut buf: Bytes) -> crate::error::MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket("truncated ack packet id".into()));
        }
        let packet_id = buf.get_u16();
        if !version.is_v5() || !buf.has_remaining() {
            return Ok(SimpleAck {
                packet_id,
                reason_code: 0,
                properties: Properties::new(),
            });
        }
        let reason_code = buf.get_u8();
        let properties = if buf.has_remaining() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        Ok(SimpleAck {
            packet_id,
            reason_code,
            properties,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_ack_v311_is_two_bytes() {
        let ack = SimpleAck::success(7);
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V311, &mut out).unwrap();
        assert_eq!(&out[..], &[0x00, 0x07]);
        let decoded = SimpleAck::decode_body(MqttVersion::V311, out.freeze()).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn success_ack_v5_is_still_two_bytes() {
        let ack = SimpleAck::success(1);
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V5, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        let decoded = SimpleAck::decode_body(MqttVersion::V5, out.freeze()).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn error_ack_v5_roundtrip_with_reason_string() {
        let mut ack = SimpleAck {
            packet_id: 99,
            reason_code: 0x80,
            properties: Properties::new(),
        };
        ack.properties
            .push(super::super::properties::Property::ReasonString(
                "nope".into(),
            ));
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V5, &mut out).unwrap();
        assert_eq!(out.len(), ack.encoded_len(MqttVersion::V5));
        let decoded = SimpleAck::decode_body(MqttVersion::V5, out.freeze()).unwrap();
        assert_eq!(decoded, ack);
    }
}
