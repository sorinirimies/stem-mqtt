# mqtt-broker

An MQTT 3.1.1 / MQTT 5.0 broker, implemented in Rust and exposed to
Kotlin, Swift, Python, and other languages via
[UniFFI](https://mozilla.github.io/uniffi-rs/). Reuses the wire-protocol
codec from [`mqtt-client`](../mqtt-client/README.md)
(`mqtt_client::protocol`) instead of duplicating it.

## Installation

Published on crates.io as `stem-mqtt-broker` (not `mqtt-broker` — kept
consistent with `stem-mqtt-client`'s naming, see that crate's README for
why). The import path is unaffected:

```sh
cargo add stem-mqtt-broker
```

```rust,no_run
use mqtt_broker::{MqttBroker, MqttBrokerConfig};
```

Both client and broker bindings ship for Kotlin, Android, Swift, and Python.
Every other supported language ships both too — one UniFFI-generated package each. See the
[root README's Installation section](../../README.md#installation) for every
supported language.

## Layout

- [`src/broker.rs`](src/broker.rs) — `BrokerState`: composes the three
  subsystems below (only the operations that genuinely span more than one
  of them live here — will delivery on disconnect, retained-message replay
  on subscribe) — and `MqttBroker`, the thin UniFFI-exported handle:
  bind sockets, spawn per-connection tasks, `start()`/`stop()` lifecycle.
- [`src/registry.rs`](src/registry.rs) — `SessionRegistry`: owns the
  `client_id -> Session` map, per-client send/queue/QoS-2 bookkeeping, and
  fan-out (matching a published message to subscribers).
- [`src/retain.rs`](src/retain.rs) — `RetainStore`: the retained-message
  store, and only that.
- [`src/events.rs`](src/events.rs) — `EventHub`: connect/disconnect/publish
  notifications to the foreign-side listener.
- [`src/connection.rs`](src/connection.rs) — per-connection actor: reads and
  decodes packets off the socket, drives the CONNECT handshake, and dispatches
  PUBLISH/SUBSCRIBE/UNSUBSCRIBE/ack packets for one client. Calls straight
  into whichever subsystem above owns a given operation.
- [`src/session.rs`](src/session.rs) — per-client session state: subscriptions,
  in-flight QoS 1/2 packet ids, and (for non-clean sessions) the queued
  message backlog.
- [`src/topic.rs`](src/topic.rs) — topic name/filter matching (`+`/`#`
  wildcards), independent of everything else.
- [`src/ws.rs`](src/ws.rs) — MQTT-over-WebSocket transport adapter, so the
  same `connection.rs` logic drives both raw TCP and WebSocket connections.
- [`src/tls.rs`](src/tls.rs) — builds a `tokio_rustls::TlsAcceptor` from
  `BrokerTlsConfig` (server cert/key, optional mTLS client-CA) — accepted
  TLS streams feed into the same generic `connection.rs` handler too.
- [`src/config.rs`](src/config.rs) — `MqttBrokerConfig`, the pluggable
  `MqttAuthProvider` trait, and the `MqttBrokerEventListener` observer trait,
  all exported across the UniFFI boundary.
- [`src/bin/mqtt-broker.rs`](src/bin/mqtt-broker.rs) — standalone CLI binary.

## Running the CLI binary

```sh
cargo run -p stem-mqtt-broker --bin mqtt-broker -- --bind 0.0.0.0 --port 1883
```

```
Usage: mqtt-broker [OPTIONS]

Options:
      --bind <BIND>              Address to bind the listening socket to [default: 0.0.0.0]
      --port <PORT>               TCP port to listen on [default: 1883]
      --ws-port <WS_PORT>          Also accept MQTT-over-WebSocket connections on this port (needed for browser clients — see demo/web/)
      --allow-anonymous            Allow clients to connect without a username/password
      --max-clients <MAX_CLIENTS>  Maximum simultaneously connected clients (0 = unlimited)
      --log-level <LOG_LEVEL>      Log verbosity: error, warn, info, debug, trace [default: info]
```

Browser clients (and anything else that can't open a raw TCP socket) need
`--ws-port`:

```sh
cargo run -p stem-mqtt-broker --bin mqtt-broker -- --ws-port 8083
```

See [`demo/`](../../demo) for a full browser-based pub/sub demo webpage
that talks to the broker this way.

## Embedding as a library

```rust,no_run
use mqtt_broker::{MqttBroker, MqttBrokerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MqttBrokerConfig::new("127.0.0.1", 1883);
    let broker = MqttBroker::new(config);
    broker.start().await?;

    tokio::signal::ctrl_c().await?;
    broker.stop().await?;
    Ok(())
}
```

Plug in authentication and observability by implementing the UniFFI-exported
traits and registering them on the broker/config:

```rust,no_run
# use mqtt_broker::MqttAuthProvider;
struct FixedCreds;
impl MqttAuthProvider for FixedCreds {
    fn authenticate(&self, _client_id: String, username: Option<String>, password: Option<Vec<u8>>) -> bool {
        username.as_deref() == Some("demo") && password.as_deref() == Some(b"demo")
    }
}
```

## QoS in the bindings

The broker's bindings are self-contained: `QoS` in `MqttBrokerConfig::max_qos` and in the event listener is
the **broker's own** enum (`mqtt_broker::QoS`), with lossless `From` conversions to the client crate's
`mqtt_client::QoS`. In a language that imports both packages you therefore have two `QoS` types — use the
broker's for broker configuration/events and the client's for client calls.

## Limits and robustness

All of these are fields on `MqttBrokerConfig` (and CLI flags on `mqtt-broker`):

| Setting | Default | Effect |
| --- | --- | --- |
| `max_packet_size` / `--max-packet-size` | 1 MiB | A client announcing a bigger packet is disconnected as soon as the header is read — before the body is buffered. |
| `max_outbound_queue` / `--max-outbound-queue` | 4096 | Packets queued per client socket; a slow consumer past this has further deliveries dropped (QoS 1/2 are retried) instead of growing memory without bound. |
| `max_clients` / `--max-clients` | unlimited | Checked atomically; a client taking over its own session doesn't count against it. |
| `redelivery_interval_secs` | 5 | Resend (DUP=1) interval for unacked QoS 1/2 deliveries. |
| `session_expiry_secs` | 0 (never) | Discard persistent (`clean_start = false`) sessions — with their subscriptions and queued messages — that stay offline this long, so clients that never return can't leak memory. |

Behaviour worth knowing: `start()` is all-or-nothing and serialised; `stop()` closes live
connections (without publishing Last Wills) and keeps persistent sessions; dropping a broker stops
it; a client id taken over by a newer connection never disturbs its replacement; an empty client id
with `clean_start = false` is refused; `SIGTERM` shuts the CLI down gracefully. Foreign callbacks that panic or throw
are contained and never kill a connection, and an auth provider that panics means "denied".

## Subscriptions and routing

- **Topic index:** subscriptions live in a topic-level trie, so a PUBLISH finds its subscribers by walking
  the topic's own levels instead of scanning every session's filters.
- **Shared subscriptions:** `$share/<group>/<filter>` delivers each message to **one** member of the group
  (round-robin, preferring connected members); ordinary subscribers still receive everything. Retained
  messages are not replayed to shared subscriptions (MQTT 5 §4.8.2).

## Authentication

- `MqttAuthProvider` — username/password, evaluated once per CONNECT (off the async workers; a panic = denied).
- `MqttEnhancedAuthProvider` — MQTT 5 *enhanced* authentication: a multi-round challenge/response (SCRAM,
  Kerberos, OAuth, …) during CONNECT. The provider's `step(client_id, method, data, round)` returns an
  `EnhancedAuthStep { outcome: Continue | Success | Failure, data }`; the broker relays challenges as AUTH
  packets and returns `data` on success in the CONNACK. A CONNECT naming a method with no provider
  registered is refused (`0x8C`). The matching client side is `MqttAuthHandler`.

## Pull-style events

For runtimes that can't receive callbacks (Dart, Haskell): `enable_event_queue(capacity)` then
`next_event(timeout_ms)` returns a `BrokerEvent` (`ClientConnected`, `ClientDisconnected`,
`MessagePublished`) or `None` on timeout. Oldest events are dropped when the queue is full.

## Foreign-language bindings

Same pattern as `mqtt-client` — Kotlin, Swift, Python, Go, C#, Java, Dart, Node.js and Haskell
(see the [root README](../../README.md#generating-foreign-language-bindings) for which generators
support which features):

```sh
nu ../../scripts/generate_bindings.nu kotlin
```

## Examples

```sh
cargo run -p stem-mqtt-broker --example simple_broker   # bare-minimum embedded broker
cargo run -p stem-mqtt-broker --example auth_broker      # + MqttAuthProvider and MqttBrokerEventListener
```

## Testing

```sh
cargo test -p stem-mqtt-broker
```

[`tests/integration.rs`](tests/integration.rs) drives a real `MqttClient`
against a real `MqttBroker` over a loopback TCP socket bound to an
OS-assigned ephemeral port (so tests can run in parallel): QoS 0/1/2
publish/subscribe, retained messages, wildcard subscriptions, unsubscribe,
auth accept/reject, and Last-Will delivery on an ungraceful disconnect.
