//! `mqtt-broker` — an MQTT 3.1.1 / MQTT 5.0 broker written in Rust,
//! exposed to other languages via [UniFFI](https://mozilla.github.io/uniffi-rs/).
//!
//! Reuses the wire-protocol codec from the `mqtt-client` crate
//! (`mqtt_client::protocol`) rather than duplicating it — this workspace
//! intentionally has no separate "core" crate; the client crate's codec
//! doubles as the shared foundation for both binaries.

mod broker;
mod config;
mod connection;
mod error;
mod events;
mod index;
mod registry;
mod retain;
mod session;
mod tls;
mod topic;
mod ws;

pub use broker::MqttBroker;
pub use config::{
    BrokerTlsConfig, EnhancedAuthOutcome, EnhancedAuthStep, MqttAuthProvider, MqttBrokerConfig,
    MqttBrokerEventListener, MqttEnhancedAuthProvider, QoS, DEFAULT_MAX_OUTBOUND_QUEUE,
    DEFAULT_MAX_PACKET_SIZE,
};
pub use error::{MqttBrokerError, MqttBrokerResult};
pub use events::BrokerEvent;

uniffi::setup_scaffolding!();
