# Rust

The native API: no bindings layer, no callbacks through FFI — `tokio` async all the way. The client crate
also hosts the wire-protocol codec (`mqtt_client::protocol`) that the broker reuses. See the
[shared concepts](README.md) for the option records and semantics.

## Install

```sh
cargo add stem-mqtt-client stem-mqtt-broker anyhow
cargo add tokio --features full
```

The crates are published as `stem-mqtt-client` / `stem-mqtt-broker` (`mqtt-client` / `mqtt-broker` are
taken by unrelated crates) but the library names are unchanged: `use mqtt_client::…` and
`use mqtt_broker::…`. API docs: [docs.rs/stem-mqtt-client](https://docs.rs/stem-mqtt-client) and
[docs.rs/stem-mqtt-broker](https://docs.rs/stem-mqtt-broker).

Just want the broker binary? `cargo install stem-mqtt-broker` installs `mqtt-broker`
(see [Running the broker](../../README.md#running-the-broker)).

## Requirements

Rust 1.75 or newer. Linux, macOS and Windows. No system libraries (TLS is `rustls`).

## Quick start

An embedded broker and a client:

```rust,no_run
use mqtt_broker::{MqttBroker, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let broker = MqttBroker::new(MqttBrokerConfig::new("127.0.0.1", 0)); // 0 = any free port
    broker.start().await?;
    let port = broker.bound_port().expect("bound");

    let client = MqttClient::new(ConnectOptions::new("127.0.0.1", port, "my-client", MqttVersion::V5));
    client.connect().await?;
    client.subscribe("stem/demo".into(), QoS::AtLeastOnce).await?;
    client.publish("stem/demo".into(), b"hello".to_vec(), QoS::AtLeastOnce, false).await?;

    client.disconnect().await?;
    broker.stop().await?;
    Ok(())
}
```

## Client

`ConnectOptions::new(host, port, client_id, version)` fills sensible defaults (clean start, 30 s
keep-alive, no TLS, no reconnect); every field is `pub`, so adjust by assignment:

```rust
let mut options = ConnectOptions::new("broker.example.com", 8883, "sensor-7", MqttVersion::V5);
options.username = Some("sensor".into());
options.password = Some(b"s3cret".to_vec());
options.auto_reconnect = true;
options.tls = Some(TlsOptions::new());          // bundled Mozilla roots
options.will = Some(WillOptions {
    topic: "sensors/7/status".into(),
    payload: b"offline".to_vec(),
    qos: QoS::AtLeastOnce,
    retain: true,
});
```

**Receiving** — implement `MqttMessageListener` and register it *before* `connect()`:

```rust
use std::sync::Arc;
use mqtt_client::{MqttMessage, MqttMessageListener};

struct Printer;
impl MqttMessageListener for Printer {
    fn on_message(&self, message: MqttMessage) {
        println!("{}: {}", message.topic, String::from_utf8_lossy(&message.payload));
    }
    fn on_disconnected(&self, reason: String) { eprintln!("disconnected: {reason}"); }
}
client.set_message_listener(Arc::new(Printer));
```

…or pull: `client.enable_message_queue(64)` then `client.next_message(5000).await` →
`Option<MqttMessage>`. `MqttMessage` carries `topic`, `payload`, `qos` and `retain`.

`publish(topic, payload, qos, retain)` returns once the QoS handshake is complete (QoS 1: PUBACK,
QoS 2: the full PUBREC/PUBREL/PUBCOMP exchange). `unsubscribe(filter)` and `is_connected()` complete
the API; `last_disconnect_reason()` explains the most recent drop.

## Broker

```rust
let mut config = MqttBrokerConfig::new("0.0.0.0", 1883);
config.ws_port = Some(8083);          // browsers
config.allow_anonymous = false;
config.max_clients = 1000;
let broker = MqttBroker::new(config);
broker.set_event_listener(Arc::new(MyListener));   // optional
broker.start().await?;
```

`bound_port()`, `bound_ws_port()`, `bound_tls_port()` report the ports actually bound (useful with `0`).
`start()` returns `MqttBrokerError::AlreadyRunning` if called twice. The standalone binary is the
same code: [`crates/mqtt-broker/src/bin/mqtt-broker.rs`](../../crates/mqtt-broker/src/bin/mqtt-broker.rs).

## Authentication

```rust
use mqtt_broker::MqttAuthProvider;

struct Fixed;
impl MqttAuthProvider for Fixed {
    fn authenticate(&self, _client_id: String, username: Option<String>, password: Option<Vec<u8>>) -> bool {
        matches!((username.as_deref(), password.as_deref()), (Some("demo"), Some(b"demo")))
    }
}
broker.set_auth_provider(Arc::new(Fixed));
```

A complete runnable pair: [`auth_broker`](../../crates/mqtt-broker/examples/auth_broker.rs) and
[`auth_client`](../../crates/mqtt-client/examples/auth_client.rs). MQTT 5 enhanced authentication:
implement `MqttEnhancedAuthProvider` (broker) and `MqttAuthHandler` (client).

## TLS

Certificates are PEM bytes. Server side:

```rust
use mqtt_broker::BrokerTlsConfig;
config.tls = Some(BrokerTlsConfig {
    port: 8883,
    cert_pem: std::fs::read("server.pem")?,
    key_pem: std::fs::read("server.key")?,
    client_ca_pem: None, // Some(ca) => require client certificates (mTLS)
});
```

Client side: `options.tls = Some(TlsOptions { ca_cert_pem: Some(std::fs::read("ca.pem")?), ..TlsOptions::new() })`.

## Language notes

- This is the only language where `QoS` from the two crates are convertible: `mqtt_broker::QoS` and
  `mqtt_client::QoS` implement `From` for each other.
- All `async` methods are `tokio` futures; you need a `tokio` runtime (`#[tokio::main]`).
- The same crates build as `cdylib`/`staticlib` for the bindings; that adds no cost to Rust users.

## Building from source

```sh
git clone https://github.com/sorinirimies/stem-mqtt && cd stem-mqtt
cargo build --workspace
cargo test --workspace
cargo run -p stem-mqtt-client --example pub_sub     # with a broker running
```

## Troubleshooting

- **`connection refused: reason code 0x86`** — the broker's auth provider (or `allow_anonymous = false`)
  rejected the credentials.
- **`operation timed out`** — a QoS 1/2 packet wasn't acknowledged; raise `operation_timeout_secs`.
- **`protocol error` on publish** — topics can't contain wildcards (`+`, `#`) when publishing.
- **Messages stop after a network blip** — set `auto_reconnect = true`; subscriptions are replayed.

## More

[Examples](../../README.md#quick-start-rust) · [`crates/mqtt-client/README.md`](../../crates/mqtt-client/README.md) ·
[`crates/mqtt-broker/README.md`](../../crates/mqtt-broker/README.md) · [fuzz targets](../../fuzz)
