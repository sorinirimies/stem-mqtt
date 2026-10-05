//! Top-level [`Packet`] enum: the fixed header, remaining-length framing,
//! and dispatch to each packet type's body codec.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::ack::SimpleAck;
use super::connect::{ConnAckPacket, ConnectPacket};
use super::macros::u8_enum;
use super::properties::Properties;
use super::publish::PublishPacket;
use super::subscribe::{SubAckPacket, SubscribePacket, UnsubAckPacket, UnsubscribePacket};
use super::varint::{encode_varint, peek_varint, MAX_VARINT};
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

/// Largest packet the wire format can express: 1 type byte + a 4-byte
/// remaining-length + a [`MAX_VARINT`]-byte body.
pub const MAX_PACKET_SIZE: usize = 1 + 4 + MAX_VARINT as usize;

impl PacketType {
    /// The fixed-header flag bits this packet type *requires*, or `None`
    /// for PUBLISH (whose flags carry DUP/QoS/RETAIN and are validated by
    /// its own decoder). MQTT-2.2.2-2: a receiver MUST treat any other
    /// value as a malformed packet.
    fn required_flags(self) -> Option<u8> {
        match self {
            PacketType::Publish => None,
            PacketType::PubRel | PacketType::Subscribe | PacketType::Unsubscribe => Some(0x02),
            _ => Some(0x00),
        }
    }

    fn validate_flags(self, flags: u8) -> MqttResult<()> {
        match self.required_flags() {
            Some(required) if required != flags => Err(MqttError::MalformedPacket(format!(
                "invalid fixed-header flags 0x{flags:X} for {self:?} (expected 0x{required:X})"
            ))),
            _ => Ok(()),
        }
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
    /// "Success" — authentication finished (MQTT-5.0 §3.15.2.1).
    pub const SUCCESS: u8 = 0x00;
    /// "Continue authentication" — another challenge/response round follows.
    pub const CONTINUE: u8 = 0x18;

    /// An AUTH packet continuing the exchange for `method` with `data`.
    pub fn continue_with(method: &str, data: Bytes) -> Self {
        AuthPacket {
            reason_code: Self::CONTINUE,
            properties: Properties::with_auth(method, Some(data)),
        }
    }

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
    /// The packet id of the request this packet *answers* — set for the
    /// acknowledgements a client-initiated exchange waits on (PUBACK,
    /// PUBREC, PUBCOMP, SUBACK, UNSUBACK) and `None` for everything else.
    ///
    /// PUBREL is deliberately excluded: it answers a PUBREC *we* sent to
    /// the peer, i.e. it belongs to the receiving side's handshake, not to
    /// a pending request of ours.
    pub fn response_id(&self) -> Option<u16> {
        match self {
            Packet::PubAck(a) | Packet::PubRec(a) | Packet::PubComp(a) => Some(a.packet_id),
            Packet::SubAck(a) => Some(a.packet_id),
            Packet::UnsubAck(a) => Some(a.packet_id),
            _ => None,
        }
    }

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
    ///
    /// Accepts packets up to the protocol maximum; use
    /// [`Packet::decode_with_limit`] for untrusted peers.
    pub fn decode(buf: &mut BytesMut, version: MqttVersion) -> MqttResult<Option<Packet>> {
        Self::decode_with_limit(buf, version, MAX_PACKET_SIZE)
    }

    /// Like [`Packet::decode`], but rejects any packet whose total size
    /// (fixed header + body) exceeds `max_packet_size` as soon as its
    /// header has been read — **before** the body is buffered, so a peer
    /// announcing a 256 MiB packet can't make us allocate it.
    pub fn decode_with_limit(
        buf: &mut BytesMut,
        version: MqttVersion,
        max_packet_size: usize,
    ) -> MqttResult<Option<Packet>> {
        if buf.is_empty() {
            return Ok(None);
        }
        let type_byte = buf[0];
        let packet_type = PacketType::from_u8(type_byte >> 4)?;
        let flags = type_byte & 0x0F;
        packet_type.validate_flags(flags)?;

        // Parse the remaining-length varint in place (no copy of the
        // buffer) and only commit to consuming `buf` once the whole packet
        // is present.
        let (remaining_len, len_bytes) = match peek_varint(&buf[1..])? {
            Some((len, n)) => (len as usize, n),
            None => return Ok(None),
        };
        let header_len = 1 + len_bytes;
        let total_len = header_len + remaining_len;
        if total_len > max_packet_size {
            return Err(MqttError::PacketTooLarge(format!(
                "{total_len} bytes exceeds the {max_packet_size} byte limit"
            )));
        }
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
    fn oversized_packet_rejected_from_header_alone() {
        // PUBLISH header announcing a ~2 MiB body, with none of it received.
        let mut buf = BytesMut::from(&[0x30u8, 0x80, 0x80, 0x80, 0x01][..]);
        let err = Packet::decode_with_limit(&mut buf, MqttVersion::V311, 1024 * 1024)
            .expect_err("limit must be enforced before the body arrives");
        assert!(matches!(err, MqttError::PacketTooLarge(_)), "{err:?}");
    }

    #[test]
    fn packet_at_the_limit_is_accepted() {
        let encoded = Packet::PingReq.encode(MqttVersion::V311).unwrap();
        let mut buf = BytesMut::from(&encoded[..]);
        let limit = encoded.len();
        assert!(
            Packet::decode_with_limit(&mut buf, MqttVersion::V311, limit)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn wrong_fixed_header_flags_rejected() {
        // PUBREL must carry flags 0b0010; 0b0000 is malformed.
        let mut buf = BytesMut::from(&[0x60u8, 0x02, 0x00, 0x01][..]);
        assert!(Packet::decode(&mut buf, MqttVersion::V311).is_err());
        // SUBSCRIBE likewise.
        let mut buf = BytesMut::from(&[0x80u8, 0x00][..]);
        assert!(Packet::decode(&mut buf, MqttVersion::V311).is_err());
        // PINGREQ must have all-zero flags.
        let mut buf = BytesMut::from(&[0xC1u8, 0x00][..]);
        assert!(Packet::decode(&mut buf, MqttVersion::V311).is_err());
    }

    #[test]
    fn auth_continue_roundtrips_with_method_and_data() {
        let pkt = Packet::Auth(AuthPacket::continue_with(
            "SCRAM-SHA-256",
            Bytes::from_static(b"nonce"),
        ));
        let encoded = pkt.encode(MqttVersion::V5).unwrap();
        assert_eq!(encoded[0] >> 4, 15, "AUTH is packet type 15");
        let mut buf = BytesMut::from(&encoded[..]);
        let Packet::Auth(auth) = Packet::decode(&mut buf, MqttVersion::V5).unwrap().unwrap() else {
            panic!("expected AUTH");
        };
        assert_eq!(auth.reason_code, AuthPacket::CONTINUE);
        assert_eq!(auth.properties.auth_method(), Some("SCRAM-SHA-256"));
        assert_eq!(
            auth.properties.auth_data().map(|d| &d[..]),
            Some(&b"nonce"[..])
        );
    }

    #[test]
    fn auth_success_with_no_properties_is_an_empty_body() {
        let pkt = Packet::Auth(AuthPacket::default());
        assert_eq!(&pkt.encode(MqttVersion::V5).unwrap()[..], &[0xF0, 0x00]);
    }

    #[test]
    fn response_id_covers_exactly_the_ack_packets() {
        use crate::protocol::ack::SimpleAck;
        let ack = SimpleAck::success(9);
        assert_eq!(Packet::PubAck(ack.clone()).response_id(), Some(9));
        assert_eq!(Packet::PubRec(ack.clone()).response_id(), Some(9));
        assert_eq!(Packet::PubComp(ack.clone()).response_id(), Some(9));
        assert_eq!(Packet::PubRel(ack).response_id(), None);
        assert_eq!(Packet::PingResp.response_id(), None);
    }

    #[test]
    fn unknown_packet_type_errors() {
        // Packet type 0 (top nibble) is reserved/unused by the spec.
        let mut buf = BytesMut::from(&[0x00u8, 0x00][..]);
        assert!(Packet::decode(&mut buf, MqttVersion::V311).is_err());
    }
}
