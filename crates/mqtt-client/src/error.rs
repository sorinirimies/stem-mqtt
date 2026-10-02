//! Error types shared across the MQTT client (and re-used by the broker
//! crate for the protocol codec it embeds).

/// Convenience alias for `Result<T, MqttError>`.
pub type MqttResult<T> = Result<T, MqttError>;

/// All errors that can be produced while encoding/decoding MQTT packets or
/// operating an [`crate::client::MqttClient`].
///
/// Exposed to foreign languages through UniFFI as a plain error enum.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MqttError {
    /// The peer sent a packet that violates the MQTT specification.
    #[error("malformed packet: {0}")]
    MalformedPacket(String),

    /// A value could not be encoded/decoded per the wire protocol.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Underlying I/O failure (connection reset, DNS failure, etc.).
    #[error("io error: {0}")]
    Io(String),

    /// The broker rejected the connection attempt.
    #[error("connection refused: {0}")]
    ConnectionRefused(String),

    /// An operation was attempted while not connected to a broker.
    #[error("not connected")]
    NotConnected,

    /// A packet's declared size exceeds the configured maximum packet size.
    #[error("packet too large: {0}")]
    PacketTooLarge(String),

    /// The client is already connected.
    #[error("already connected")]
    AlreadyConnected,

    /// The peer did not acknowledge a QoS 1/2 packet before the timeout.
    #[error("operation timed out")]
    Timeout,

    /// Generic keep-alive / session error.
    #[error("session error: {0}")]
    Session(String),
}

impl From<std::io::Error> for MqttError {
    fn from(e: std::io::Error) -> Self {
        MqttError::Io(e.to_string())
    }
}
