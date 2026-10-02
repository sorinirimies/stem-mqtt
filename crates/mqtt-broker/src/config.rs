//! Broker configuration, authentication, and event-observation types
//! exposed across the UniFFI boundary.

use std::sync::Arc;
use std::time::Duration;

use mqtt_client::support::secs_or;
use mqtt_client::QoS;

/// Default cap on a single inbound packet (fixed header + body): 1 MiB.
/// An unauthenticated peer can announce any size up to the protocol's
/// ~256 MiB maximum; without a cap each connection could force that much
/// buffering.
pub const DEFAULT_MAX_PACKET_SIZE: u32 = 1024 * 1024;

/// Default number of encoded packets that may be queued for one client's
/// socket before further deliveries to it are dropped (see
/// [`MqttBrokerConfig::max_outbound_queue`]).
pub const DEFAULT_MAX_OUTBOUND_QUEUE: u32 = 4096;

/// Configuration for an [`crate::broker::MqttBroker`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MqttBrokerConfig {
    /// Address to bind the listening socket to, e.g. `"0.0.0.0"` or `"::"`.
    pub bind_address: String,
    pub port: u16,
    /// Port to accept MQTT-over-WebSocket connections on (MQTT-5.0 §6 /
    /// MQTT-3.1.1 Appendix B), bound alongside the raw-TCP `port` above.
    /// `None` disables the WebSocket listener entirely — the default,
    /// since most deployments only need raw TCP. `Some(0)` binds an
    /// OS-assigned ephemeral WebSocket port (mirroring `port`'s own `0`
    /// convention), which is why this is `Option<u16>` rather than reusing
    /// `0` itself as the "disabled" sentinel.
    pub ws_port: Option<u16>,
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
    /// How often the broker resends (with DUP=1) an outgoing QoS 1/2
    /// packet that hasn't been acked yet. `0` selects the built-in default
    /// (5 seconds).
    pub redelivery_interval_secs: u32,
    /// If set, also accept TLS (`mqtts`) connections on
    /// [`BrokerTlsConfig::port`], alongside the plain-TCP `port` (and
    /// optional WebSocket `ws_port`) above. `None` (the default) means no
    /// TLS listener at all, unchanged from before TLS support existed.
    pub tls: Option<BrokerTlsConfig>,
    /// Largest packet (fixed header + body, in bytes) accepted from any
    /// client. A client announcing a bigger one is disconnected as soon as
    /// the packet *header* is read, before the body is buffered. `0`
    /// selects the built-in default
    /// ([`DEFAULT_MAX_PACKET_SIZE`], 1 MiB).
    #[uniffi(default = 0)]
    pub max_packet_size: u32,
    /// Maximum encoded packets queued for one client's socket. A client
    /// that reads slower than messages arrive would otherwise make the
    /// broker buffer without bound; once this many are waiting, further
    /// deliveries to *that* client are dropped (QoS 1/2 ones are retried
    /// by the redelivery sweep). `0` selects the built-in default
    /// ([`DEFAULT_MAX_OUTBOUND_QUEUE`], 4096).
    #[uniffi(default = 0)]
    pub max_outbound_queue: u32,
}

/// TLS configuration for [`MqttBrokerConfig::tls`]. All certificate/key
/// material is PEM-encoded bytes (not file paths), matching
/// `mqtt_client::TlsOptions` so this works identically across every
/// language binding without assuming a filesystem layout.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct BrokerTlsConfig {
    /// Port to accept TLS connections on.
    pub port: u16,
    /// PEM-encoded server certificate chain (leaf certificate first, then
    /// any intermediates).
    pub cert_pem: Vec<u8>,
    /// PEM-encoded server private key.
    pub key_pem: Vec<u8>,
    /// PEM-encoded CA certificate(s) to verify *client* certificates
    /// against, for mutual TLS (mTLS). If unset (the default), client
    /// certificates aren't required or checked — just a normal one-way
    /// TLS listener.
    pub client_ca_pem: Option<Vec<u8>>,
}

// Never print the server private key when a config is `Debug`-formatted.
mqtt_client::redacted_debug!(BrokerTlsConfig { port, cert_pem, client_ca_pem } secret { key_pem });

impl MqttBrokerConfig {
    pub fn new(bind_address: impl Into<String>, port: u16) -> Self {
        MqttBrokerConfig {
            bind_address: bind_address.into(),
            port,
            ws_port: None,
            allow_anonymous: true,
            max_clients: 0,
            max_qos: QoS::ExactlyOnce,
            max_retained_messages: 10_000,
            max_queued_per_client: 1_000,
            redelivery_interval_secs: 0,
            tls: None,
            max_packet_size: 0,
            max_outbound_queue: 0,
        }
    }

    pub(crate) fn redelivery_interval(&self) -> Duration {
        secs_or(
            self.redelivery_interval_secs,
            crate::registry::DEFAULT_REDELIVERY_INTERVAL.as_secs() as u32,
        )
    }

    pub(crate) fn max_packet_size_bytes(&self) -> usize {
        match self.max_packet_size {
            0 => DEFAULT_MAX_PACKET_SIZE as usize,
            n => n as usize,
        }
    }

    pub(crate) fn outbound_queue_capacity(&self) -> usize {
        match self.max_outbound_queue {
            0 => DEFAULT_MAX_OUTBOUND_QUEUE as usize,
            n => n as usize,
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
