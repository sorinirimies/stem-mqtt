//! SUBSCRIBE / SUBACK / UNSUBSCRIBE / UNSUBACK packets (MQTT-3.1.1 §3.8-§3.11;
//! MQTT-5.0 §3.8-§3.11).

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::properties::Properties;
use super::varint::{decode_utf8_string, encode_utf8_string};
use super::{MqttVersion, QoS};
use crate::error::{MqttError, MqttResult};

/// How the broker should handle retained messages when a subscription is
/// established (MQTT 5.0 only; ignored for MQTT 3.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainHandling {
    /// Send retained messages at the time of the subscribe.
    SendAtSubscribe,
    /// Send retained messages only if the subscription didn't already exist.
    SendIfNewSubscription,
    /// Never send retained messages for this subscription.
    DoNotSend,
}

impl RetainHandling {
    fn as_u8(self) -> u8 {
        match self {
            RetainHandling::SendAtSubscribe => 0,
            RetainHandling::SendIfNewSubscription => 1,
            RetainHandling::DoNotSend => 2,
        }
    }

    fn from_u8(v: u8) -> MqttResult<Self> {
        match v {
            0 => Ok(RetainHandling::SendAtSubscribe),
            1 => Ok(RetainHandling::SendIfNewSubscription),
            2 => Ok(RetainHandling::DoNotSend),
            other => Err(MqttError::MalformedPacket(format!(
                "invalid retain handling {other}"
            ))),
        }
    }
}

/// One topic filter + its subscribe options within a SUBSCRIBE packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeFilter {
    pub topic_filter: String,
    pub qos: QoS,
    /// MQTT 5.0: don't echo back the subscriber's own publishes.
    pub no_local: bool,
    /// MQTT 5.0: preserve the RETAIN flag on forwarded messages.
    pub retain_as_published: bool,
    pub retain_handling: RetainHandling,
}

impl SubscribeFilter {
    pub fn new(topic_filter: impl Into<String>, qos: QoS) -> Self {
        SubscribeFilter {
            topic_filter: topic_filter.into(),
            qos,
            no_local: false,
            retain_as_published: false,
            retain_handling: RetainHandling::SendAtSubscribe,
        }
    }

    fn options_byte(&self) -> u8 {
        let mut byte = self.qos.as_u8() & 0x03;
        if self.no_local {
            byte |= 0x04;
        }
        if self.retain_as_published {
            byte |= 0x08;
        }
        byte |= self.retain_handling.as_u8() << 4;
        byte
    }
}

/// A SUBSCRIBE packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribePacket {
    pub packet_id: u16,
    pub filters: Vec<SubscribeFilter>,
    pub properties: Properties,
}

impl SubscribePacket {
    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        out.put_u16(self.packet_id);
        if version.is_v5() {
            self.properties.encode(out)?;
        }
        for filter in &self.filters {
            encode_utf8_string(&filter.topic_filter, out)?;
            out.put_u8(filter.options_byte());
        }
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, mut buf: Bytes) -> MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket(
                "truncated SUBSCRIBE packet id".into(),
            ));
        }
        let packet_id = buf.get_u16();
        let properties = if version.is_v5() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        let mut filters = Vec::new();
        while buf.has_remaining() {
            let topic_filter = decode_utf8_string(&mut buf)?;
            if !buf.has_remaining() {
                return Err(MqttError::MalformedPacket(
                    "truncated subscribe options".into(),
                ));
            }
            let options = buf.get_u8();
            let qos = QoS::from_u8(options & 0x03)
                .ok_or_else(|| MqttError::MalformedPacket("invalid subscribe QoS".into()))?;
            filters.push(SubscribeFilter {
                topic_filter,
                qos,
                no_local: options & 0x04 != 0,
                retain_as_published: options & 0x08 != 0,
                retain_handling: RetainHandling::from_u8((options >> 4) & 0x03)?,
            });
        }
        if filters.is_empty() {
            return Err(MqttError::Protocol(
                "SUBSCRIBE must contain at least one filter".into(),
            ));
        }
        Ok(SubscribePacket {
            packet_id,
            filters,
            properties,
        })
    }
}

/// SUBACK per-filter result code. MQTT 3.1.1 uses 0/1/2 (granted QoS) or
/// 0x80 (failure); MQTT 5.0 defines a larger reason-code set that is a
/// superset of the 3.1.1 codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubAckReasonCode(pub u8);

impl SubAckReasonCode {
    pub fn granted(qos: QoS) -> Self {
        SubAckReasonCode(qos.as_u8())
    }

    pub const FAILURE: SubAckReasonCode = SubAckReasonCode(0x80);

    pub fn is_success(self) -> bool {
        self.0 < 0x80
    }
}

/// A SUBACK packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAckPacket {
    pub packet_id: u16,
    pub reason_codes: Vec<SubAckReasonCode>,
    pub properties: Properties,
}

impl SubAckPacket {
    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        out.put_u16(self.packet_id);
        if version.is_v5() {
            self.properties.encode(out)?;
        }
        for code in &self.reason_codes {
            out.put_u8(code.0);
        }
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, mut buf: Bytes) -> MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket(
                "truncated SUBACK packet id".into(),
            ));
        }
        let packet_id = buf.get_u16();
        let properties = if version.is_v5() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        let mut reason_codes = Vec::new();
        while buf.has_remaining() {
            reason_codes.push(SubAckReasonCode(buf.get_u8()));
        }
        Ok(SubAckPacket {
            packet_id,
            reason_codes,
            properties,
        })
    }
}

/// An UNSUBSCRIBE packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubscribePacket {
    pub packet_id: u16,
    pub topic_filters: Vec<String>,
    pub properties: Properties,
}

impl UnsubscribePacket {
    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        out.put_u16(self.packet_id);
        if version.is_v5() {
            self.properties.encode(out)?;
        }
        for filter in &self.topic_filters {
            encode_utf8_string(filter, out)?;
        }
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, mut buf: Bytes) -> MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket(
                "truncated UNSUBSCRIBE packet id".into(),
            ));
        }
        let packet_id = buf.get_u16();
        let properties = if version.is_v5() {
            Properties::decode(&mut buf)?
        } else {
            Properties::new()
        };
        let mut topic_filters = Vec::new();
        while buf.has_remaining() {
            topic_filters.push(decode_utf8_string(&mut buf)?);
        }
        if topic_filters.is_empty() {
            return Err(MqttError::Protocol(
                "UNSUBSCRIBE must contain at least one filter".into(),
            ));
        }
        Ok(UnsubscribePacket {
            packet_id,
            topic_filters,
            properties,
        })
    }
}

/// An UNSUBACK packet. Carries no per-filter reason codes in MQTT 3.1.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubAckPacket {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Properties,
}

impl UnsubAckPacket {
    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        out.put_u16(self.packet_id);
        if version.is_v5() {
            self.properties.encode(out)?;
            for code in &self.reason_codes {
                out.put_u8(*code);
            }
        }
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, mut buf: Bytes) -> MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket(
                "truncated UNSUBACK packet id".into(),
            ));
        }
        let packet_id = buf.get_u16();
        if !version.is_v5() {
            return Ok(UnsubAckPacket {
                packet_id,
                reason_codes: Vec::new(),
                properties: Properties::new(),
            });
        }
        let properties = Properties::decode(&mut buf)?;
        let mut reason_codes = Vec::new();
        while buf.has_remaining() {
            reason_codes.push(buf.get_u8());
        }
        Ok(UnsubAckPacket {
            packet_id,
            reason_codes,
            properties,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_v311_roundtrip() {
        let pkt = SubscribePacket {
            packet_id: 10,
            filters: vec![
                SubscribeFilter::new("a/#", QoS::AtLeastOnce),
                SubscribeFilter::new("b/+/c", QoS::ExactlyOnce),
            ],
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V311, &mut out).unwrap();
        let decoded = SubscribePacket::decode_body(MqttVersion::V311, out.freeze()).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn subscribe_v5_roundtrip_with_options() {
        let pkt = SubscribePacket {
            packet_id: 11,
            filters: vec![SubscribeFilter {
                topic_filter: "shared/topic".into(),
                qos: QoS::AtLeastOnce,
                no_local: true,
                retain_as_published: true,
                retain_handling: RetainHandling::DoNotSend,
            }],
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V5, &mut out).unwrap();
        let decoded = SubscribePacket::decode_body(MqttVersion::V5, out.freeze()).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn suback_roundtrip() {
        let pkt = SubAckPacket {
            packet_id: 10,
            reason_codes: vec![
                SubAckReasonCode::granted(QoS::AtLeastOnce),
                SubAckReasonCode::FAILURE,
            ],
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V5, &mut out).unwrap();
        let decoded = SubAckPacket::decode_body(MqttVersion::V5, out.freeze()).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn unsubscribe_unsuback_roundtrip() {
        let pkt = UnsubscribePacket {
            packet_id: 5,
            topic_filters: vec!["a/b".into(), "c/d".into()],
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        pkt.encode_body(MqttVersion::V311, &mut out).unwrap();
        let decoded = UnsubscribePacket::decode_body(MqttVersion::V311, out.freeze()).unwrap();
        assert_eq!(decoded, pkt);

        let ack = UnsubAckPacket {
            packet_id: 5,
            reason_codes: vec![0, 0x11],
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V5, &mut out).unwrap();
        let decoded = UnsubAckPacket::decode_body(MqttVersion::V5, out.freeze()).unwrap();
        assert_eq!(decoded, ack);
    }
}
