//! Top-level [`Packet`] enum: the fixed header, remaining-length framing,
//! and dispatch to each packet type's body codec.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::ack::SimpleAck;
use super::connect::{ConnAckPacket, ConnectPacket};
use super::macros::u8_enum;
use super::properties::Properties;
use super::publish::PublishPacket;
use super::subscribe::{SubAckPacket, SubscribePacket, UnsubAckPacket, UnsubscribePacket};
use super::varint::{decode_varint, encode_varint, MAX_VARINT};
use super::MqttVersion;
use crate::error::{MqttError, MqttResult};

// `u8_enum!` (see `super::macros`) generates both the `#[repr(u8)]`
// discriminants and the `from_u8` decoder from this one id/variant table,
// so the two can never drift apart the way a hand-written enum + a
// hand-written reverse `match` easily can.
u8_enum! {
    /// MQTT control packet type identifiers (MQTT-3.1.1 §2.2.1 / MQTT-5.0 §2.1.2).
    pub enum PacketType, "packet type" {
        Connect = 1,
        ConnAck = 2,
        Publish = 3,
        PubAck = 4,
        PubRec = 5,
        PubRel = 6,
        PubComp = 7,
        Subscribe = 8,
        SubAck = 9,
        Unsubscribe = 10,
        UnsubAck = 11,
        PingReq = 12,
        PingResp = 13,
        Disconnect = 14,
        Auth = 15,
    }
}

/// DISCONNECT packet body (empty in MQTT 3.1.1; reason code + properties in
/// MQTT 5.0, both of which may be omitted when the reason is "normal").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisconnectPacket {
    pub reason_code: u8,
    pub properties: Properties,
}

impl DisconnectPacket {
    pub fn normal() -> Self {
        Self::default()
    }

    fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        if version.is_v5() && (self.reason_code != 0 || !self.properties.0.is_empty()) {
            out.put_u8(self.reason_code);
            self.properties.encode(out)?;
        }
        Ok(())
    }

    fn decode_body(version: MqttVersion, mut buf: Bytes) -> MqttResult<Self> {
        if !version.is_v5() || !buf.has_remaining() {
            return Ok(DisconnectPacket::default());
        }
        let reason_code = buf.get_u8();
        let properties = if buf.has_remaining() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        Ok(DisconnectPacket {
            reason_code,
            properties,
        })
    }
}

/// AUTH packet body (MQTT 5.0 only — enhanced authentication exchange).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthPacket {
    pub reason_code: u8,
    pub properties: Properties,
}

impl AuthPacket {
    fn encode_body(&self, out: &mut BytesMut) -> MqttResult<()> {
        if self.reason_code != 0 || !self.properties.0.is_empty() {
            out.put_u8(self.reason_code);
            self.properties.encode(out)?;
        }
        Ok(())
    }

    fn decode_body(mut buf: Bytes) -> MqttResult<Self> {
        if !buf.has_remaining() {
            return Ok(AuthPacket::default());
        }
        let reason_code = buf.get_u8();
        let properties = if buf.has_remaining() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        Ok(AuthPacket {
            reason_code,
            properties,
        })
    }
}

/// Every possible MQTT control packet, spanning both protocol versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Connect(ConnectPacket),
    ConnAck(ConnAckPacket),
    Publish(PublishPacket),
    PubAck(SimpleAck),
    PubRec(SimpleAck),
    PubRel(SimpleAck),
    PubComp(SimpleAck),
    Subscribe(SubscribePacket),
    SubAck(SubAckPacket),
    Unsubscribe(UnsubscribePacket),
    UnsubAck(UnsubAckPacket),
    PingReq,
    PingResp,
    Disconnect(DisconnectPacket),
    Auth(AuthPacket),
}

impl Packet {
    fn packet_type(&self) -> PacketType {
        match self {
            Packet::Connect(_) => PacketType::Connect,
            Packet::ConnAck(_) => PacketType::ConnAck,
            Packet::Publish(_) => PacketType::Publish,
            Packet::PubAck(_) => PacketType::PubAck,
            Packet::PubRec(_) => PacketType::PubRec,
            Packet::PubRel(_) => PacketType::PubRel,
            Packet::PubComp(_) => PacketType::PubComp,
            Packet::Subscribe(_) => PacketType::Subscribe,
            Packet::SubAck(_) => PacketType::SubAck,
            Packet::Unsubscribe(_) => PacketType::Unsubscribe,
            Packet::UnsubAck(_) => PacketType::UnsubAck,
            Packet::PingReq => PacketType::PingReq,
            Packet::PingResp => PacketType::PingResp,
            Packet::Disconnect(_) => PacketType::Disconnect,
            Packet::Auth(_) => PacketType::Auth,
        }
    }

    /// Fixed-header flags (low nibble of byte 1). Fixed per packet type
    /// except PUBLISH, which encodes DUP/QoS/RETAIN.
    fn header_flags(&self) -> u8 {
        match self {
            Packet::Publish(p) => p.flags(),
            Packet::PubRel(_) | Packet::Subscribe(_) | Packet::Unsubscribe(_) => 0x02,
            _ => 0x00,
        }
    }

    fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        match self {
            Packet::Connect(p) => p.encode_body(out),
            Packet::ConnAck(p) => p.encode_body(version, out),
            Packet::Publish(p) => p.encode_body(version, out),
            Packet::PubAck(p) | Packet::PubRec(p) | Packet::PubRel(p) | Packet::PubComp(p) => {
                p.encode_body(version, out)
            }
            Packet::Subscribe(p) => p.encode_body(version, out),
            Packet::SubAck(p) => p.encode_body(version, out),
            Packet::Unsubscribe(p) => p.encode_body(version, out),
            Packet::UnsubAck(p) => p.encode_body(version, out),
            Packet::PingReq | Packet::PingResp => Ok(()),
            Packet::Disconnect(p) => p.encode_body(version, out),
            Packet::Auth(p) => p.encode_body(out),
        }
    }

    /// Encode this packet (fixed header + body) as a standalone buffer
    /// ready to write to a socket.
    pub fn encode(&self, version: MqttVersion) -> MqttResult<BytesMut> {
        let mut body = BytesMut::new();
        self.encode_body(version, &mut body)?;
        if body.len() as u32 > MAX_VARINT {
            return Err(MqttError::Protocol(
                "packet body exceeds maximum size".into(),
            ));
        }

        let mut out = BytesMut::with_capacity(body.len() + 5);
        let type_byte = ((self.packet_type() as u8) << 4) | (self.header_flags() & 0x0F);
        out.put_u8(type_byte);
        encode_varint(body.len() as u32, &mut out)?;
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Attempt to decode one full packet from the front of `buf`. Returns
    /// `Ok(None)` when `buf` does not yet hold a complete packet — callers
    /// should read more bytes from the socket and retry without consuming
    /// `buf` (this function never partially consumes on an incomplete read).
    ///
    /// `version` is the protocol version already negotiated for this
    /// connection (irrelevant when decoding the initial CONNECT packet,
    /// which carries its own protocol level).
    pub fn decode(buf: &mut BytesMut, version: MqttVersion) -> MqttResult<Option<Packet>> {
        if buf.is_empty() {
            return Ok(None);
        }
        let type_byte = buf[0];
        let packet_type = PacketType::from_u8(type_byte >> 4)?;
        let flags = type_byte & 0x0F;

        // Try to parse the remaining-length varint without committing to
        // consuming `buf` unless we know the full packet is present.
        let mut cursor = Bytes::copy_from_slice(&buf[1..]);
        let remaining_len = match decode_varint(&mut cursor)? {
            Some(len) => len as usize,
            None => return Ok(None),
        };
        let header_len = 1 + (buf.len() - 1 - cursor.len());
        let total_len = header_len + remaining_len;
        if buf.len() < total_len {
            return Ok(None);
        }

        let mut frame = buf.split_to(total_len);
        frame.advance(header_len);
        let body = frame.freeze();

        let packet = match packet_type {
            PacketType::Connect => Packet::Connect(ConnectPacket::decode_body(&mut body.clone())?),
            PacketType::ConnAck => {
                Packet::ConnAck(ConnAckPacket::decode_body(version, &mut body.clone())?)
            }
            PacketType::Publish => {
                Packet::Publish(PublishPacket::decode_body(version, flags, body)?)
            }
            PacketType::PubAck => Packet::PubAck(SimpleAck::decode_body(version, body)?),
            PacketType::PubRec => Packet::PubRec(SimpleAck::decode_body(version, body)?),
            PacketType::PubRel => Packet::PubRel(SimpleAck::decode_body(version, body)?),
            PacketType::PubComp => Packet::PubComp(SimpleAck::decode_body(version, body)?),
            PacketType::Subscribe => {
                Packet::Subscribe(SubscribePacket::decode_body(version, body)?)
            }
            PacketType::SubAck => Packet::SubAck(SubAckPacket::decode_body(version, body)?),
            PacketType::Unsubscribe => {
                Packet::Unsubscribe(UnsubscribePacket::decode_body(version, body)?)
            }
            PacketType::UnsubAck => Packet::UnsubAck(UnsubAckPacket::decode_body(version, body)?),
            PacketType::PingReq => Packet::PingReq,
            PacketType::PingResp => Packet::PingResp,
            PacketType::Disconnect => {
                Packet::Disconnect(DisconnectPacket::decode_body(version, body)?)
            }
            PacketType::Auth => Packet::Auth(AuthPacket::decode_body(body)?),
        };
        Ok(Some(packet))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::QoS;

    #[test]
    fn ping_roundtrip() {
        let out = Packet::PingReq.encode(MqttVersion::V311).unwrap();
        assert_eq!(&out[..], &[0xC0, 0x00]);
        let mut buf = BytesMut::from(&out[..]);
        let decoded = Packet::decode(&mut buf, MqttVersion::V311)
            .unwrap()
            .unwrap();
        assert_eq!(decoded, Packet::PingReq);
        assert!(buf.is_empty());
    }

    #[test]
    fn incomplete_packet_returns_none_and_does_not_consume() {
        let full = Packet::PingResp.encode(MqttVersion::V311).unwrap();
        let mut partial = BytesMut::from(&full[..full.len() - 1]);
        let before = partial.clone();
        let result = Packet::decode(&mut partial, MqttVersion::V311).unwrap();
        assert!(result.is_none());
        assert_eq!(partial, before);
    }

    #[test]
    fn publish_end_to_end_v5() {
        let pkt = Packet::Publish(PublishPacket {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: "topic/x".into(),
            packet_id: Some(7),
            payload: Bytes::from_static(b"payload"),
            properties: Properties::new(),
        });
        let encoded = pkt.encode(MqttVersion::V5).unwrap();
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded = Packet::decode(&mut buf, MqttVersion::V5).unwrap().unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn two_packets_back_to_back() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&Packet::PingReq.encode(MqttVersion::V311).unwrap());
        buf.extend_from_slice(&Packet::PingResp.encode(MqttVersion::V311).unwrap());

        let first = Packet::decode(&mut buf, MqttVersion::V311)
            .unwrap()
            .unwrap();
        assert_eq!(first, Packet::PingReq);
        let second = Packet::decode(&mut buf, MqttVersion::V311)
            .unwrap()
            .unwrap();
        assert_eq!(second, Packet::PingResp);
        assert!(buf.is_empty());
    }

    #[test]
    fn disconnect_v5_normal_is_empty_body() {
        let pkt = Packet::Disconnect(DisconnectPacket::normal());
        let encoded = pkt.encode(MqttVersion::V5).unwrap();
        assert_eq!(&encoded[..], &[0xE0, 0x00]);
    }

    #[test]
    fn unknown_packet_type_errors() {
        // Packet type 0 (top nibble) is reserved/unused by the spec.
        let mut buf = BytesMut::from(&[0x00u8, 0x00][..]);
        assert!(Packet::decode(&mut buf, MqttVersion::V311).is_err());
    }
}
