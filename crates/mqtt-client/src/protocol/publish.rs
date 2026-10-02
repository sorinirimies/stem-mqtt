//! PUBLISH packet (MQTT-3.1.1 §3.3; MQTT-5.0 §3.3).

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::properties::Properties;
use super::varint::{decode_utf8_string, encode_utf8_string};
use super::{MqttVersion, QoS};
use crate::error::{MqttError, MqttResult};

/// A PUBLISH packet, in either direction (client->broker or broker->client).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPacket {
    pub dup: bool,
    pub qos: QoS,
    pub retain: bool,
    pub topic: String,
    /// Present iff `qos != AtMostOnce`.
    pub packet_id: Option<u16>,
    pub payload: Bytes,
    pub properties: Properties,
}

impl PublishPacket {
    /// Fixed-header flags byte (bits 3-0) for this PUBLISH.
    pub fn flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.dup {
            flags |= 0x08;
        }
        flags |= (self.qos.as_u8() & 0x03) << 1;
        if self.retain {
            flags |= 0x01;
        }
        flags
    }

    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        encode_utf8_string(&self.topic, out)?;
        if self.qos != QoS::AtMostOnce {
            let id = self.packet_id.ok_or_else(|| {
                MqttError::Protocol("QoS > 0 PUBLISH requires a packet id".into())
            })?;
            out.put_u16(id);
        }
        if version.is_v5() {
            self.properties.encode(out)?;
        }
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, flags: u8, mut buf: Bytes) -> MqttResult<Self> {
        let dup = flags & 0x08 != 0;
        let qos = QoS::from_u8((flags >> 1) & 0x03)
            .ok_or_else(|| MqttError::MalformedPacket("invalid PUBLISH QoS".into()))?;
        let retain = flags & 0x01 != 0;

        let topic = decode_utf8_string(&mut buf)?;
        let packet_id = if qos != QoS::AtMostOnce {
            if buf.remaining() < 2 {
                return Err(MqttError::MalformedPacket(
                    "truncated PUBLISH packet id".into(),
                ));
            }
            match buf.get_u16() {
                // MQTT-2.2.1-3: a packet identifier MUST be non-zero.
                0 => {
                    return Err(MqttError::MalformedPacket(
                        "PUBLISH packet id must be non-zero".into(),
                    ))
                }
                id => Some(id),
            }
        } else {
            None
        };
        let properties = if version.is_v5() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        let payload = buf.copy_to_bytes(buf.remaining());

        Ok(PublishPacket {
            dup,
            qos,
            retain,
            topic,
            packet_id,
            payload,
            properties,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_qos0_v311_roundtrip() {
        let pkt = PublishPacket {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            topic: "a/b".into(),
            packet_id: None,
            payload: Bytes::from_static(b"hello"),
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V311, &mut out).unwrap();
        let decoded =
            PublishPacket::decode_body(MqttVersion::V311, pkt.flags(), out.freeze()).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn publish_qos2_v5_roundtrip_with_properties() {
        let mut properties = Properties::new();
        properties.push(super::super::properties::Property::ContentType(
            "text/plain".into(),
        ));
        let pkt = PublishPacket {
            dup: true,
            qos: QoS::ExactlyOnce,
            retain: true,
            topic: "sensors/temp".into(),
            packet_id: Some(42),
            payload: Bytes::from_static(b"23.5"),
            properties,
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V5, &mut out).unwrap();
        let decoded =
            PublishPacket::decode_body(MqttVersion::V5, pkt.flags(), out.freeze()).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn publish_decode_rejects_zero_packet_id() {
        // topic "a", packet id 0, no payload
        let body = Bytes::from_static(&[0x00, 0x01, b'a', 0x00, 0x00]);
        let flags = QoS::AtLeastOnce.as_u8() << 1;
        assert!(PublishPacket::decode_body(MqttVersion::V311, flags, body).is_err());
    }

    #[test]
    fn publish_qos1_requires_packet_id() {
        let pkt = PublishPacket {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: "x".into(),
            packet_id: None,
            payload: Bytes::new(),
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        assert!(pkt.encode_body(MqttVersion::V311, &mut out).is_err());
    }
}
