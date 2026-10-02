//! Public data types for the client API: connection configuration,
//! results, delivered messages, and the foreign-callback listener trait.
//! No I/O or connection-state logic lives here — see
//! [`crate::client::inner`] and [`crate::client::io`].

use std::time::Duration;

use crate::protocol::packet::MAX_PACKET_SIZE;
use crate::protocol::{MqttVersion, QoS};
use crate::support::secs_or;

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

/// TLS configuration for [`ConnectOptions::tls`]. All certificate/key
/// material is PEM-encoded bytes (not file paths) so this works
/// identically across every language binding without assuming a
/// filesystem layout.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct TlsOptions {
    /// Additional PEM-encoded root CA certificate(s) to trust, on top of
    /// the bundled Mozilla root store (`webpki-roots`). Needed to connect
    /// to a broker with a self-signed or private-CA certificate.
    pub ca_cert_pem: Option<Vec<u8>>,
    /// PEM-encoded client certificate, for mutual TLS (mTLS). Requires
    /// [`Self::client_key_pem`] too.
    pub client_cert_pem: Option<Vec<u8>>,
    /// PEM-encoded client private key, for mutual TLS. Requires
    /// [`Self::client_cert_pem`] too.
    pub client_key_pem: Option<Vec<u8>>,
    /// Skip server certificate verification entirely. **Dangerous** — only
    /// for local development against a broker with a self-signed
    /// certificate you can't otherwise easily trust (e.g. one generated
    /// on the fly for a demo). Never use this in production: it makes the
    /// connection trivially interceptable.
    pub insecure_skip_certificate_verification: bool,
}

// Never print the client private key (or a CA bundle's presence-only detail
// beyond "is set") when a `TlsOptions` is `Debug`-formatted.
crate::redacted_debug!(TlsOptions {
    ca_cert_pem, client_cert_pem, insecure_skip_certificate_verification
} secret { client_key_pem });

impl TlsOptions {
    /// TLS with the bundled Mozilla root store and no client certificate —
    /// the common case of connecting to a broker with a certificate from a
    /// well-known public CA.
    pub fn new() -> Self {
        TlsOptions {
            ca_cert_pem: None,
            client_cert_pem: None,
            client_key_pem: None,
            insecure_skip_certificate_verification: false,
        }
    }
}

impl Default for TlsOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything needed to establish an MQTT connection.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
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
    /// If set, connect over TLS (`mqtts`) instead of plain TCP. `None`
    /// (the default) is plain TCP, unchanged from before TLS support
    /// existed.
    pub tls: Option<TlsOptions>,
    /// Largest packet (fixed header + body, in bytes) this client will
    /// accept from the broker; a larger one is treated as a protocol error
    /// and drops the connection instead of being buffered. `0` selects the
    /// built-in default (the MQTT protocol maximum, ~256 MiB).
    #[uniffi(default = 0)]
    pub max_packet_size: u32,
}

// Hand-rolled `Debug` so the password never reaches a log line.
crate::redacted_debug!(ConnectOptions {
    host, port, client_id, version, clean_start, keep_alive_secs, username, will,
    connect_timeout_secs, operation_timeout_secs, auto_reconnect,
    reconnect_backoff_secs, reconnect_max_backoff_secs, tls, max_packet_size
} secret { password });

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
            tls: None,
            max_packet_size: 0,
        }
    }

    pub(super) fn operation_timeout(&self) -> Duration {
        secs_or(self.operation_timeout_secs, DEFAULT_OPERATION_TIMEOUT_SECS)
    }

    pub(super) fn connect_timeout(&self) -> Duration {
        // A zero connect timeout would fail instantly; treat it as "1s".
        secs_or(self.connect_timeout_secs, 1)
    }

    pub(super) fn initial_reconnect_backoff(&self) -> Duration {
        secs_or(self.reconnect_backoff_secs, 1)
    }

    pub(super) fn max_reconnect_backoff(&self) -> Duration {
        secs_or(self.reconnect_max_backoff_secs, 30)
    }

    pub(super) fn max_packet_size(&self) -> usize {
        match self.max_packet_size {
            0 => MAX_PACKET_SIZE,
            n => n as usize,
        }
    }

    /// Reject configurations that can never work, before touching the
    /// network, with a message naming the offending field.
    pub(super) fn validate(&self) -> crate::error::MqttResult<()> {
        use crate::error::MqttError::Protocol;
        use crate::protocol::topic::is_valid_topic_name;
        if self.host.is_empty() {
            return Err(Protocol("ConnectOptions.host must not be empty".into()));
        }
        if self.client_id.len() > usize::from(u16::MAX) {
            return Err(Protocol("ConnectOptions.client_id is too long".into()));
        }
        if self.password.is_some() && self.username.is_none() && !self.version.is_v5() {
            // MQTT-3.1.2-22: 3.1.1 forbids a password without a username.
            return Err(Protocol(
                "ConnectOptions.password requires a username in MQTT 3.1.1".into(),
            ));
        }
        if let Some(will) = &self.will {
            if !is_valid_topic_name(&will.topic) {
                return Err(Protocol(format!(
                    "invalid will topic {:?}: must be non-empty and wildcard-free",
                    will.topic
                )));
            }
        }
        Ok(())
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

impl From<&crate::protocol::publish::PublishPacket> for MqttMessage {
    fn from(p: &crate::protocol::publish::PublishPacket) -> Self {
        MqttMessage {
            topic: p.topic.clone(),
            payload: p.payload.to_vec(),
            qos: p.qos,
            retain: p.retain,
        }
    }
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
