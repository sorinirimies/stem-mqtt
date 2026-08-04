//! Shared broker state — [`BrokerState`] composes the three subsystems
//! ([`SessionRegistry`], [`RetainStore`], [`EventHub`]) and owns only the
//! logic that genuinely spans more than one of them (will delivery on
//! disconnect, retained-message replay on subscribe). Everything else is a
//! direct call into whichever subsystem owns it — see `connection.rs`.
//! [`MqttBroker`] itself is the thin UniFFI-exported handle: bind sockets,
//! spawn per-connection tasks, expose start/stop/status.

use std::sync::{Arc, Mutex};

use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::QoS;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::config::{
    MqttAuthProvider, MqttBrokerConfig, MqttBrokerEventListener, SharedAuthProvider,
};
use crate::connection::handle_connection;
use crate::events::EventHub;
use crate::registry::SessionRegistry;
use crate::retain::RetainStore;
use crate::session::QueuedMessage;
use crate::ws::WsByteStream;

/// State shared by every connection task belonging to one [`MqttBroker`].
///
/// Deliberately *not* a single flat bag of fields: each subsystem
/// ([`Self::sessions`], [`Self::retained`], [`Self::events`]) is a
/// self-contained type that can be understood (and tested) on its own.
/// `BrokerState` itself only holds the config plus the handful of
/// operations that need to coordinate two subsystems at once.
pub(crate) struct BrokerState {
    pub config: MqttBrokerConfig,
    pub sessions: SessionRegistry,
    pub retained: RetainStore,
    pub auth_provider: Mutex<Option<SharedAuthProvider>>,
    pub events: EventHub,
}

impl BrokerState {
    fn new(config: MqttBrokerConfig) -> Self {
        let retained = RetainStore::new(config.max_retained_messages);
        BrokerState {
            config,
            sessions: SessionRegistry::new(),
            retained,
            auth_provider: Mutex::new(None),
            events: EventHub::new(),
        }
    }

    pub fn client_count(&self) -> u32 {
        self.sessions.client_count()
    }

    /// Send every retained message matching `filter` to `client_id`,
    /// downgrading each to at most `max_qos` — the QoS just granted for
    /// that subscription. Spans [`RetainStore`] (lookup) and
    /// [`SessionRegistry`] (delivery), so it lives here rather than on
    /// either subsystem alone.
    pub fn send_matching_retained(&self, client_id: &str, filter: &str, max_qos: QoS) {
        for (topic, msg) in self.retained.matching(filter) {
            self.sessions
                .send_retained(client_id, topic, msg.payload, msg.qos.min(max_qos));
        }
    }

    /// Detach `client_id`'s connection (delegating the session
    /// bookkeeping to [`SessionRegistry::detach`]) and, if it disconnected
    /// ungracefully with a Last Will configured, publish that will —
    /// retaining it first if requested. Spans [`SessionRegistry`] and
    /// [`RetainStore`], so it lives here rather than on either subsystem
    /// alone.
    pub fn detach_session(&self, client_id: &str, graceful: bool) {
        let Some(will) = self.sessions.detach(client_id, graceful) else {
            return;
        };

        let message = QueuedMessage {
            topic: will.topic,
            payload: will.payload,
            qos: will.qos,
            retain: will.retain,
        };
        if message.retain {
            self.retained.update(&PublishPacket {
                dup: false,
                qos: message.qos,
                retain: true,
                topic: message.topic.clone(),
                packet_id: None,
                payload: message.payload.clone(),
                properties: mqtt_client::protocol::properties::Properties::new(),
            });
        }
        self.sessions.fan_out(client_id, &message);
    }
}

/// A running (or not-yet-started) MQTT broker instance, speaking both MQTT
/// 3.1.1 and MQTT 5.0 — the protocol version is negotiated independently
/// per connection from each client's CONNECT packet.
#[derive(uniffi::Object)]
pub struct MqttBroker {
    state: Arc<BrokerState>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
    ws_accept_task: Mutex<Option<JoinHandle<()>>>,
    bound_port: Mutex<Option<u16>>,
    bound_ws_port: Mutex<Option<u16>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MqttBroker {
    #[uniffi::constructor]
    pub fn new(config: MqttBrokerConfig) -> Self {
        MqttBroker {
            state: Arc::new(BrokerState::new(config)),
            accept_task: Mutex::new(None),
            ws_accept_task: Mutex::new(None),
            bound_port: Mutex::new(None),
            bound_ws_port: Mutex::new(None),
        }
    }

    /// Register a pluggable authentication provider. Must be called before
    /// [`start`](Self::start) to affect connections made right away, but
    /// may be replaced at any time.
    pub fn set_auth_provider(&self, provider: Arc<dyn MqttAuthProvider>) {
        *self.state.auth_provider.lock().unwrap() = Some(provider);
    }

    /// Register a listener for connect/disconnect/publish events.
    pub fn set_event_listener(&self, listener: Arc<dyn MqttBrokerEventListener>) {
        self.state.events.set_listener(listener);
    }

    /// Bind the listening socket(s) and start accepting connections. Also
    /// binds a second, MQTT-over-WebSocket listener on
    /// `config.ws_port` if it's non-zero (see
    /// [`MqttBrokerConfig::ws_port`]).
    pub async fn start(&self) -> mqtt_client::error::MqttResult<()> {
        if self.accept_task.lock().unwrap().is_some() {
            return Err(mqtt_client::MqttError::AlreadyConnected);
        }
        let addr = format!(
            "{}:{}",
            self.state.config.bind_address, self.state.config.port
        );
        let listener = TcpListener::bind(&addr).await?;
        let local_port = listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(self.state.config.port);
        *self.bound_port.lock().unwrap() = Some(local_port);

        let state = self.state.clone();
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        stream.set_nodelay(true).ok();
                        let state = state.clone();
                        tokio::spawn(handle_connection(state, stream, peer.to_string()));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept() failed");
                        break;
                    }
                }
            }
        });
        *self.accept_task.lock().unwrap() = Some(handle);

        if let Some(ws_port) = self.state.config.ws_port {
            let ws_addr = format!("{}:{}", self.state.config.bind_address, ws_port);
            let ws_listener = TcpListener::bind(&ws_addr).await?;
            let local_ws_port = ws_listener
                .local_addr()
                .map(|a| a.port())
                .unwrap_or(ws_port);
            *self.bound_ws_port.lock().unwrap() = Some(local_ws_port);

            let ws_state = self.state.clone();
            let ws_handle = tokio::spawn(async move {
                loop {
                    match ws_listener.accept().await {
                        Ok((stream, peer)) => {
                            stream.set_nodelay(true).ok();
                            let state = ws_state.clone();
                            tokio::spawn(accept_ws_connection(state, stream, peer.to_string()));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "ws accept() failed");
                            break;
                        }
                    }
                }
            });
            *self.ws_accept_task.lock().unwrap() = Some(ws_handle);
        }

        Ok(())
    }

    /// Stop accepting new connections and abort the accept loop(s).
    /// Existing client connections are left running until they naturally
    /// close.
    pub async fn stop(&self) -> mqtt_client::error::MqttResult<()> {
        if let Some(handle) = self.accept_task.lock().unwrap().take() {
            handle.abort();
        }
        if let Some(handle) = self.ws_accept_task.lock().unwrap().take() {
            handle.abort();
        }
        *self.bound_port.lock().unwrap() = None;
        *self.bound_ws_port.lock().unwrap() = None;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.accept_task.lock().unwrap().is_some()
    }

    /// Number of currently connected clients.
    pub fn client_count(&self) -> u32 {
        self.state.client_count()
    }

    /// The TCP port actually bound (useful when `config.port == 0` was used
    /// to request an ephemeral port, e.g. in tests).
    pub fn bound_port(&self) -> Option<u16> {
        *self.bound_port.lock().unwrap()
    }

    /// The WebSocket port actually bound, if `config.ws_port != 0`.
    pub fn bound_ws_port(&self) -> Option<u16> {
        *self.bound_ws_port.lock().unwrap()
    }
}

/// Complete the WebSocket upgrade handshake (negotiating the `mqtt`
/// subprotocol per MQTT-5.0 §6.4.1), then hand off to the same
/// [`handle_connection`] every raw-TCP connection goes through.
async fn accept_ws_connection(
    state: Arc<BrokerState>,
    stream: tokio::net::TcpStream,
    peer: String,
) {
    // The `Err` side of `accept_hdr_async`'s callback contract is
    // tungstenite's own `ErrorResponse` type (an HTTP response) — its size
    // isn't ours to shrink, so this lint doesn't apply here.
    #[allow(clippy::result_large_err)]
    let ws_stream = match tokio_tungstenite::accept_hdr_async(
        stream,
        |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
         mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            use tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;
            use tokio_tungstenite::tungstenite::http::HeaderValue;

            let offers_mqtt = req
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(',').any(|p| p.trim().eq_ignore_ascii_case("mqtt")))
                .unwrap_or(false);

            if !offers_mqtt {
                let resp: ErrorResponse = tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(400)
                    .body(Some("expected Sec-WebSocket-Protocol: mqtt".to_string()))
                    .unwrap();
                return Err(resp);
            }
            response
                .headers_mut()
                .insert("sec-websocket-protocol", HeaderValue::from_static("mqtt"));
            Ok(response)
        },
    )
    .await
    {
        Ok(ws) => ws,
        Err(e) => {
            tracing::debug!(%peer, error = %e, "WebSocket handshake failed");
            return;
        }
    };

    handle_connection(state, WsByteStream::new(ws_stream), peer).await;
}
