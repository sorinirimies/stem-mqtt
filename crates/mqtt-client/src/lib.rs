//! `mqtt-client` — an MQTT 3.1.1 / MQTT 5.0 client written in Rust,
//! exposed to other languages (Kotlin, Swift, and Python) via
//! [UniFFI](https://mozilla.github.io/uniffi-rs/).
//!
//! The wire format lives in the runtime-free [`stem-mqtt-proto`] crate, re-exported
//! here as [`protocol`] and [`error`] (the paths are unchanged); [`client`] builds an
//! async, `tokio`-based client on top of it. The `mqtt-broker` crate re-uses the same
//! codec instead of duplicating it.
//!
//! [`stem-mqtt-proto`]: https://docs.rs/stem-mqtt-proto

#![cfg_attr(not(test), deny(clippy::unwrap_used))]
pub mod client;
pub mod support;

/// The MQTT wire protocol codec (the `stem-mqtt-proto` crate, re-exported).
pub mod protocol {
    pub use mqtt_proto::*;
}

/// The error type (from `stem-mqtt-proto`, re-exported).
pub mod error {
    pub use mqtt_proto::error::*;
}

pub use client::{
    ConnectOptions, ConnectResult, MqttAuthHandler, MqttClient, MqttMessage, MqttMessageListener,
    TlsOptions, WillOptions,
};
pub use mqtt_proto::redacted_debug;
pub use mqtt_proto::{MqttError, MqttVersion, QoS};

uniffi::setup_scaffolding!();

// The codec types live in `stem-mqtt-proto`, which has no UniFFI dependency. They are declared
// here as *remote* types so that they belong to this crate's UniFFI namespace exactly as before:
// the generated Kotlin, Swift, Python, ... bindings are unchanged. Keep these definitions in sync
// with `mqtt_proto` (a mismatch is a compile error).

/// The MQTT protocol version negotiated for a connection.
#[uniffi::remote(Enum)]
pub enum MqttVersion {
    /// MQTT 3.1.1 (protocol level 4).
    V311,
    /// MQTT 5.0 (protocol level 5).
    V5,
}

/// MQTT Quality of Service level.
#[uniffi::remote(Enum)]
pub enum QoS {
    /// QoS 0 — fire and forget, no acknowledgement.
    AtMostOnce,
    /// QoS 1 — acknowledged delivery, may be duplicated.
    AtLeastOnce,
    /// QoS 2 — exactly-once delivery via a four-part handshake.
    ExactlyOnce,
}

/// All errors that can be produced while encoding/decoding MQTT packets or
/// operating an [`crate::client::MqttClient`].
///
/// Exposed to foreign languages through UniFFI as a plain error enum.
#[uniffi::remote(Error)]
#[uniffi(flat_error)]
pub enum MqttError {
    /// The peer sent a packet that violates the MQTT specification.
    MalformedPacket(String),
    /// A value could not be encoded/decoded per the wire protocol.
    Protocol(String),
    /// Underlying I/O failure (connection reset, DNS failure, etc.).
    Io(String),
    /// The broker rejected the connection attempt.
    ConnectionRefused(String),
    /// An operation was attempted while not connected to a broker.
    NotConnected,
    /// A packet's declared size exceeds the configured maximum packet size.
    PacketTooLarge(String),
    /// The client is already connected.
    AlreadyConnected,
    /// The peer did not acknowledge a QoS 1/2 packet before the timeout.
    Timeout,
    /// Generic keep-alive / session error.
    Session(String),
}
