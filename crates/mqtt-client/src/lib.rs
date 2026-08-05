//! `mqtt-client` — a full MQTT 3.1.1 / MQTT 5.0 client written in Rust,
//! exposed to other languages (Kotlin, Swift, Python, Ruby, Go, ...) via
//! [UniFFI](https://mozilla.github.io/uniffi-rs/).
//!
//! The [`protocol`] module is a pure, allocation-friendly codec for the MQTT
//! wire format with no networking dependencies; [`client`] builds an async,
//! `tokio`-based client on top of it. The `mqtt-broker` crate re-uses
//! [`protocol`] directly instead of duplicating the codec.

pub mod client;
pub mod error;
pub mod protocol;

pub use client::{
    ConnectOptions, ConnectResult, MqttClient, MqttMessage, MqttMessageListener, TlsOptions,
    WillOptions,
};
pub use error::MqttError;
pub use protocol::{MqttVersion, QoS};

uniffi::setup_scaffolding!();
