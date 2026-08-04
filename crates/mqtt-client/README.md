# mqtt-client

An async, `tokio`-based MQTT 3.1.1 / MQTT 5.0 client, and the wire-protocol
codec shared with [`mqtt-broker`](../mqtt-broker/README.md). Exposed to
Kotlin, Swift, Python, and other languages via
[UniFFI](https://mozilla.github.io/uniffi-rs/).

## Layout

- [`src/protocol`](src/protocol) — pure, synchronous, allocation-friendly
  codec for the MQTT wire format. No networking or async dependencies, so
  it is unit-tested byte-for-byte and reused as-is by `mqtt-broker`.
  - `varint` — MQTT variable byte integer encoding.
  - `packet` — the top-level `Packet` enum, fixed header, and remaining-length
    framing/dispatch.
  - `connect`, `publish`, `subscribe`, `ack` — per-packet-type bodies.
  - `properties` — MQTT 5.0 properties (user properties, content type,
    message expiry, etc.); a no-op on MQTT 3.1.1 connections.
- [`src/client.rs`](src/client.rs) — `MqttClient`: connect/publish/subscribe/
  unsubscribe/disconnect over `tokio::net::TcpStream`, packet-id bookkeeping
  for QoS 1/2 acks, a background read loop, and keep-alive pings.
- [`src/error.rs`](src/error.rs) — `MqttError` / `MqttResult`.

## Rust usage

```rust,no_run
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = ConnectOptions::new("localhost", 1883, "demo-client", MqttVersion::V5);
    opts.keep_alive_secs = 30;

    let client = MqttClient::new(opts);
    let result = client.connect().await?;
    println!("connected, session_present={}", result.session_present);

    client.subscribe("stem/demo".into(), QoS::AtLeastOnce).await?;
    client
        .publish("stem/demo".into(), b"hello".to_vec(), QoS::AtLeastOnce, false)
        .await?;

    client.disconnect().await?;
    Ok(())
}
```

Register an `MqttMessageListener` (before or after `connect()`) to receive
incoming PUBLISH messages and disconnect notifications without polling:

```rust,no_run
# use mqtt_client::{MqttMessage, MqttMessageListener};
struct Printer;
impl MqttMessageListener for Printer {
    fn on_message(&self, message: MqttMessage) {
        println!("{}: {:?}", message.topic, message.payload);
    }
    fn on_disconnected(&self, reason: String) {
        eprintln!("disconnected: {reason}");
    }
}
```

## Foreign-language bindings

```sh
cargo build --release -p mqtt-client
cargo run -p mqtt-client --features uniffi/cli --bin uniffi-bindgen -- \
    generate --library ../../target/release/libmqtt_client.so \
    --language kotlin --out-dir bindings/kotlin
```

Or use the repo-level helper, which builds the right `cdylib` for your
platform first: `../../scripts/generate-bindings.sh kotlin`.

## Examples

```sh
cargo run -p mqtt-client --example pub_sub               # connect, subscribe, publish, receive
cargo run -p mqtt-client --example will_and_retain        # Last-Will-and-Testament + retained messages
cargo run -p mqtt-client --example mqtt_versions          # MQTT 3.1.1 + 5.0 on the same broker
cargo run -p mqtt-client --example topics                 # topic hierarchies, `+`/`#` wildcards
cargo run -p mqtt-client --example long_lived_connection  # persistent connection, keep-alive over ~20s
```

All expect a broker listening on `127.0.0.1:1883` (run
`cargo run -p mqtt-broker --bin mqtt-broker` first, or
`cargo run -p mqtt-broker --example simple_broker`).

## Testing

```sh
cargo test -p mqtt-client
```

Protocol round-trip tests live alongside each codec module (`#[cfg(test)]`);
there are no separate integration tests for this crate — connection-level
behavior (including the QoS 1/2 handshakes and Last-Will delivery) is
covered by `mqtt-broker`'s integration tests, which drive a real client
against a real broker over a loopback socket.
