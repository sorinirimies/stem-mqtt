//! MQTT wire protocol: packet encoding/decoding for both MQTT 3.1.1 and
//! MQTT 5.0.
//!
//! This module has no networking or async dependencies — it is a pure,
//! synchronous codec so it can be unit-tested byte-for-byte and reused
//! by both the client's connection loop and the `mqtt-broker` crate.

pub mod ack;
pub mod connect;
mod macros;
pub mod packet;
pub mod properties;
pub mod publish;
pub mod subscribe;
pub mod topic;
pub mod varint;

pub use ack::*;
pub use connect::*;
pub use packet::*;
pub use properties::*;
pub use publish::*;
pub use subscribe::*;

use uniffi::Enum;

/// The MQTT protocol version negotiated for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Enum)]
pub enum MqttVersion {
    /// MQTT 3.1.1 (protocol level 4).
    V311,
    /// MQTT 5.0 (protocol level 5).
    V5,
}

impl MqttVersion {
    pub fn protocol_level(self) -> u8 {
        match self {
            MqttVersion::V311 => 4,
            MqttVersion::V5 => 5,
        }
    }

    pub fn from_protocol_level(level: u8) -> Option<Self> {
        match level {
            4 => Some(MqttVersion::V311),
            5 => Some(MqttVersion::V5),
            _ => None,
        }
    }

    pub fn is_v5(self) -> bool {
        matches!(self, MqttVersion::V5)
    }
}

/// MQTT Quality of Service level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Enum)]
pub enum QoS {
    /// QoS 0 — fire and forget, no acknowledgement.
    AtMostOnce,
    /// QoS 1 — acknowledged delivery, may be duplicated.
    AtLeastOnce,
    /// QoS 2 — exactly-once delivery via a four-part handshake.
    ExactlyOnce,
}

impl QoS {
    pub fn as_u8(self) -> u8 {
        match self {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
            QoS::ExactlyOnce => 2,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(QoS::AtMostOnce),
            1 => Some(QoS::AtLeastOnce),
            2 => Some(QoS::ExactlyOnce),
            _ => None,
        }
    }
}
