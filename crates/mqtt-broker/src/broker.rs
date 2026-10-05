//! Shared broker state — [`BrokerState`] composes the three subsystems
//! ([`SessionRegistry`], [`RetainStore`], [`EventHub`]) and owns only the
//! logic that genuinely spans more than one of them (will delivery on
//! disconnect, retained-message replay on subscribe). Everything else is a
//! direct call into whichever subsystem owns it — see `connection.rs`.
//! [`MqttBroker`] itself is the thin UniFFI-exported handle: bind sockets,
//! spawn per-connection tasks, expose start/stop/status.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mqtt_client::support::LockExt;
use mqtt_client::QoS;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::config::{
    MqttAuthProvider, MqttBrokerConfig, MqttBrokerEventListener, MqttEnhancedAuthProvider,
    SharedAuthProvider, SharedEnhancedAuth,
};
use crate::connection::handle_connection;
use crate::error::{MqttBrokerError, MqttBrokerResult};
use crate::events::{BrokerEvent, EventHub};
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
    pub enhanced_auth: Mutex<Option<SharedEnhancedAuth>>,
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
            enhanced_auth: Mutex::new(None),
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
    ///
    /// `conn_id` identifies the *connection* being torn down: if a newer
    /// connection has since taken the client id over, this is a no-op (see
    /// [`SessionRegistry::detach`]).
    pub fn detach_session(&self, client_id: &str, conn_id: u64, graceful: bool) {
        let Some(will) = self.sessions.detach(client_id, conn_id, graceful) else {
            return;
        };

        let message = QueuedMessage {
            topic: will.topic,
            payload: will.payload,
            qos: will.qos,
            retain: will.retain,
        };
        if message.retain {
            self.retained.update(&message.to_publish(None, false));
        }
        self.sessions.fan_out(client_id, &message);
    }
}

/// How long a peer gets to finish a TLS / WebSocket handshake before it's
/// dropped. Without a bound, a client that opens a socket and goes silent
/// would hold a task and a file descriptor forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long an accept loop backs off after an `accept()` error before
/// trying again.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// What a started broker owns; present exactly while it's running.
struct Running {
    tasks: Vec<JoinHandle<()>>,
    port: u16,
    ws_port: Option<u16>,
    tls_port: Option<u16>,
}

impl Running {
    fn abort_tasks(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// A running (or not-yet-started) MQTT broker instance, speaking both MQTT
/// 3.1.1 and MQTT 5.0 — the protocol version is negotiated independently
/// per connection from each client's CONNECT packet.
///
/// Dropping a broker stops it (listeners are closed and live connections
/// told to shut down); there is no way to reach a broker that has no
/// remaining handle, so leaving it running would just leak its ports.
#[derive(uniffi::Object)]
pub struct MqttBroker {
    state: Arc<BrokerState>,
    /// Serialises `start`/`stop` so two concurrent calls can't both pass the
    /// "already running?" check. Held across `.await`, hence tokio's mutex.
    lifecycle: tokio::sync::Mutex<()>,
    running: Mutex<Option<Running>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MqttBroker {
    #[uniffi::constructor]
    pub fn new(config: MqttBrokerConfig) -> Self {
        MqttBroker {
            state: Arc::new(BrokerState::new(config)),
            lifecycle: tokio::sync::Mutex::new(()),
            running: Mutex::new(None),
        }
    }

    /// Register a pluggable authentication provider. Must be called before
    /// [`start`](Self::start) to affect connections made right away, but
    /// may be replaced at any time.
    pub fn set_auth_provider(&self, provider: Arc<dyn MqttAuthProvider>) {
        *self.state.auth_provider.lock_safe() = Some(provider);
    }

    /// Register the MQTT 5.0 enhanced-authentication provider (multi-round
    /// challenge/response during CONNECT). Without one, a CONNECT that names
    /// an `Authentication Method` is refused with "Bad authentication method".
    pub fn set_enhanced_auth_provider(&self, provider: Arc<dyn MqttEnhancedAuthProvider>) {
        *self.state.enhanced_auth.lock_safe() = Some(provider);
    }

    /// Register a listener for connect/disconnect/publish events.
    pub fn set_event_listener(&self, listener: Arc<dyn MqttBrokerEventListener>) {
        self.state.events.set_listener(listener);
    }

    /// Pull-style alternative to [`set_event_listener`](Self::set_event_listener)
    /// for runtimes that can't receive callbacks from Rust threads (Dart,
    /// Haskell): keep up to `capacity` events for [`next_event`](Self::next_event).
    /// `0` turns the queue off; when it fills up the oldest event is dropped.
    pub fn enable_event_queue(&self, capacity: u32) {
        self.state.events.queue.enable(capacity as usize);
    }

    /// The next queued [`BrokerEvent`], waiting up to `timeout_ms`
    /// milliseconds; `None` if none arrived. Requires
    /// [`enable_event_queue`](Self::enable_event_queue).
    pub async fn next_event(&self, timeout_ms: u32) -> Option<BrokerEvent> {
        self.state
            .events
            .queue
            .next(Duration::from_millis(u64::from(timeout_ms)))
            .await
    }

    /// Bind the listening socket(s) and start accepting connections. Also
    /// binds a second, MQTT-over-WebSocket listener on `config.ws_port` and
    /// a TLS listener on `config.tls` when those are configured.
    ///
    /// All-or-nothing: every socket is bound (and the TLS configuration
    /// validated) *before* anything starts accepting, so a failure — say
    /// the WebSocket port is taken — leaves the broker cleanly stopped
    /// instead of half-running with some listeners already live.
    pub async fn start(&self) -> MqttBrokerResult<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.running.lock_safe().is_some() {
            return Err(MqttBrokerError::AlreadyRunning);
        }
        let config = &self.state.config;

        // ── Bind everything first ────────────────────────────────────────
        let tls = match &config.tls {
            Some(tls_config) => {
                let acceptor =
                    crate::tls::build_acceptor(tls_config).map_err(MqttBrokerError::Tls)?;
                Some((acceptor, bind(&config.bind_address, tls_config.port).await?))
            }
            None => None,
        };
        let tcp = bind(&config.bind_address, config.port).await?;
        let ws = match config.ws_port {
            Some(port) => Some(bind(&config.bind_address, port).await?),
            None => None,
        };

        let mut running = Running {
            tasks: Vec::new(),
            port: local_port(&tcp, config.port),
            ws_port: ws.as_ref().map(|l| local_port(l, 0)),
            tls_port: tls.as_ref().map(|(_, l)| local_port(l, 0)),
        };

        // ── Then start accepting ─────────────────────────────────────────
        let state = self.state.clone();
        running
            .tasks
            .push(spawn_accept_loop(tcp, "tcp", move |stream, peer| {
                handle_connection(state.clone(), stream, peer)
            }));

        if let Some(listener) = ws {
            let state = self.state.clone();
            running
                .tasks
                .push(spawn_accept_loop(listener, "ws", move |stream, peer| {
                    accept_ws_connection(state.clone(), stream, peer)
                }));
        }

        if let Some((acceptor, listener)) = tls {
            let state = self.state.clone();
            running
                .tasks
                .push(spawn_accept_loop(listener, "tls", move |stream, peer| {
                    accept_tls_connection(state.clone(), acceptor.clone(), stream, peer)
                }));
        }

        let state = self.state.clone();
        let interval = config.redelivery_interval();
        let expiry = config.session_expiry();
        running.tasks.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // The first tick fires immediately; skip it so we don't sweep
            // an empty registry the instant the broker starts.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                state.sessions.retry_pending(interval);
                if let Some(ttl) = expiry {
                    let removed = state.sessions.sweep_expired(ttl);
                    if removed > 0 {
                        tracing::info!(removed, "expired offline sessions");
                    }
                }
            }
        }));

        *self.running.lock_safe() = Some(running);
        Ok(())
    }

    /// Stop the broker: close the listening sockets and ask every
    /// connected client's connection to shut down. Sessions that persist
    /// across disconnects (`clean_start = false`) are kept in memory, so a
    /// later [`start`](Self::start) resumes them. Calling `stop()` on a
    /// broker that isn't running is a no-op.
    pub async fn stop(&self) -> MqttBrokerResult<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.shutdown();
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.lock_safe().is_some()
    }

    /// Number of currently connected clients.
    pub fn client_count(&self) -> u32 {
        self.state.client_count()
    }

    /// The TCP port actually bound (useful when `config.port == 0` was used
    /// to request an ephemeral port, e.g. in tests).
    pub fn bound_port(&self) -> Option<u16> {
        self.running.lock_safe().as_ref().map(|r| r.port)
    }

    /// The WebSocket port actually bound, if `config.ws_port` is set.
    pub fn bound_ws_port(&self) -> Option<u16> {
        self.running.lock_safe().as_ref().and_then(|r| r.ws_port)
    }

    /// The TLS port actually bound, if `config.tls` is set.
    pub fn bound_tls_port(&self) -> Option<u16> {
        self.running.lock_safe().as_ref().and_then(|r| r.tls_port)
    }
}

impl MqttBroker {
    /// Synchronous core of `stop()`, shared with `Drop`.
    fn shutdown(&self) {
        if let Some(running) = self.running.lock_safe().take() {
            running.abort_tasks();
            self.state.sessions.shutdown_all();
        }
    }
}

impl Drop for MqttBroker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn bind(address: &str, port: u16) -> MqttBrokerResult<TcpListener> {
    Ok(TcpListener::bind((address, port)).await?)
}

/// The port `listener` is actually bound to (differs from the requested one
/// when `0` asked for an ephemeral port).
fn local_port(listener: &TcpListener, fallback: u16) -> u16 {
    listener.local_addr().map(|a| a.port()).unwrap_or(fallback)
}

/// Accept connections on `listener` forever, spawning `on_connection` for
/// each. One implementation for every transport (raw TCP, WebSocket, TLS):
/// they differ only in what they do with the accepted socket.
///
/// An `accept()` error (typically `EMFILE` under file-descriptor pressure,
/// or a connection reset between SYN and accept) is logged and retried
/// after a short pause instead of ending the loop — otherwise one
/// transient error would silently and permanently stop a listener while
/// [`MqttBroker::is_running`] kept reporting `true`.
fn spawn_accept_loop<F, Fut>(
    listener: TcpListener,
    transport: &'static str,
    mut on_connection: F,
) -> JoinHandle<()>
where
    F: FnMut(TcpStream, String) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    stream.set_nodelay(true).ok();
                    tokio::spawn(on_connection(stream, peer.to_string()));
                }
                Err(e) => {
                    tracing::warn!(error = %e, transport, "accept() failed; retrying");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                }
            }
        }
    })
}

/// Complete the TLS handshake, then hand off to the same
/// [`handle_connection`] every raw-TCP connection goes through — it's
/// generic over the byte transport, so an already-decrypted
/// `tokio_rustls::server::TlsStream` works exactly like a raw `TcpStream`.
async fn accept_tls_connection(
    state: Arc<BrokerState>,
    acceptor: tokio_rustls::TlsAcceptor,
    stream: TcpStream,
    peer: String,
) {
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
        Ok(Ok(tls_stream)) => handle_connection(state, tls_stream, peer).await,
        Ok(Err(e)) => tracing::debug!(%peer, error = %e, "TLS handshake failed"),
        Err(_) => tracing::debug!(%peer, "TLS handshake timed out"),
    }
}

/// Complete the WebSocket upgrade handshake (negotiating the `mqtt`
/// subprotocol per MQTT-5.0 §6.4.1), then hand off to the same
/// [`handle_connection`] every raw-TCP connection goes through.
async fn accept_ws_connection(state: Arc<BrokerState>, stream: TcpStream, peer: String) {
    // Cap a single WebSocket message/frame at the MQTT packet limit (plus
    // slack for framing), so the transport can't be used to buffer more than
    // the packet decoder itself would ever accept.
    let limit = state.config.max_packet_size_bytes().saturating_add(1024);
    let ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(limit),
        max_frame_size: Some(limit),
        ..Default::default()
    };

    // The `Err` side of `accept_hdr_async`'s callback contract is
    // tungstenite's own `ErrorResponse` type (an HTTP response) — its size
    // isn't ours to shrink, so this lint doesn't apply here.
    #[allow(clippy::result_large_err)]
    let handshake = tokio_tungstenite::accept_hdr_async_with_config(
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
        Some(ws_config),
    );
    let ws_stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            tracing::debug!(%peer, error = %e, "WebSocket handshake failed");
            return;
        }
        Err(_) => {
            tracing::debug!(%peer, "WebSocket handshake timed out");
            return;
        }
    };

    handle_connection(state, WsByteStream::new(ws_stream), peer).await;
}
