//! CONNECT / CONNACK packets (MQTT-3.1.1 §3.1, §3.2; MQTT-5.0 §3.1, §3.2).

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::properties::Properties;
use super::varint::{decode_binary, decode_utf8_string, encode_binary, encode_utf8_string};
use super::{MqttVersion, QoS};
use crate::error::{MqttError, MqttResult};

/// An MQTT Will message, published by the broker if the client disconnects
/// unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Will {
    pub topic: String,
    pub payload: Bytes,
    pub qos: QoS,
    pub retain: bool,
    /// MQTT 5.0 only: properties attached to the will message.
    pub properties: Properties,
    /// MQTT 5.0 only: seconds to delay publishing the will after disconnect.
    pub delay_interval: u32,
}

/// A CONNECT packet.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectPacket {
    pub version: MqttVersion,
    pub client_id: String,
    pub clean_start: bool,
    pub keep_alive: u16,
    pub username: Option<String>,
    pub password: Option<Bytes>,
    pub will: Option<Will>,
    pub properties: Properties,
}

// Hand-rolled `Debug` (via the macro) so `{:?}` on a CONNECT never prints the
// password — this packet is `Debug`-formatted in error paths such as
// "expected CONNECT, got ...".
crate::redacted_debug!(ConnectPacket {
    version, client_id, clean_start, keep_alive, username, will, properties
} secret { password });

impl ConnectPacket {
    pub fn encode_body(&self, out: &mut BytesMut) -> MqttResult<()> {
        encode_utf8_string("MQTT", out)?;
        out.put_u8(self.version.protocol_level());

        let mut flags = 0u8;
        if self.clean_start {
            flags |= 0x02;
        }
        if let Some(will) = &self.will {
            flags |= 0x04;
            flags |= (will.qos.as_u8() & 0x03) << 3;
            if will.retain {
                flags |= 0x20;
            }
        }
        if self.username.is_some() {
            flags |= 0x80;
        }
        if self.password.is_some() {
            flags |= 0x40;
        }
        out.put_u8(flags);
        out.put_u16(self.keep_alive);

        if self.version.is_v5() {
            self.properties.encode(out)?;
        }

        encode_utf8_string(&self.client_id, out)?;

        if let Some(will) = &self.will {
            if self.version.is_v5() {
                will.properties.encode(out)?;
            }
            encode_utf8_string(&will.topic, out)?;
            encode_binary(&will.payload, out)?;
        }
        if let Some(username) = &self.username {
            encode_utf8_string(username, out)?;
        }
        if let Some(password) = &self.password {
            encode_binary(password, out)?;
        }
        Ok(())
    }

    pub fn decode_body(buf: &mut Bytes) -> MqttResult<Self> {
        let proto_name = decode_utf8_string(buf)?;
        if proto_name != "MQTT" {
            return Err(MqttError::MalformedPacket(format!(
                "unexpected protocol name {proto_name:?}"
            )));
        }
        if buf.remaining() < 1 {
            return Err(MqttError::MalformedPacket(
                "truncated protocol level".into(),
            ));
        }
        let level = buf.get_u8();
        let version = MqttVersion::from_protocol_level(level).ok_or_else(|| {
            MqttError::MalformedPacket(format!("unsupported protocol level {level}"))
        })?;

        if buf.remaining() < 3 {
            return Err(MqttError::MalformedPacket("truncated connect flags".into()));
        }
        let flags = buf.get_u8();
        let clean_start = flags & 0x02 != 0;
        let will_flag = flags & 0x04 != 0;
        let will_qos = QoS::from_u8((flags >> 3) & 0x03)
            .ok_or_else(|| MqttError::MalformedPacket("invalid will QoS".into()))?;
        let will_retain = flags & 0x20 != 0;
        let has_password = flags & 0x40 != 0;
        let has_username = flags & 0x80 != 0;
        let keep_alive = buf.get_u16();

        let properties = if version.is_v5() {
            Properties::decode(buf)?
        } else {
            Properties::new()
        };

        let client_id = decode_utf8_string(buf)?;

        let will = if will_flag {
            let will_properties = if version.is_v5() {
                Properties::decode(buf)?
            } else {
                Properties::new()
            };
            let topic = decode_utf8_string(buf)?;
            let payload = decode_binary(buf)?;
            let delay_interval = will_properties
                .0
                .iter()
                .find_map(|p| match p {
                    super::properties::Property::WillDelayInterval(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(0);
            Some(Will {
                topic,
                payload,
                qos: will_qos,
                retain: will_retain,
                properties: will_properties,
                delay_interval,
            })
        } else {
            None
        };

        let username = if has_username {
            Some(decode_utf8_string(buf)?)
        } else {
            None
        };
        let password = if has_password {
            Some(decode_binary(buf)?)
        } else {
            None
        };

        Ok(ConnectPacket {
            version,
            client_id,
            clean_start,
            keep_alive,
            username,
            password,
            will,
            properties,
        })
    }
}

/// CONNACK return/reason code. MQTT 3.1.1 defines 0-5; MQTT 5.0 defines a
/// much larger set. Stored as the raw byte plus a human-readable helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectReasonCode(pub u8);

impl ConnectReasonCode {
    pub const SUCCESS: ConnectReasonCode = ConnectReasonCode(0x00);
    pub const UNSPECIFIED_ERROR: ConnectReasonCode = ConnectReasonCode(0x80);
    pub const MALFORMED_PACKET: ConnectReasonCode = ConnectReasonCode(0x81);
    pub const NOT_AUTHORIZED: ConnectReasonCode = ConnectReasonCode(0x87);
    pub const BAD_USERNAME_OR_PASSWORD: ConnectReasonCode = ConnectReasonCode(0x86);
    pub const CLIENT_IDENTIFIER_NOT_VALID: ConnectReasonCode = ConnectReasonCode(0x85);
    pub const UNSUPPORTED_PROTOCOL_VERSION: ConnectReasonCode = ConnectReasonCode(0x84);
    pub const SERVER_UNAVAILABLE: ConnectReasonCode = ConnectReasonCode(0x88);
    pub const QUOTA_EXCEEDED: ConnectReasonCode = ConnectReasonCode(0x97);

    pub fn is_success(self) -> bool {
        self.0 == 0
    }

    /// Map a 3.1.1-style return code (0-5) onto the equivalent 5.0 reason
    /// code, so callers only need to reason about one type.
    pub fn from_v311_return_code(code: u8) -> Self {
        match code {
            0 => Self::SUCCESS,
            1 => ConnectReasonCode(0x84), // unacceptable protocol version
            2 => Self::CLIENT_IDENTIFIER_NOT_VALID,
            3 => ConnectReasonCode(0x88), // server unavailable
            4 => Self::BAD_USERNAME_OR_PASSWORD,
            5 => Self::NOT_AUTHORIZED,
            _ => Self::UNSPECIFIED_ERROR,
        }
    }

    /// Collapse a 5.0 reason code down to the nearest 3.1.1 return code, for
    /// interop when a v3.1.1 client connects to the shared broker logic.
    pub fn to_v311_return_code(self) -> u8 {
        match self.0 {
            0x00 => 0,
            0x84 => 1,
            0x85 => 2,
            // server unavailable / server busy / quota exceeded
            0x88 | 0x89 | 0x97 => 3,
            0x86 => 4,
            0x87 => 5,
            _ => 5,
        }
    }
}

/// A CONNACK packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnAckPacket {
    pub session_present: bool,
    pub reason_code: ConnectReasonCode,
    pub properties: Properties,
}

impl ConnAckPacket {
    pub fn encode_body(&self, version: MqttVersion, out: &mut BytesMut) -> MqttResult<()> {
        out.put_u8(if self.session_present { 0x01 } else { 0x00 });
        let code = if version.is_v5() {
            self.reason_code.0
        } else {
            self.reason_code.to_v311_return_code()
        };
        out.put_u8(code);
        if version.is_v5() {
            self.properties.encode(out)?;
        }
        Ok(())
    }

    pub fn decode_body(version: MqttVersion, buf: &mut Bytes) -> MqttResult<Self> {
        if buf.remaining() < 2 {
            return Err(MqttError::MalformedPacket("truncated CONNACK".into()));
        }
        let flags = buf.get_u8();
        let session_present = flags & 0x01 != 0;
        let raw_code = buf.get_u8();
        let reason_code = if version.is_v5() {
            ConnectReasonCode(raw_code)
        } else {
            ConnectReasonCode::from_v311_return_code(raw_code)
        };
        let properties = if version.is_v5() {
            Properties::decode(buf)?
        } else {
            Properties::new()
        };
        Ok(ConnAckPacket {
            session_present,
            reason_code,
            properties,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connect(version: MqttVersion) -> ConnectPacket {
        ConnectPacket {
            version,
            client_id: "client-1".into(),
            clean_start: true,
            keep_alive: 60,
            username: Some("alice".into()),
            password: Some(Bytes::from_static(b"secret")),
            will: Some(Will {
                topic: "clients/client-1/status".into(),
                payload: Bytes::from_static(b"offline"),
                qos: QoS::AtLeastOnce,
                retain: true,
                properties: Properties::new(),
                delay_interval: 0,
            }),
            properties: Properties::new(),
        }
    }

    #[test]
    fn connect_v311_roundtrip() {
        let pkt = sample_connect(MqttVersion::V311);
        let mut out = BytesMut::new();
        pkt.encode_body(&mut out).unwrap();
        let mut bytes = out.freeze();
        let decoded = ConnectPacket::decode_body(&mut bytes).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn connect_v5_roundtrip_with_properties() {
        let mut pkt = sample_connect(MqttVersion::V5);
        pkt.properties
            .push(super::super::properties::Property::SessionExpiryInterval(
                3600,
            ));
        let mut out = BytesMut::new();
        pkt.encode_body(&mut out).unwrap();
        let mut bytes = out.freeze();
        let decoded = ConnectPacket::decode_body(&mut bytes).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn connack_v311_roundtrip() {
        let ack = ConnAckPacket {
            session_present: true,
            reason_code: ConnectReasonCode::SUCCESS,
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V311, &mut out).unwrap();
        let mut bytes = out.freeze();
        let decoded = ConnAckPacket::decode_body(MqttVersion::V311, &mut bytes).unwrap();
        assert_eq!(decoded, ack);
    }

    #[test]
    fn connack_v5_roundtrip() {
        let ack = ConnAckPacket {
            session_present: false,
            reason_code: ConnectReasonCode::NOT_AUTHORIZED,
            properties: Properties::new(),
        };
        let mut out = BytesMut::new();
        ack.encode_body(MqttVersion::V5, &mut out).unwrap();
        let mut bytes = out.freeze();
        let decoded = ConnAckPacket::decode_body(MqttVersion::V5, &mut bytes).unwrap();
        assert_eq!(decoded, ack);
    }
}
