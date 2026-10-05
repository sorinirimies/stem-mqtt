//! Node.js / TypeScript bindings for [`mqtt_client`], built with
//! [napi-rs](https://napi.rs/). This is a separate binding path from the
//! UniFFI-generated Kotlin/Swift/Python bindings — UniFFI's C-ABI FFI model
//! doesn't map onto JavaScript the way it does onto the JVM/Swift/CPython
//! runtimes, so Node gets its own native addon crate instead.
//!
//! Scope: this wraps the existing `tokio`-based TCP/TLS `MqttClient`, so it
//! targets **Node.js (server-side / Electron main process)**, not the
//! browser. A browser/WASM MQTT client would need an MQTT-over-WebSocket
//! transport (browsers can't open raw TCP sockets), so that's future work,
//! not something this crate papers over.

#![deny(clippy::all)]

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

use mqtt_client::{
    ConnectOptions as CoreConnectOptions, MqttClient as CoreClient, MqttMessage as CoreMessage,
    MqttMessageListener, MqttVersion as CoreVersion, QoS as CoreQoS, TlsOptions as CoreTls,
    WillOptions as CoreWill,
};

/// Copy each listed `Option<T>` field from `$src` onto the same-named field
/// of `$dst`, but only when it is `Some` — so an omitted JS option keeps the
/// core library's default instead of being overwritten. Every optional
/// numeric/boolean connect option follows this exact shape.
macro_rules! apply_some {
    ($dst:ident <- $src:ident: $($field:ident),+ $(,)?) => {
        $( if let Some(value) = $src.$field { $dst.$field = value; } )+
    };
}

fn qos_from_u8(qos: u8) -> Result<CoreQoS> {
    CoreQoS::from_u8(qos).ok_or_else(|| Error::from_reason(format!("invalid QoS: {qos}")))
}

fn map_err(e: mqtt_client::MqttError) -> Error {
    Error::from_reason(e.to_string())
}

// NOTE: this crate has no Rust unit tests on purpose — a `cargo test` binary for a
// napi `cdylib` can't link (the `napi_*` symbols only exist inside a Node process).
// It is covered end to end by `scripts/smoke_test.mjs` (CI job `test-node`).

/// Parse the JS-facing protocol version string.
fn parse_version(version: &str) -> Result<CoreVersion> {
    match version {
        "3.1.1" | "311" => Ok(CoreVersion::V311),
        "5.0" | "5" => Ok(CoreVersion::V5),
        other => Err(Error::from_reason(format!(
            "invalid MQTT version {other:?}, expected \"3.1.1\" or \"5.0\""
        ))),
    }
}

/// Last-Will-and-Testament configuration, mirrors [`mqtt_client::WillOptions`].
#[napi(object)]
pub struct WillOptions {
    pub topic: String,
    pub payload: Buffer,
    pub qos: u8,
    pub retain: bool,
}

/// TLS configuration. Certificate and key values contain PEM bytes.
#[napi(object)]
pub struct TlsOptions {
    pub ca_cert_pem: Option<Buffer>,
    pub client_cert_pem: Option<Buffer>,
    pub client_key_pem: Option<Buffer>,
    pub insecure_skip_certificate_verification: Option<bool>,
}

/// Everything needed to establish an MQTT connection.
/// `version` is `"3.1.1"` or `"5.0"`.
#[napi(object)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub version: String,
    pub clean_start: Option<bool>,
    pub keep_alive_secs: Option<u16>,
    pub username: Option<String>,
    pub password: Option<Buffer>,
    pub will: Option<WillOptions>,
    pub connect_timeout_secs: Option<u32>,
    pub operation_timeout_secs: Option<u32>,
    pub auto_reconnect: Option<bool>,
    pub reconnect_backoff_secs: Option<u32>,
    pub reconnect_max_backoff_secs: Option<u32>,
    pub tls: Option<TlsOptions>,
    /// Largest packet (bytes) accepted from the broker; larger ones drop
    /// the connection. Omit for the library default.
    pub max_packet_size: Option<u32>,
}

impl ConnectOptions {
    fn into_core(self) -> Result<CoreConnectOptions> {
        let version = parse_version(&self.version)?;
        let mut opts = CoreConnectOptions::new(self.host, self.port, self.client_id, version);
        apply_some!(opts <- self:
            clean_start,
            keep_alive_secs,
            connect_timeout_secs,
            operation_timeout_secs,
            auto_reconnect,
            reconnect_backoff_secs,
            reconnect_max_backoff_secs,
            max_packet_size,
        );
        opts.username = self.username;
        opts.password = self.password.map(|b| b.to_vec());
        if let Some(w) = self.will {
            opts.will = Some(CoreWill {
                topic: w.topic,
                payload: w.payload.to_vec(),
                qos: qos_from_u8(w.qos)?,
                retain: w.retain,
            });
        }
        if let Some(tls) = self.tls {
            opts.tls = Some(CoreTls {
                ca_cert_pem: tls.ca_cert_pem.map(|value| value.to_vec()),
                client_cert_pem: tls.client_cert_pem.map(|value| value.to_vec()),
                client_key_pem: tls.client_key_pem.map(|value| value.to_vec()),
                insecure_skip_certificate_verification: tls
                    .insecure_skip_certificate_verification
                    .unwrap_or(false),
            });
        }
        Ok(opts)
    }
}

/// Outcome of a successful [`MqttClient::connect`].
#[napi(object)]
pub struct ConnectResult {
    pub session_present: bool,
    pub reason_code: u8,
}

/// A message delivered to a subscriber.
#[napi(object)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Buffer,
    pub qos: u8,
    pub retain: bool,
}

impl From<CoreMessage> for MqttMessage {
    fn from(m: CoreMessage) -> Self {
        MqttMessage {
            topic: m.topic,
            payload: m.payload.into(),
            qos: m.qos.as_u8(),
            retain: m.retain,
        }
    }
}

/// The reason/return code the broker granted for a subscribe request.
#[napi(object)]
pub struct SubscribeResult {
    pub reason_code: u8,
}

struct JsListener {
    on_message: ThreadsafeFunction<MqttMessage, ()>,
    on_disconnected: ThreadsafeFunction<String, ()>,
}

impl MqttMessageListener for JsListener {
    fn on_message(&self, message: CoreMessage) {
        self.on_message
            .call(Ok(message.into()), ThreadsafeFunctionCallMode::NonBlocking);
    }

    fn on_disconnected(&self, reason: String) {
        self.on_disconnected
            .call(Ok(reason), ThreadsafeFunctionCallMode::NonBlocking);
    }
}

/// A connected (or not-yet-connected) MQTT client instance.
///
/// One `MqttClient` corresponds to one broker connection / MQTT session.
/// Create it with `new MqttClient(options)`, call `connect()`, then
/// `publish()`/`subscribe()`/`unsubscribe()`. Call `disconnect()` (or drop
/// the object) to close the connection.
#[napi]
pub struct MqttClient {
    inner: Arc<CoreClient>,
}

#[napi]
impl MqttClient {
    #[napi(constructor)]
    pub fn new(options: ConnectOptions) -> Result<Self> {
        Ok(MqttClient {
            inner: Arc::new(CoreClient::new(options.into_core()?)),
        })
    }

    /// Register the listener that receives incoming messages and
    /// disconnect notifications. Safe to call before or after `connect()`.
    ///
    /// Both callbacks follow Node's error-first convention (`(err, value)`)
    /// because they're backed by a napi-rs `ThreadsafeFunction` in
    /// "callee handled" mode — `err` is always `null` here (these callbacks
    /// can't themselves fail), so in practice you only need the second
    /// argument.
    #[napi]
    pub fn set_message_listener(
        &self,
        #[napi(ts_arg_type = "(err: Error | null, message: MqttMessage) => void")]
        on_message: Function<MqttMessage, ()>,
        #[napi(ts_arg_type = "(err: Error | null, reason: string) => void")]
        on_disconnected: Function<String, ()>,
    ) -> Result<()> {
        let on_message: ThreadsafeFunction<MqttMessage, ()> = on_message
            .build_threadsafe_function()
            .callee_handled::<true>()
            .build()?;
        let on_disconnected: ThreadsafeFunction<String, ()> = on_disconnected
            .build_threadsafe_function()
            .callee_handled::<true>()
            .build()?;
        self.inner.set_message_listener(Arc::new(JsListener {
            on_message,
            on_disconnected,
        }));
        Ok(())
    }

    /// Open the TCP connection and complete the CONNECT/CONNACK handshake.
    #[napi]
    pub async fn connect(&self) -> Result<ConnectResult> {
        let result = self.inner.connect().await.map_err(map_err)?;
        Ok(ConnectResult {
            session_present: result.session_present,
            reason_code: result.reason_code,
        })
    }

    /// `true` once `connect()` has succeeded and the connection has not
    /// since been lost or closed.
    #[napi]
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    /// Publish `payload` to `topic` at the given QoS (0/1/2).
    #[napi]
    pub async fn publish(
        &self,
        topic: String,
        payload: Buffer,
        qos: u8,
        retain: bool,
    ) -> Result<()> {
        self.inner
            .publish(topic, payload.to_vec(), qos_from_u8(qos)?, retain)
            .await
            .map_err(map_err)
    }

    /// Subscribe to `topic_filter` requesting at most `qos`.
    #[napi]
    pub async fn subscribe(&self, topic_filter: String, qos: u8) -> Result<SubscribeResult> {
        let result = self
            .inner
            .subscribe(topic_filter, qos_from_u8(qos)?)
            .await
            .map_err(map_err)?;
        Ok(SubscribeResult {
            reason_code: result.reason_code,
        })
    }

    /// Unsubscribe from `topic_filter`.
    #[napi]
    pub async fn unsubscribe(&self, topic_filter: String) -> Result<()> {
        self.inner.unsubscribe(topic_filter).await.map_err(map_err)
    }

    /// Gracefully disconnect. Safe to call multiple times.
    #[napi]
    pub async fn disconnect(&self) -> Result<()> {
        self.inner.disconnect().await.map_err(map_err)
    }
}
