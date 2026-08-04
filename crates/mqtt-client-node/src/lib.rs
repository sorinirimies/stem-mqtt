//! Node.js / TypeScript bindings for [`mqtt_client`], built with
//! [napi-rs](https://napi.rs/). This is a separate binding path from the
//! UniFFI-generated Kotlin/Swift/Python bindings — UniFFI's C-ABI FFI model
//! doesn't map onto JavaScript the way it does onto the JVM/Swift/CPython
//! runtimes, so Node gets its own native addon crate instead.
//!
//! Scope: this wraps the existing `tokio`-based, raw-TCP `MqttClient`, so it
//! targets **Node.js (server-side / Electron main process)**, not the
//! browser. A browser/WASM MQTT client would need an MQTT-over-WebSocket
//! transport (browsers can't open raw TCP sockets) — the broker and client
//! here only speak plain MQTT-over-TCP today, so that's future work, not
//! something this crate papers over.

#![deny(clippy::all)]

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

use mqtt_client::{
    ConnectOptions as CoreConnectOptions, MqttClient as CoreClient, MqttMessage as CoreMessage,
    MqttMessageListener, MqttVersion as CoreVersion, QoS as CoreQoS, WillOptions as CoreWill,
};

fn qos_from_u8(qos: u8) -> Result<CoreQoS> {
    CoreQoS::from_u8(qos).ok_or_else(|| Error::from_reason(format!("invalid QoS: {qos}")))
}

fn qos_to_u8(qos: CoreQoS) -> u8 {
    qos.as_u8()
}

fn map_err(e: mqtt_client::MqttError) -> Error {
    Error::from_reason(e.to_string())
}

/// Last-Will-and-Testament configuration, mirrors [`mqtt_client::WillOptions`].
#[napi(object)]
pub struct WillOptions {
    pub topic: String,
    pub payload: Buffer,
    pub qos: u8,
    pub retain: bool,
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
}

impl ConnectOptions {
    fn into_core(self) -> Result<CoreConnectOptions> {
        let version = match self.version.as_str() {
            "3.1.1" | "311" => CoreVersion::V311,
            "5.0" | "5" => CoreVersion::V5,
            other => {
                return Err(Error::from_reason(format!(
                    "invalid MQTT version {other:?}, expected \"3.1.1\" or \"5.0\""
                )))
            }
        };
        let mut opts = CoreConnectOptions::new(self.host, self.port, self.client_id, version);
        if let Some(v) = self.clean_start {
            opts.clean_start = v;
        }
        if let Some(v) = self.keep_alive_secs {
            opts.keep_alive_secs = v;
        }
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
        if let Some(v) = self.connect_timeout_secs {
            opts.connect_timeout_secs = v;
        }
        if let Some(v) = self.operation_timeout_secs {
            opts.operation_timeout_secs = v;
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
            qos: qos_to_u8(m.qos),
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
