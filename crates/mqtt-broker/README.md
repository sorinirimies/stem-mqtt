# mqtt-broker

A full MQTT 3.1.1 / MQTT 5.0 broker, implemented in Rust and exposed to
Kotlin, Swift, Python, and other languages via
[UniFFI](https://mozilla.github.io/uniffi-rs/). Reuses the wire-protocol
codec from [`mqtt-client`](../mqtt-client/README.md)
(`mqtt_client::protocol`) instead of duplicating it.

## Layout

- [`src/broker.rs`](src/broker.rs) — `MqttBroker`: accepts TCP connections,
  owns the shared topic tree and session table, `start()`/`stop()` lifecycle.
- [`src/connection.rs`](src/connection.rs) — per-connection actor: reads and
  decodes packets off the socket, drives the CONNECT handshake, and dispatches
  PUBLISH/SUBSCRIBE/UNSUBSCRIBE/ack packets for one client.
- [`src/session.rs`](src/session.rs) — per-client session state: subscriptions,
  in-flight QoS 1/2 packet ids, and (for non-clean sessions) the queued
  message backlog.
- [`src/topic.rs`](src/topic.rs) — topic-filter tree for matching PUBLISH
  topics against subscriptions (`+`/`#` wildcards) and storing retained
  messages.
- [`src/config.rs`](src/config.rs) — `MqttBrokerConfig`, the pluggable
  `MqttAuthProvider` trait, and the `MqttBrokerEventListener` observer trait,
  all exported across the UniFFI boundary.
- [`src/bin/mqtt-broker.rs`](src/bin/mqtt-broker.rs) — standalone CLI binary.

## Running the CLI binary

```sh
cargo run -p mqtt-broker --bin mqtt-broker -- --bind 0.0.0.0 --port 1883
```

```
Usage: mqtt-broker [OPTIONS]

Options:
      --bind <BIND>              Address to bind the listening socket to [default: 0.0.0.0]
      --port <PORT>               TCP port to listen on [default: 1883]
      --allow-anonymous            Allow clients to connect without a username/password
      --max-clients <MAX_CLIENTS>  Maximum simultaneously connected clients (0 = unlimited)
      --log-level <LOG_LEVEL>      Log verbosity: error, warn, info, debug, trace [default: info]
```

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

## Foreign-language bindings

Same pattern as `mqtt-client`:

```sh
../../scripts/generate-bindings.sh kotlin
```

## Examples

```sh
cargo run -p mqtt-broker --example simple_broker   # bare-minimum embedded broker
cargo run -p mqtt-broker --example auth_broker      # + MqttAuthProvider and MqttBrokerEventListener
```

## Testing

```sh
cargo test -p mqtt-broker
```

[`tests/integration.rs`](tests/integration.rs) drives a real `MqttClient`
against a real `MqttBroker` over a loopback TCP socket bound to an
OS-assigned ephemeral port (so tests can run in parallel): QoS 0/1/2
publish/subscribe, retained messages, wildcard subscriptions, unsubscribe,
auth accept/reject, and Last-Will delivery on an ungraceful disconnect.
