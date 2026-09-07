//! Broker-local errors exposed through UniFFI.
//!
//! Keep this type in the broker crate: UniFFI's Kotlin backend cannot lower
//! errors imported from another crate on exported async methods. Returning a
//! broker-owned error from `MqttBroker::start`/`stop` keeps every language's
//! generated API complete.

/// Convenience alias for broker lifecycle operations.
pub type MqttBrokerResult<T> = Result<T, MqttBrokerError>;

/// Errors produced while starting or stopping an [`crate::MqttBroker`].
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MqttBrokerError {
    /// `start()` was called while the broker was already running.
    #[error("broker is already running")]
    AlreadyRunning,

    /// A listening socket could not be bound or queried.
    #[error("broker I/O error: {0}")]
    Io(String),

    /// TLS certificate, key, or verifier configuration was invalid.
    #[error("broker TLS configuration error: {0}")]
    Tls(String),
}

impl From<std::io::Error> for MqttBrokerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
