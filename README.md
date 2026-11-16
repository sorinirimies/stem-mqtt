# stem-mqtt

An **MQTT 3.1.1 / 5.0 client and broker written in Rust** — async (`tokio`), TLS, WebSocket,
QoS 0/1/2, shared subscriptions, enhanced authentication — usable as a Rust library, as a
standalone broker binary, from **Kotlin, Swift, Python, Ruby, Go, C#, Java, Dart, Node.js,
Haskell, C, C++, Zig and Lua** through generated [UniFFI](https://mozilla.github.io/uniffi-rs/) bindings, and from
**Elixir and Erlang** through a [Rustler](https://github.com/rusterlium/rustler) NIF. Client *and*
broker ship in every language, one generator per language, each runtime-tested against a real
broker in CI.

[![CI](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml)
[![Audit](https://github.com/sorinirimies/stem-mqtt/actions/workflows/security.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/security.yml)
[![Fuzz](https://github.com/sorinirimies/stem-mqtt/actions/workflows/fuzz.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/fuzz.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](Cargo.toml)

| Crate | Version | Downloads | Docs |
| --- | --- | --- | --- |
| `stem-mqtt-proto` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-proto.svg)](https://crates.io/crates/stem-mqtt-proto) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-proto.svg)](https://crates.io/crates/stem-mqtt-proto) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-proto)](https://docs.rs/stem-mqtt-proto) |
| `stem-mqtt-client-blocking` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-client-blocking.svg)](https://crates.io/crates/stem-mqtt-client-blocking) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-client-blocking.svg)](https://crates.io/crates/stem-mqtt-client-blocking) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-client-blocking)](https://docs.rs/stem-mqtt-client-blocking) |
| `stem-mqtt-client` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-client.svg)](https://crates.io/crates/stem-mqtt-client) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-client.svg)](https://crates.io/crates/stem-mqtt-client) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-client)](https://docs.rs/stem-mqtt-client) |
| `stem-mqtt-broker` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-broker.svg)](https://crates.io/crates/stem-mqtt-broker) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-broker.svg)](https://crates.io/crates/stem-mqtt-broker) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-broker)](https://docs.rs/stem-mqtt-broker) |

- [Quick start](#quick-start-rust) · [Installation](#installation) · [Features](#features) ·
  [Running the broker](#running-the-broker) · [Other languages](#generating-foreign-language-bindings)

## Quick start (Rust)

```sh
cargo add stem-mqtt-client stem-mqtt-broker anyhow
cargo add tokio --features full
```

An embedded broker and a client talking to each other:

```rust,no_run
use mqtt_broker::{MqttBroker, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Broker on a free local port.
    let broker = MqttBroker::new(MqttBrokerConfig::new("127.0.0.1", 0));
    broker.start().await?;
    let port = broker.bound_port().expect("bound");

    // Client (MQTT 5; use MqttVersion::V311 for 3.1.1).
    let client = MqttClient::new(ConnectOptions::new("127.0.0.1", port, "my-client-id", MqttVersion::V5));
    client.connect().await?;
    client.subscribe("stem/demo".into(), QoS::AtLeastOnce).await?;
    client.publish("stem/demo".into(), b"hello".to_vec(), QoS::AtLeastOnce, false).await?;

    client.disconnect().await?;
    broker.stop().await?;
    Ok(())
}
```

Just want a broker? `cargo install stem-mqtt-broker && mqtt-broker --port 1883`
(see [Running the broker](#running-the-broker)).

Need only the wire format (for firmware, a proxy, a test tool)? [`stem-mqtt-proto`](crates/mqtt-proto/README.md)
is the runtime-free codec the client and broker are built on: no async runtime, no TLS, no sockets.

Runnable examples (the `mqtt-client` ones need a broker already running):

```sh
cargo run -p stem-mqtt-broker --example simple_broker
cargo run -p stem-mqtt-client --example pub_sub
cargo run -p stem-mqtt-client --example will_and_retain
cargo run -p stem-mqtt-client --example mqtt_versions          # MQTT 3.1.1 + 5.0 on the same broker
cargo run -p stem-mqtt-client --example topics                 # topic hierarchies, `+` / `#` wildcards
cargo run -p stem-mqtt-client --example long_lived_connection  # persistent connection, keep-alive
cargo run -p stem-mqtt-client --example shared_subscriptions   # $share/<group>/… round-robin work queue
cargo run -p stem-mqtt-broker --example auth_broker            # then, in another terminal:
cargo run -p stem-mqtt-client --example auth_client            # one login rejected, one accepted
```

Full crate docs: [`crates/mqtt-proto`](crates/mqtt-proto/README.md),
[`crates/mqtt-client`](crates/mqtt-client/README.md),
[`crates/mqtt-broker`](crates/mqtt-broker/README.md).

## Preview

**Publish / subscribe**

![Publish / subscribe](examples/vhs/generated/pub-sub-demo.gif)

The `mqtt-broker` CLI, then the `pub_sub` example against it: a full publish/subscribe round trip.

**MQTT 3.1.1 and 5.0 side by side**

![MQTT 3.1.1 and 5.0 side by side](examples/vhs/generated/mqtt-versions-demo.gif)

Two clients on different protocol versions share one broker, each publishing and both receiving both messages — the version is negotiated per connection, not per broker.

**Topics and wildcards**

![Topics and wildcards](examples/vhs/generated/topics-demo.gif)

An exact topic, a `+` single-level and a `#` multi-level filter watch the same topic tree; four publishes show which filter catches which message.

**Shared subscriptions**

![Shared subscriptions](examples/vhs/generated/shared-subscriptions-demo.gif)

Three workers join `$share/workers/jobs/#` and split nine jobs round-robin, three each, while an ordinary subscriber on the same topics still sees all nine.

**Retained messages and Last Will**

![Retained messages and Last Will](examples/vhs/generated/will-and-retain-demo.gif)

A subscriber that arrives after a retained publish still receives it; when a client vanishes without DISCONNECT, the broker publishes its will.

**Authentication**

![Authentication](examples/vhs/generated/auth-demo.gif)

A pluggable auth provider rejects two wrong logins and accepts the right one; the broker's own log then shows every decision and connection.

**Long-lived connection**

![Long-lived connection](examples/vhs/generated/long-lived-connection-demo.gif)

One connection held open across several keep-alive intervals with a heartbeat every two seconds — the shape of a background service or IoT device.

**TLS**

![TLS](examples/vhs/generated/tls-demo.gif)

The blocking client over TLS (`stem-mqtt-client-blocking`, feature `rustls`) against a broker with a TLS listener and a throwaway self-signed certificate: without the broker's CA the certificate is refused, with it a subscriber receives what a publisher sends.

Recorded with [VHS](https://github.com/charmbracelet/vhs): `just vhs-tape <name>` or `just vhs-all`
(tapes in [`examples/vhs`](examples/vhs); each tape runs the example of the same name, except `tls-demo`, which runs the `tls_broker` and `mqtts` examples). A tape builds first, then records only the command that matters: short hidden shell functions (`demo`, `mqtts`) stand in for the long `cargo run` lines.

## Installation

Every published package is prefixed `stem-mqtt-` (`mqtt-client` / `mqtt-broker` are taken by unrelated
projects on crates.io and PyPI; `stem-mqtt-proto` is a Rust-only crate, with no foreign-language package); import names in code are unchanged. A package is available once a
release has published it to that registry.

| Language | Install | Import | Guide |
| --- | --- | --- | --- |
| **Rust** | `cargo add stem-mqtt-client stem-mqtt-broker` | `use mqtt_client::…; use mqtt_broker::…;` | [rust.md](docs/languages/rust.md) |
| Python | `pip install stem-mqtt-client stem-mqtt-broker` | `import mqtt_client, mqtt_broker` | [python.md](docs/languages/python.md) |
| Node.js | `npm i stem-mqtt` | `stem-mqtt/client` · `stem-mqtt/broker` | [node.md](docs/languages/node.md) |
| Kotlin (JVM/Android) | `io.github.sorinirimies.stemmqtt:stem-mqtt-kotlin` (Maven Central) | `import uniffi.mqtt_client.*` | [kotlin.md](docs/languages/kotlin.md) |
| Java (JDK 22+) | `io.github.sorinirimies.stemmqtt:stem-mqtt-java` (Maven Central) | `import uniffi.mqtt_client.*;` | [java.md](docs/languages/java.md) |
| C# | `dotnet add package StemMqtt` | `using uniffi.mqtt_client;` | [csharp.md](docs/languages/csharp.md) |
| Go | `go get github.com/sorinirimies/stem-mqtt/go` | `…/stem-mqtt/go/mqtt_client` | [go.md](docs/languages/go.md) |
| Dart | `dart pub add stem_mqtt` | `package:stem_mqtt/mqtt_client.dart` | [dart.md](docs/languages/dart.md) |
| Haskell | `cabal install stem-mqtt` | `import UniFFI.MqttClient` | [haskell.md](docs/languages/haskell.md) |
| Elixir / Erlang | `{:stem_mqtt, "~> 0.5"}` (Hex) | `StemMqtt.Client` · `stem_mqtt` | [elixir.md](docs/languages/elixir.md) |
| Ruby | `gem install stem-mqtt-client stem-mqtt-broker` | `require "stem_mqtt_client"` | [ruby.md](docs/languages/ruby.md) |
| Swift | `StemMqttSwift-<version>.zip` from the GitHub Release | `import MqttClient` | [swift.md](docs/languages/swift.md) |
| C | sources zip from the GitHub Release | see guide | [c.md](docs/languages/c.md) |
| C++ | sources zip from the GitHub Release | see guide | [cpp.md](docs/languages/cpp.md) |
| Zig | sources zip from the GitHub Release | see guide | [zig.md](docs/languages/zig.md) |
| Lua | sources zip from the GitHub Release | see guide | [lua.md](docs/languages/lua.md) |

Every language gets a client and a broker. Each guide covers requirements, a complete example, the API, authentication, TLS, building from source and troubleshooting: [`docs/languages/`](docs/languages/README.md).

Callbacks are available everywhere except Dart and Haskell, which use the
[pull-style API](docs/languages/README.md#receiving-messages-and-events).

## Features

**Protocol**

| | |
| --- | --- |
| **MQTT 3.1.1 and 5.0** | One codec, version negotiated per connection (`MqttVersion::V311` / `V5`). |
| **QoS 0, 1, 2** | Full exactly-once handshake (PUBLISH → PUBREC → PUBREL → PUBCOMP) with real DUP=1 redelivery on timeout, client and broker side. |
| **Retained, Will, keep-alive** | Retained messages, Last Will and Testament, keep-alive pings and timeout detection. |
| **Shared subscriptions** | `$share/<group>/<filter>`: one delivery per group, round-robin, via a topic-trie index (no per-publish scan of sessions). |
| **Session expiry** | `session_expiry_secs` for persistent sessions that never return. |

**Transport and security**

| | |
| --- | --- |
| **TLS** | Pure-Rust `rustls` on client and broker: custom CA, mutual TLS, no OpenSSL. The blocking client has it too ([details](crates/mqtt-client-blocking#securing-a-client)). |
| **WebSocket** | `--ws-port` serves MQTT-over-WebSocket next to raw TCP, so browsers connect directly (see [`demo/`](demo)). |
| **Authentication** | Pluggable username/password (`MqttAuthProvider`) and MQTT 5 enhanced auth: multi-round SCRAM, Kerberos, OAuth… (`MqttEnhancedAuthProvider` on the broker, `MqttAuthHandler` on the client). |

**Client and broker**

| | |
| --- | --- |
| **Auto-reconnect** | Opt in with `ConnectOptions::auto_reconnect`: exponential backoff, subscriptions replayed after reconnect. |
| **Event observation** | `MqttBrokerEventListener` for connections and messages. |
| **Pull-style API** | For runtimes without callbacks (Dart, Haskell): `enable_message_queue` / `next_message` on the client, `enable_event_queue` / `next_event` on the broker. |

**Packaging**

| | |
| --- | --- |
| **Runtime-free codec** | `stem-mqtt-proto` covers every control packet incl. MQTT 5 AUTH; no async runtime, no sockets, fine on a microcontroller. |
| **Blocking client** | `stem-mqtt-client-blocking` runs on any `std::io` transport, including ESP32 with ESP-IDF. |
| **16 languages** | Client and broker in each, via UniFFI and Rustler. See [Installation](#installation) and [bindings](#generating-foreign-language-bindings). |

**Quality**

- **Hardened by default**: packet-size limits, bounded per-client send queues, credential-redacting `Debug`, foreign callbacks that can't crash I/O tasks.
- **Fuzzed decoder**: randomised tests in every build plus `cargo-fuzz` targets in [`fuzz/`](fuzz).
- **Runtime-tested bindings**: every language runs against a real broker in CI. See `CHANGELOG.md`.

## Running the broker

```sh
cargo install stem-mqtt-broker                 # or: cargo run -p stem-mqtt-broker --bin mqtt-broker --
mqtt-broker --bind 0.0.0.0 --port 1883 --ws-port 8083 --allow-anonymous
```

| Option | Meaning |
| --- | --- |
| `--bind`, `--port` | TCP listener (default `0.0.0.0:1883`) |
| `--ws-port` | also accept MQTT-over-WebSocket (browser clients) |
| `--allow-anonymous` | accept clients without credentials |
| `--max-clients` | connection limit (0 = unlimited) |
| `--log-level` | `error` … `trace` (default `info`) |

Pre-built Linux and Windows `mqtt-broker` binaries are attached to every release.

### Demos

```sh
docker compose up --build        # broker (TCP + WS) + a browser client at http://localhost:8090
just demo-dashboard              # broker + the Rust/Topcoat web dashboard (multi-client, live feed) on :3000
```

[`demo/README.md`](demo/README.md) covers the plain-binaries path; [`packaging/k8s`](packaging/k8s/README.md)
has a one-Pod and a Deployment + Service Kubernetes demo.

## Workspace layout

| Path | Description |
| --- | --- |
| [`crates/mqtt-proto`](crates/mqtt-proto) | The MQTT 3.1.1 / 5.0 wire codec (`stem-mqtt-proto`): pure and synchronous, no async runtime, TLS or UniFFI. Shared by the client and the broker. |
| [`crates/mqtt-client-blocking`](crates/mqtt-client-blocking) | The **blocking** client over any `std::io` transport, built on the codec: no async runtime, runs on an ESP32 with ESP-IDF `std`; optional `rustls` TLS for PCs. The small counterpart of `mqtt-client`. |
| [`crates/mqtt-client`](crates/mqtt-client) | Async, `tokio`-based MQTT client; re-exports the codec as `mqtt_client::protocol`. |
| [`crates/mqtt-broker`](crates/mqtt-broker) | Standalone broker (library + `mqtt-broker` CLI), built on the same codec. |
| [`demo/`](demo) | Browser client (`web/`) and a Rust + Topcoat dashboard (`dashboard/`). |
| [`tests/bindings`](tests/bindings) | One end-to-end smoke test per language, run by `scripts/test_bindings.nu`. |
| [`packaging/`](packaging) | Per-language package templates, Dockerfile and Kubernetes manifests. |
| [`scripts/`](scripts) | Nushell scripts for bindings, packaging, publishing and releases (tested in `scripts/tests`). |
| [`fuzz/`](fuzz) | `cargo-fuzz` targets (on `stem-mqtt-proto`): packet decoder, encode/decode round-trip, chunked reads, varint/strings, PUBLISH, topic matching (`just fuzz <target>`). |

`stem-mqtt-proto` is the shared foundation: a pure, synchronous codec with no networking or async
dependencies. The client re-exports it as `mqtt_client::protocol` / `mqtt_client::error` (the paths are
unchanged) and declares `QoS`, `MqttVersion` and `MqttError` to UniFFI as remote types, so the generated
bindings keep their shape. Publish order: `stem-mqtt-proto`, then `stem-mqtt-client`, then `stem-mqtt-broker`.

## Generating foreign-language bindings

Everything is driven by [Nushell](https://www.nushell.sh/) scripts; the generator
table (languages, pinned generator versions, required UniFFI release, registry)
lives in one file, [`scripts/bindings/spec.nu`](scripts/bindings/spec.nu).

```sh
nu scripts/generate_bindings.nu kotlin            # kotlin | swift | python (built into UniFFI)
nu scripts/install_bindgens.nu                    # install the pinned third-party generators
nu scripts/generate_bindings.nu go mqtt-broker    # go | csharp | java | dart | node | haskell | c | cpp | zig | lua
just bindings-all                                 # or: just bindings-third-party
```

| Language | Generator | Runtime-tested | Status |
| --- | --- | :-: | --- |
| Kotlin | UniFFI (built in) | Gradle packaging test in CI | stable |
| Swift | UniFFI (built in) | ✅ client + broker + all callbacks | stable |
| Python | UniFFI (built in) | ✅ client + broker + all callbacks | stable |
| Go | [uniffi-bindgen-go](https://github.com/NordSecurity/uniffi-bindgen-go) | ✅ client + broker + all callbacks | stable |
| C# | [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) | ✅ client + broker + all callbacks | stable |
| Java (JDK 22+) | [uniffi-bindgen-java](https://github.com/IronCoreLabs/uniffi-bindgen-java) | ✅ client + broker + all callbacks | stable |
| Dart | [uniffi-dart](https://github.com/acterglobal/uniffi-dart) | ✅ client + broker through the pull-style API ¹ | experimental |
| Node.js | [uniffi-bindgen-node-js](https://github.com/criccomini/uniffi-bindgen-node-js) | ✅ client + broker + all callbacks | experimental |
| Haskell | [uniffi-bindgen-haskell](https://github.com/mercury/uniffi-bindgen-haskell) + [our patch](packaging/haskell) (PR [#3](https://github.com/mercury/uniffi-bindgen-haskell/pull/3)) ² | ✅ client + broker through the pull-style API ¹ | experimental |
| C | [uniffi-bindgen-c](https://github.com/sorinirimies/uniffi-bindgen-c) | ✅ client + broker + all callbacks | experimental |
| C++ | [uniffi-bindgen-cpp](https://github.com/sorinirimies/uniffi-bindgen-cpp) | ✅ client + broker + all callbacks | experimental |
| Zig | [uniffi-bindgen-zig](https://github.com/sorinirimies/uniffi-bindgen-zig) | ✅ client + broker + all callbacks | experimental |
| Lua | [uniffi-bindgen-lua](https://github.com/sorinirimies/uniffi-bindgen-lua) ³ | ✅ client + broker + all callbacks | experimental |

¹ Dart and Haskell can't implement foreign callbacks; they use the [pull-style API](docs/languages/README.md#receiving-messages-and-events).
² Haskell uses our patched generator, proposed upstream. ³ Lua needs uniffi-bindgen-lua 0.1.2 or later.
Details: [docs/binding-generators.md](docs/binding-generators.md).

C, C++, Zig and Lua ship as sources (`bindings-<language>.zip` on each release) to compile into your program and link against the `mqtt-client-` / `mqtt-broker-<version>-<target>` archives.

Each language has one generator, and each component is self-contained (the broker owns its own `QoS` enum), so every language gets a standalone client and broker. Third-party generators pin different UniFFI releases, so `generate_bindings.nu` builds each in a scratch workspace under `target/uniffi-<version>/` and never touches yours.

### Do the bindings actually work?

```sh
nu scripts/test_bindings.nu            # every language (missing toolchains are skipped)
nu scripts/test_bindings.nu go java --strict
just test-bindings go
```

For each language this builds the native libraries, generates the bindings, stages a
scratch project with the smoke test from [`tests/bindings/<language>/`](tests/bindings) and runs
it against real sockets: start a broker, subscribe, publish QoS 1, receive it through a *foreign*
callback, refuse a bad client through a *foreign* auth callback, observe connections through a
*foreign* event listener, stop. Languages that can't do callbacks run a reduced scenario (see the
header of each test). Verdicts: `PASS`, `FAIL`, `SKIP` (toolchain missing). CI runs the same command for every
language on every push, and each package is built and installed the way a user would before it is released.
Maintainer docs: [development](docs/development.md) · [publishing](docs/publishing.md).

## License

MIT — see [LICENSE](LICENSE).
