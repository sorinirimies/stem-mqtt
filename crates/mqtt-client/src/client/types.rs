//! Public data types for the client API: connection configuration,
//! results, delivered messages, and the foreign-callback listener trait.
//! No I/O or connection-state logic lives here — see
//! [`crate::client::inner`] and [`crate::client::io`].

use std::time::Duration;

use crate::protocol::{MqttVersion, QoS};

/// Default time to wait for a broker acknowledgement (PUBACK/PUBREC/
/// PUBCOMP/SUBACK/UNSUBACK) before returning [`crate::error::MqttError::Timeout`].
pub(super) const DEFAULT_OPERATION_TIMEOUT_SECS: u32 = 15;

/// Last-Will-and-Testament configuration for a [`ConnectOptions`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WillOptions {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// Everything needed to establish an MQTT connection.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    /// Which protocol version to speak. The broker never up/downgrades a
    /// connection once CONNECT is sent.
    pub version: MqttVersion,
    /// MQTT 3.1.1 "Clean Session" / MQTT 5.0 "Clean Start".
    pub clean_start: bool,
    /// Keep-alive interval in seconds. `0` disables keep-alive pings.
    pub keep_alive_secs: u16,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub will: Option<WillOptions>,
    /// Seconds to wait for the TCP connection + CONNACK before failing.
    pub connect_timeout_secs: u32,
    /// Seconds to wait for a QoS 1/2 handshake step *per attempt* before
    /// retrying (with DUP=1) — `MqttClient::publish` retries an unacked
    /// QoS 1/2 PUBLISH (or QoS 2's PUBREL) up to 3 times before giving up,
    /// so the worst-case total wait for a single `publish()` call is up to
    /// 4x this value. `0` selects the built-in default (15s).
    pub operation_timeout_secs: u32,
    /// If true, [`crate::client::MqttClient`] automatically reconnects
    /// (with exponential backoff, see [`Self::reconnect_backoff_secs`] /
    /// [`Self::reconnect_max_backoff_secs`]) after a connection loss that
    /// wasn't caused by calling `disconnect()`, replaying every topic this
    /// client is currently subscribed to (per its own bookkeeping) once
    /// reconnected. Off by default — the caller must opt in.
    pub auto_reconnect: bool,
    /// Initial delay before the first reconnect attempt, doubling after
    /// each failed attempt up to [`Self::reconnect_max_backoff_secs`].
    /// `0` selects the built-in default (1s). Ignored unless
    /// [`Self::auto_reconnect`] is set.
    pub reconnect_backoff_secs: u32,
    /// Cap on the exponential reconnect backoff delay. `0` selects the
    /// built-in default (30s). Ignored unless [`Self::auto_reconnect`] is
    /// set.
    pub reconnect_max_backoff_secs: u32,
}

impl ConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        client_id: impl Into<String>,
        version: MqttVersion,
    ) -> Self {
        ConnectOptions {
            host: host.into(),
            port,
            client_id: client_id.into(),
            version,
            clean_start: true,
            keep_alive_secs: 30,
            username: None,
            password: None,
            will: None,
            connect_timeout_secs: 10,
            operation_timeout_secs: 0,
            auto_reconnect: false,
            reconnect_backoff_secs: 0,
            reconnect_max_backoff_secs: 0,
        }
    }

    pub(super) fn operation_timeout(&self) -> Duration {
        let secs = if self.operation_timeout_secs == 0 {
            DEFAULT_OPERATION_TIMEOUT_SECS
        } else {
            self.operation_timeout_secs
        };
        Duration::from_secs(secs as u64)
    }

    pub(super) fn initial_reconnect_backoff(&self) -> Duration {
        let secs = if self.reconnect_backoff_secs == 0 {
            1
        } else {
            self.reconnect_backoff_secs
        };
        Duration::from_secs(secs as u64)
    }

    pub(super) fn max_reconnect_backoff(&self) -> Duration {
        let secs = if self.reconnect_max_backoff_secs == 0 {
            30
        } else {
            self.reconnect_max_backoff_secs
        };
        Duration::from_secs(secs as u64)
    }
}

/// Outcome of a successful [`crate::client::MqttClient::connect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ConnectResult {
    pub session_present: bool,
    /// `0x00` on success. MQTT 5.0 reason code (MQTT 3.1.1 codes are mapped
    /// onto the equivalent 5.0 code by
    /// [`ConnectReasonCode`](crate::protocol::connect::ConnectReasonCode)).
    pub reason_code: u8,
}

/// A message delivered to a subscriber.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// Result of a subscribe request: the reason/return code the broker
/// granted for the requested filter (`< 0x80` is success; MQTT 3.1.1 codes
/// 0/1/2 map onto the granted QoS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SubscribeResult {
    pub reason_code: u8,
}

/// Callback interface implemented by foreign code to receive events from an
/// [`crate::client::MqttClient`] without polling.
#[uniffi::export(with_foreign)]
pub trait MqttMessageListener: Send + Sync {
    /// Called for every PUBLISH delivered to a subscription this client holds.
    fn on_message(&self, message: MqttMessage);
    /// Called when the connection to the broker is lost or closed, whether
    /// by the client, the broker, or the network.
    fn on_disconnected(&self, reason: String);
}
