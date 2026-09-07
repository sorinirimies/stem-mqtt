//! stem-mqtt dashboard: a Rust + Topcoat (<https://github.com/tokio-rs/topcoat>)
//! — the Rust full-stack web framework, *not* the unrelated Topcoat CSS
//! framework `demo/web/` uses) web UI for exercising a running
//! `mqtt-broker`: connect multiple clients at once (any mix of MQTT 3.1.1
//! and 5.0), subscribe/publish on any topic at any QoS, and watch messages
//! arrive live.
//!
//! Architecture note: the "New connection" form on the initial page load
//! uses Topcoat's full reactive system (signals + a `#[procedure]` call).
//! Everything *dynamically added afterwards* — a new client's card, live
//! feed entries — is plain HTML pushed over one shared SSE stream
//! (Datastar's `PatchElements`) with plain inline `onclick`/`fetch()`
//! wiring instead: Topcoat's own signal/procedure hydration is computed
//! against the page as rendered by the *original* request, so it can't
//! apply to content injected afterwards from an unrelated stream.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::Stream;
use mqtt_client::{ConnectOptions, MqttClient, MqttMessage, MqttMessageListener, MqttVersion, QoS};
use tokio::sync::broadcast;
use topcoat::{
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::{app_context, Cx},
    datastar::{ElementPatchMode, PatchElements},
    router::{
        content::sse::{Event as SseEvent, Sse},
        page, path_param, query_params, route, Router, RouterBuilderDiscoverExt,
    },
    runtime::{self, procedure, Event},
    view::view,
    Result,
};

// ── Shared state ──────────────────────────────────────────────────────────

/// One live MQTT connection the dashboard is managing.
struct Connection {
    client: Arc<MqttClient>,
}

#[derive(Clone)]
enum DashboardEvent {
    /// A new client card to append to `#connections`.
    ClientConnected { html: String },
    /// A new `<li>` to append to that client's `#feed-{client_id}`.
    Message { client_id: String, html: String },
}

struct AppState {
    connections: Mutex<HashMap<String, Connection>>,
    events: broadcast::Sender<DashboardEvent>,
}

impl AppState {
    fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(1024);
        Arc::new(AppState {
            connections: Mutex::new(HashMap::new()),
            events: tx,
        })
    }
}

/// Forwards every message (and disconnect notice) for one client into the
/// shared event bus, tagged with which client it came from so the browser
/// can route it into the right feed.
struct DashboardListener {
    client_id: String,
    events: broadcast::Sender<DashboardEvent>,
}

impl MqttMessageListener for DashboardListener {
    fn on_message(&self, message: MqttMessage) {
        let html = format!(
            "<li class=\"recv\"><strong>{}</strong> <span class=\"qos\">qos {}</span> {}{}</li>",
            escape_html(&message.topic),
            message.qos as u8,
            escape_html(&String::from_utf8_lossy(&message.payload)),
            if message.retain {
                " <em>[retained]</em>"
            } else {
                ""
            },
        );
        let _ = self.events.send(DashboardEvent::Message {
            client_id: self.client_id.clone(),
            html,
        });
    }

    fn on_disconnected(&self, reason: String) {
        let html = format!(
            "<li class=\"disconnected\">disconnected: {}</li>",
            escape_html(&reason)
        );
        let _ = self.events.send(DashboardEvent::Message {
            client_id: self.client_id.clone(),
            html,
        });
    }
}

/// Minimal HTML-escaping for plain-text content injected via raw strings
/// (everything dynamically appended after page load — see the module docs
/// for why this doesn't go through `view!`).
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn parse_version(s: &str) -> MqttVersion {
    if s == "3.1.1" {
        MqttVersion::V311
    } else {
        MqttVersion::V5
    }
}

fn parse_qos(s: &str) -> QoS {
    match s {
        "1" => QoS::AtLeastOnce,
        "2" => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

// ── Entry point ───────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();
    let state = AppState::new();

    println!(
        "stem-mqtt dashboard listening on http://{}:{}",
        std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        std::env::var("PORT").unwrap_or_else(|_| "3000".into()),
    );

    topcoat::start(
        Router::builder()
            .assets(AssetBundle::load().unwrap())
            .discover()
            .app_context(state)
            .build(),
    )
    .await
    .unwrap();
}

// ── Page shell + "new connection" form (Topcoat reactive system) ─────────

#[page("/")]
async fn home() -> Result {
    view! {
        <!DOCTYPE html>
        <html>
            <head>
                <title>"stem-mqtt dashboard"</title>
                topcoat::dev::script()
                runtime::script()
                <style>(DASHBOARD_CSS)</style>
            </head>
            <body>
                signal dark_mode = false;
                <div class="page" :data-theme=$(if dark_mode.get() { "dark" } else { "light" })>
                    <div class="topbar">
                        <h1>"stem-mqtt dashboard"</h1>
                        <button class="theme-toggle" @click=$(|_e: Event| dark_mode.toggle())>
                            $(if dark_mode.get() { "☀️ Light mode" } else { "🌙 Dark mode" })
                        </button>
                    </div>
                    <p class="subtitle">
                        "Rust + "
                        <a href="https://github.com/tokio-rs/topcoat">"Topcoat"</a>
                        " — connect any number of clients, any mix of MQTT 3.1.1/5.0, "
                        "chat over any topic at any QoS, and watch the broker's live log."
                    </p>

                    <section class="card">
                        <h2>"New connection"</h2>
                        signal host = String::from("127.0.0.1");
                        signal port = String::from("1883");
                        signal client_id = String::from("dashboard-client");
                        signal version = String::from("5.0");
                        signal connect_status = String::new();

                        <div class="row">
                            <label>"Host" <input :value=$(host.get()) @input=$(|e: Event| host.set(e.target.value))></label>
                            <label>"Port" <input :value=$(port.get()) @input=$(|e: Event| port.set(e.target.value))></label>
                            <label>"Client ID" <input :value=$(client_id.get()) @input=$(|e: Event| client_id.set(e.target.value))></label>
                            <label>
                                "MQTT version"
                                <select @change=$(|e: Event| version.set(e.target.value))>
                                    <option value="5.0">"MQTT 5.0"</option>
                                    <option value="3.1.1">"MQTT 3.1.1"</option>
                                </select>
                            </label>
                            <button
                                @click=$(async |_e| {
                                    let status = connect(
                                        host.get(),
                                        port.get(),
                                        client_id.get(),
                                        version.get(),
                                    ).await;
                                    connect_status.set(status);
                                })
                            >
                                "Connect"
                            </button>
                        </div>
                        <p><span>$(connect_status.get())</span></p>
                    </section>

                    <h2>"Connections"</h2>
                    <div id="connections"></div>
                </div>

                <script>(topcoat::view::Unescaped::new_unchecked(STREAM_SCRIPT))</script>
            </body>
        </html>
    }
}

const DASHBOARD_CSS: &str = r#"
:root {
    --bg: #f6f7fb;
    --fg: #1a1a2e;
    --muted: #5a5a6a;
    --card-bg: #ffffff;
    --border: #ddd;
    --input-bg: #ffffff;
    --input-border: #ccc;
    --accent: #3d5afe;
    --accent-hover: #304ffe;
    --secondary: #757575;
    --secondary-hover: #616161;
    --feed-border: #eee;
    --danger: #c62828;
    --qos-color: #888;
    --sent-bg: #3d5afe;
    --sent-fg: #ffffff;
    --recv-bg: #eef0f7;
    --recv-fg: #1a1a2e;
}
body[data-theme="dark"], .page[data-theme="dark"] {
    --bg: #14151c;
    --fg: #e8e8f0;
    --muted: #a0a0b4;
    --card-bg: #1c1e29;
    --border: #333646;
    --input-bg: #23252f;
    --input-border: #3a3d4d;
    --accent: #7c8cff;
    --accent-hover: #96a3ff;
    --secondary: #4a4d5c;
    --secondary-hover: #5a5d6e;
    --feed-border: #2a2c38;
    --danger: #ff6b6b;
    --qos-color: #8a8da0;
    --sent-bg: #4a5aff;
    --sent-fg: #ffffff;
    --recv-bg: #262837;
    --recv-fg: #e8e8f0;
}
* { box-sizing: border-box; }
body { font-family: -apple-system, system-ui, sans-serif; margin: 0; padding: 0; background: var(--bg); }
body:has(.page[data-theme="dark"]) { background: #14151c; }
.page { max-width: 900px; margin: 0 auto; padding: 2rem 1rem; color: var(--fg); background: var(--bg); min-height: 100vh; transition: background 0.15s ease, color 0.15s ease; }
h1 { margin-bottom: 0.25rem; }
.topbar { display: flex; align-items: center; justify-content: space-between; gap: 1rem; flex-wrap: wrap; }
.subtitle { color: var(--muted); margin-top: 0; }
.card { border: 1px solid var(--border); border-radius: 8px; padding: 1rem 1.25rem; margin-bottom: 1.5rem; background: var(--card-bg); }
.row { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: flex-end; }
label { display: flex; flex-direction: column; font-size: 0.85rem; color: var(--muted); gap: 0.25rem; }
input, select { padding: 0.4rem 0.5rem; border: 1px solid var(--input-border); border-radius: 4px; font-size: 0.95rem; background: var(--input-bg); color: var(--fg); }
button { padding: 0.5rem 1rem; border: none; border-radius: 4px; background: var(--accent); color: white; cursor: pointer; font-size: 0.95rem; }
button:hover { background: var(--accent-hover); }
button.secondary { background: var(--secondary); }
button.secondary:hover { background: var(--secondary-hover); }
button.theme-toggle { background: transparent; color: var(--fg); border: 1px solid var(--border); }
button.theme-toggle:hover { background: var(--recv-bg); }
.chat-log { list-style: none; padding: 0.5rem; margin: 0.5rem 0 0 0; max-height: 260px; overflow-y: auto; border-top: 1px solid var(--feed-border); display: flex; flex-direction: column; gap: 0.4rem; background: var(--bg); border-radius: 6px; }
.chat-log li { max-width: 80%; padding: 0.4rem 0.6rem; border-radius: 10px; font-size: 0.9rem; line-height: 1.35; word-wrap: break-word; }
.chat-log li.recv { align-self: flex-start; background: var(--recv-bg); color: var(--recv-fg); border-bottom-left-radius: 2px; }
.chat-log li.sent { align-self: flex-end; background: var(--sent-bg); color: var(--sent-fg); border-bottom-right-radius: 2px; }
.chat-log li.system { align-self: center; max-width: 100%; background: transparent; color: var(--muted); font-style: italic; font-size: 0.8rem; padding: 0.2rem 0; }
.chat-log li.disconnected { align-self: center; max-width: 100%; background: transparent; color: var(--danger); font-style: italic; }
.qos { opacity: 0.7; font-size: 0.8rem; }
"#;

/// Hand-rolled SSE client for `/stream`, wired with a plain synchronous
/// (non-module) inline `<script>` instead of Datastar's
/// `data-on:load="@get('/stream')"` action. That action binds to the
/// browser's real `load` event, but Datastar's own module script (loaded
/// from a CDN) frequently finishes booting *after* `load` has already
/// fired -- module scripts + cross-origin fetches race that event in
/// practice -- so the very first `/stream` subscription is silently
/// missed and no client card or feed entry ever appears. This script runs
/// synchronously while `<body>` is parsed, so there is no such race. It
/// speaks the same `datastar-patch-elements` SSE payload topcoat's
/// `PatchElements` sends (a `selector ...` line, a `mode ...` line, then
/// one or more `elements ...` lines), just parsed by hand instead of by
/// Datastar's client library.
const STREAM_SCRIPT: &str = r#"
(function () {
    function toFragment(html) {
        var wrapper = document.createElement('div');
        wrapper.innerHTML = html;
        var frag = document.createDocumentFragment();
        var nodes = Array.prototype.slice.call(wrapper.childNodes);
        for (var i = 0; i < nodes.length; i++) {
            var n = nodes[i];
            // <script> elements parsed via innerHTML/insertAdjacentHTML are
            // inert and never execute; re-create each as a real <script>
            // element (which *does* execute once appended) so the per-card
            // onclick wiring in render_client_card's <script> block runs.
            if (n.nodeType === 1 && n.tagName === 'SCRIPT') {
                var s = document.createElement('script');
                for (var j = 0; j < n.attributes.length; j++) {
                    s.setAttribute(n.attributes[j].name, n.attributes[j].value);
                }
                s.textContent = n.textContent;
                frag.appendChild(s);
            } else {
                frag.appendChild(n);
            }
        }
        return frag;
    }

    var es = new EventSource('/stream');
    es.addEventListener('datastar-patch-elements', function (e) {
        var lines = e.data.split('\n');
        var selector = null;
        var mode = 'outer';
        var htmlLines = [];
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i];
            if (selector === null && line.indexOf('selector ') === 0) {
                selector = line.slice(9);
            } else if (line.indexOf('mode ') === 0) {
                mode = line.slice(5);
            } else if (line.indexOf('elements ') === 0) {
                htmlLines.push(line.slice(9));
            } else {
                htmlLines.push(line);
            }
        }
        if (!selector) return;
        var target = document.querySelector(selector);
        if (!target) return;
        var frag = toFragment(htmlLines.join('\n'));
        if (mode === 'append') {
            target.appendChild(frag);
        } else if (mode === 'prepend') {
            target.insertBefore(frag, target.firstChild);
        } else {
            target.innerHTML = '';
            target.appendChild(frag);
        }
    });
})();
"#;

#[procedure]
async fn connect(
    cx: &Cx,
    host: String,
    port: String,
    client_id: String,
    version: String,
) -> Result<String> {
    let state: &Arc<AppState> = app_context(cx);
    let Ok(port_num) = port.parse::<u16>() else {
        return Ok("invalid port".into());
    };
    if state.connections.lock().unwrap().contains_key(&client_id) {
        return Ok(format!("client id {:?} is already connected", client_id));
    }

    let options = ConnectOptions::new(
        host.clone(),
        port_num,
        client_id.clone(),
        parse_version(&version),
    );
    let client = Arc::new(MqttClient::new(options));
    client.set_message_listener(Arc::new(DashboardListener {
        client_id: client_id.clone(),
        events: state.events.clone(),
    }));

    match client.connect().await {
        Ok(result) => {
            state
                .connections
                .lock()
                .unwrap()
                .insert(client_id.clone(), Connection { client });

            let card = render_client_card(&client_id, &host, port_num, &version);
            let _ = state
                .events
                .send(DashboardEvent::ClientConnected { html: card });
            Ok(format!(
                "connected (session_present={})",
                result.session_present
            ))
        }
        Err(e) => Ok(format!("connect failed: {e}")),
    }
}

/// Plain HTML (not `view!`) with inline `onclick`/`fetch()` wiring — see the
/// module docs for why dynamically-appended content can't use Topcoat's own
/// signal/procedure hydration.
fn render_client_card(client_id: &str, host: &str, port: u16, version: &str) -> String {
    let id = escape_html(client_id);
    format!(
        r##"<div class="card" id="client-{id}">
    <h3>{id} <span class="qos">{host}:{port} &middot; MQTT {version}</span></h3>
    <div class="row">
        <label>"Topic filter"<input id="sub-topic-{id}" value="stem-mqtt/dashboard/#"></label>
        <label>"QoS"
            <select id="sub-qos-{id}">
                <option value="0">0</option>
                <option value="1" selected>1</option>
                <option value="2">2</option>
            </select>
        </label>
        <button onclick="stemMqttSubscribe('{id}')">"Subscribe"</button>
    </div>
    <ul class="chat-log" id="feed-{id}"></ul>
    <div class="row">
        <label>"Topic"<input id="pub-topic-{id}" value="stem-mqtt/dashboard/hello"></label>
        <label>"Message"<input id="pub-payload-{id}" value="hello from the dashboard"></label>
        <label>"QoS"
            <select id="pub-qos-{id}">
                <option value="0">0</option>
                <option value="1" selected>1</option>
                <option value="2">2</option>
            </select>
        </label>
        <label><input type="checkbox" id="pub-retain-{id}"> "retain"</label>
        <button onclick="stemMqttPublish('{id}')">"Send"</button>
        <button class="secondary" onclick="stemMqttDisconnect('{id}')">"Disconnect"</button>
    </div>
</div>
<script>
if (!window.stemMqttSubscribe) {{
    window.stemMqttSubscribe = function(id) {{
        const topic = encodeURIComponent(document.getElementById('sub-topic-' + id).value);
        const qos = document.getElementById('sub-qos-' + id).value;
        fetch('/connections/' + id + '/subscribe?topic=' + topic + '&qos=' + qos, {{ method: 'POST' }});
    }};
    window.stemMqttPublish = function(id) {{
        const topicInput = document.getElementById('pub-topic-' + id);
        const payloadInput = document.getElementById('pub-payload-' + id);
        const topic = encodeURIComponent(topicInput.value);
        const payload = encodeURIComponent(payloadInput.value);
        const qos = document.getElementById('pub-qos-' + id).value;
        const retain = document.getElementById('pub-retain-' + id).checked;
        fetch('/connections/' + id + '/publish?topic=' + topic + '&payload=' + payload + '&qos=' + qos + '&retain=' + retain, {{ method: 'POST' }});
    }};
    window.stemMqttDisconnect = function(id) {{
        fetch('/connections/' + id + '/disconnect', {{ method: 'POST' }}).then(() => {{
            const el = document.getElementById('client-' + id);
            if (el) el.remove();
        }});
    }};
}}
</script>"##
    )
}

// ── Plain REST-ish routes for dynamically-added cards ────────────────────

#[path_param]
struct ClientId(str);

#[query_params(error = bad_request)]
struct SubscribeQuery {
    topic: String,
    qos: String,
}

#[route(POST "/connections/{client_id}/subscribe")]
async fn subscribe<'a>(cx: &'a Cx) -> Result<&'static str> {
    let state: &Arc<AppState> = app_context(cx);
    let id = path_param::<ClientId>(cx);
    let query = query_params::<SubscribeQuery>(cx)?;

    let client = {
        let conns = state.connections.lock().unwrap();
        conns.get(id).map(|c| c.client.clone())
    };
    let Some(client) = client else {
        return Ok("ok");
    };
    let qos = parse_qos(&query.qos);
    let html = match client.subscribe(query.topic.clone(), qos).await {
        Ok(result) if result.reason_code < 0x80 => format!(
            "<li class=\"system\">subscribed to <strong>{}</strong> (qos {})</li>",
            escape_html(&query.topic),
            qos as u8,
        ),
        Ok(result) => format!(
            "<li class=\"disconnected\">subscribe to {} rejected (reason 0x{:02x})</li>",
            escape_html(&query.topic),
            result.reason_code,
        ),
        Err(e) => format!(
            "<li class=\"disconnected\">subscribe to {} failed: {}</li>",
            escape_html(&query.topic),
            escape_html(&e.to_string()),
        ),
    };
    let _ = state.events.send(DashboardEvent::Message {
        client_id: id.to_owned(),
        html,
    });
    Ok("ok")
}

#[query_params(error = bad_request)]
struct PublishQuery {
    topic: String,
    payload: String,
    qos: String,
    retain: String,
}

#[route(POST "/connections/{client_id}/publish")]
async fn publish<'a>(cx: &'a Cx) -> Result<&'static str> {
    let state: &Arc<AppState> = app_context(cx);
    let id = path_param::<ClientId>(cx);
    let query = query_params::<PublishQuery>(cx)?;

    let client = {
        let conns = state.connections.lock().unwrap();
        conns.get(id).map(|c| c.client.clone())
    };
    let Some(client) = client else {
        return Ok("ok");
    };
    let qos = parse_qos(&query.qos);
    let retain = query.retain == "true";
    let html = match client
        .publish(
            query.topic.clone(),
            query.payload.clone().into_bytes(),
            qos,
            retain,
        )
        .await
    {
        Ok(()) => format!(
            "<li class=\"sent\">→ <strong>{}</strong> <span class=\"qos\">qos {}</span> {}{}</li>",
            escape_html(&query.topic),
            qos as u8,
            escape_html(&query.payload),
            if retain { " <em>[retained]</em>" } else { "" },
        ),
        Err(e) => format!(
            "<li class=\"disconnected\">publish to {} failed: {}</li>",
            escape_html(&query.topic),
            escape_html(&e.to_string()),
        ),
    };
    let _ = state.events.send(DashboardEvent::Message {
        client_id: id.to_owned(),
        html,
    });
    Ok("ok")
}

#[route(POST "/connections/{client_id}/disconnect")]
async fn disconnect(cx: &Cx) -> Result<&'static str> {
    let state: &Arc<AppState> = app_context(cx);
    let id = path_param::<ClientId>(cx);

    let client = state
        .connections
        .lock()
        .unwrap()
        .remove(id)
        .map(|c| c.client);
    if let Some(client) = client {
        let _ = client.disconnect().await;
    }
    Ok("ok")
}

// ── Live SSE stream: multiplexes every client's messages + new cards ─────

#[route(GET "/stream")]
async fn stream(cx: &Cx) -> Result<Sse<impl Stream<Item = Result<SseEvent>> + use<>>> {
    let state: &Arc<AppState> = app_context(cx);
    let rx = state.events.subscribe();

    let events = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(DashboardEvent::ClientConnected { html }) => {
                    let event = PatchElements::new(html)
                        .selector("#connections")
                        .mode(ElementPatchMode::Append)
                        .into();
                    return Some((Ok(event), rx));
                }
                Ok(DashboardEvent::Message { client_id, html }) => {
                    let event = PatchElements::new(html)
                        .selector(format!("#feed-{}", escape_html(&client_id)))
                        .mode(ElementPatchMode::Append)
                        .into();
                    return Some((Ok(event), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(Sse::new(events))
}
