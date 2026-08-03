//! Broker configuration, authentication, and event-observation types
//! exposed across the UniFFI boundary.

use std::sync::Arc;

use mqtt_client::QoS;

/// Configuration for an [`crate::broker::MqttBroker`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MqttBrokerConfig {
    /// Address to bind the listening socket to, e.g. `"0.0.0.0"` or `"::"`.
    pub bind_address: String,
    pub port: u16,
    /// Accept connections without a username/password when no
    /// [`MqttAuthProvider`] is registered. Ignored if an auth provider is
    /// set — the provider always makes the final decision.
    pub allow_anonymous: bool,
    /// Maximum simultaneously connected clients. `0` means unlimited.
    pub max_clients: u32,
    /// Highest QoS the broker will grant on SUBSCRIBE (messages published
    /// at a higher QoS are still accepted, but delivered at this ceiling).
    pub max_qos: QoS,
    /// Maximum number of retained messages kept in memory. `0` means
    /// unlimited.
    pub max_retained_messages: u32,
    /// Maximum number of messages queued per offline session (for clients
    /// that connected with `clean_start = false`). `0` means unlimited.
    pub max_queued_per_client: u32,
}

impl MqttBrokerConfig {
    pub fn new(bind_address: impl Into<String>, port: u16) -> Self {
        MqttBrokerConfig {
            bind_address: bind_address.into(),
            port,
            allow_anonymous: true,
            max_clients: 0,
            max_qos: QoS::ExactlyOnce,
            max_retained_messages: 10_000,
            max_queued_per_client: 1_000,
        }
    }
}

/// Pluggable authentication, evaluated once per CONNECT.
#[uniffi::export(with_foreign)]
pub trait MqttAuthProvider: Send + Sync {
    /// Return `true` to accept the connection.
    fn authenticate(
        &self,
        client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
    ) -> bool;
}

/// Broker-side view of a delivered/observed event, for monitoring and
/// logging from foreign code.
#[uniffi::export(with_foreign)]
pub trait MqttBrokerEventListener: Send + Sync {
    fn on_client_connected(&self, client_id: String);
    fn on_client_disconnected(&self, client_id: String, reason: String);
    fn on_message_published(&self, client_id: String, topic: String, qos: QoS);
}

pub(crate) type SharedAuthProvider = Arc<dyn MqttAuthProvider>;
pub(crate) type SharedEventListener = Arc<dyn MqttBrokerEventListener>;
