# stem-mqtt

Full MQTT 3.1.1 / MQTT 5.0 client and broker, written in Rust and exposed
to Kotlin, Swift, Python, and Ruby via [UniFFI](https://mozilla.github.io/uniffi-rs/),
plus Node.js/TypeScript via a hand-written [napi-rs](https://napi.rs/) addon
(JavaScript isn't a UniFFI target).

[![CI](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Preview

![Pub/Sub Demo](examples/vhs/generated/pub-sub-demo.gif)

Starting the `mqtt-broker` CLI, then running the `pub_sub` client example
against it for a full publish/subscribe round trip.

![MQTT Versions Demo](examples/vhs/generated/mqtt-versions-demo.gif)

MQTT 3.1.1 and MQTT 5.0 clients connected to the same broker at the same
time, each publishing and both receiving both messages — the protocol
version is negotiated per-connection, not per-broker.

![Topics & Wildcards Demo](examples/vhs/generated/topics-demo.gif)

An exact topic, a `+` single-level wildcard, and a `#` multi-level
wildcard all watching the same topic tree, then four publishes of varying
depth — see exactly which filter catches which message and why.

![Long-Lived Connection Demo](examples/vhs/generated/long-lived-connection-demo.gif)

A single connection held open for ~20 seconds across multiple keep-alive
intervals, publishing a heartbeat every 2 seconds — the shape a
long-running background service or IoT device holds a connection in,
rather than connect-publish-disconnect.

All four recorded with [VHS](https://github.com/charmbracelet/vhs) —
regenerate one with `just vhs-tape <name>` (tape names match the `.gif`
filenames above, minus the extension), or render every tape under
[`examples/vhs`](examples/vhs) with `just vhs-all`.

## Installation

Every published package name is prefixed `stem-mqtt-`/`stem_mqtt` —
`mqtt-client`/`mqtt-broker` are already taken by unrelated projects on
crates.io and PyPI, so this project uses the `stem-mqtt-` prefix
consistently everywhere to avoid that collision. Language-facing import/use
names (Rust `mqtt_client`/`mqtt_broker`, Python `mqtt_client`/`mqtt_broker`,
Swift `MqttClient`) are unaffected — only the *published package* identity
is prefixed.

### Rust (crates.io)

```sh
cargo add stem-mqtt-client   # client
cargo add stem-mqtt-broker   # broker (depends on stem-mqtt-client)
```

```rust,no_run
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS}; // import path unaffected by the package rename
```

### Node.js / TypeScript (npm)

```sh
npm install stem-mqtt-client
```

```ts
import { MqttClient } from "stem-mqtt-client";

const client = new MqttClient({ host: "localhost", port: 1883, clientId: "demo", version: "5.0" });
await client.connect();
await client.subscribe("demo/topic", 1);
await client.publish("demo/topic", Buffer.from("hi"), 1, false);
```

Server-side Node only (no browser build — see [`packaging/node/README.md`](packaging/node/README.md)).

### Kotlin (JVM or Android, via GitHub Packages)

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        maven {
            url = uri("https://maven.pkg.github.com/sorinirimies/stem-mqtt")
            credentials {
                username = providers.gradleProperty("gpr.user").getOrElse(System.getenv("GITHUB_ACTOR") ?: "")
                password = providers.gradleProperty("gpr.token").getOrElse(System.getenv("GITHUB_TOKEN") ?: "")
            }
        }
    }
}
```

```kotlin
// build.gradle.kts
dependencies {
    // JVM/desktop:
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-client-kotlin:0.2.1")
    // Android (real AAR with jniLibs for arm64-v8a/armeabi-v7a/x86_64/x86), instead:
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-client-android:0.2.1")
}
```

Reading a GitHub Package still requires an authenticated `GITHUB_TOKEN`/PAT
with `read:packages`, even for a public repo — a GitHub Packages platform
limitation. `mqtt-client` only (not `mqtt-broker` — an
[upstream uniffi-rs bug](https://github.com/mozilla/uniffi-rs/issues/2392)
blocks the broker's Kotlin bindings specifically; see
[`packaging/kotlin/README.md`](packaging/kotlin/README.md)). Full details:
[`packaging/kotlin/README.md`](packaging/kotlin/README.md).

### Swift (Swift Package Manager)

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/sorinirimies/stem-mqtt", from: "0.2.1"),
]
```

```swift
import MqttClient

let client = MqttClient(options: ConnectOptions(host: "localhost", port: 1883, clientId: "demo", version: .v5))
try await client.connect()
```

Pre-built XCFramework (macOS + iOS device + iOS simulator) attached as a
GitHub Release asset — no local Rust toolchain needed to consume it. Client
only, same reasoning as Node. Full details:
[`packaging/swift/README.md`](packaging/swift/README.md).

### Python (PyPI)

```sh
pip install stem-mqtt-client   # client
pip install stem-mqtt-broker   # broker
```

```python
import mqtt_client  # import name unaffected by the package rename

client = mqtt_client.MqttClient(mqtt_client.ConnectOptions("localhost", 1883, "demo", mqtt_client.MqttVersion.V5))
```

Not yet published (requires the `PYPI_API_TOKEN` repository secret to be
configured — the release workflow's `publish-python` job skips gracefully
until then). Full details: [`packaging/python/README.md`](packaging/python/README.md).

### Ruby (RubyGems)

```sh
gem install stem_mqtt   # one gem, both client and broker bindings
```

Not yet published (requires the `RUBYGEMS_API_KEY` repository secret to be
configured — the release workflow's `publish-ruby` job skips gracefully
until then). Full details: [`packaging/ruby/README.md`](packaging/ruby/README.md).

## Workspace layout

| Crate | Description |
| --- | --- |
| [`crates/mqtt-client`](crates/mqtt-client) | Async, `tokio`-based MQTT client. Also hosts the wire-protocol codec (`mqtt_client::protocol`) shared by the broker. |
| [`crates/mqtt-broker`](crates/mqtt-broker) | Standalone MQTT broker (library + `mqtt-broker` CLI binary), reusing the client crate's codec instead of duplicating it. |
| [`crates/mqtt-client-node`](crates/mqtt-client-node) | Node.js/TypeScript bindings for `mqtt-client` via napi-rs (server-side Node only — see [`packaging/node`](packaging/node)). |

There is intentionally no separate "core" crate — `mqtt-client::protocol` is
a pure, allocation-friendly, synchronous codec with no networking or async
dependencies, so it doubles as the shared foundation for both the client and
the broker.

## Features

- **MQTT 3.1.1 and MQTT 5.0** — one codec, version negotiated per connection
  (`MqttVersion::V311` / `MqttVersion::V5`).
- **QoS 0, 1, 2** — at-most-once, at-least-once, and the full exactly-once
  four-part handshake (PUBLISH → PUBREC → PUBREL → PUBCOMP), with real
  redelivery (DUP=1 resend on timeout) on both the client and broker side
  — not just a single fire-and-hope attempt.
- **All standard packet types** — CONNECT/CONNACK, PUBLISH and its ack
  chain, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PING, DISCONNECT, and
  MQTT 5 AUTH.
- **TLS** (client + broker, `mqtt-client::TlsOptions` /
  `mqtt-broker::BrokerTlsConfig`) — pure-Rust `rustls`, custom CA support,
  and mutual TLS (mTLS) client-certificate verification. No OpenSSL/system-
  TLS dependency.
- **Opt-in client auto-reconnect** (`ConnectOptions::auto_reconnect`) with
  exponential backoff, replaying every topic the client was subscribed to
  once reconnected.
- **Last Will and Testament**, retained messages, keep-alive pings.
- **MQTT-over-WebSocket** (`--ws-port`) alongside raw TCP, so browser
  clients (which can't open raw TCP sockets) can connect directly —
  see [`demo/`](demo) for a full browser pub/sub demo webpage.
- **Pluggable broker auth** (`MqttAuthProvider`) and event observation
  (`MqttBrokerEventListener`) for foreign callers.
- **UniFFI bindings** — both crates build as `cdylib`/`staticlib` and ship a
  `uniffi-bindgen` binary to generate Kotlin, Swift, or Python bindings
  (Ruby via the separate `uniffi-bindgen-ruby` generator).
- **Node.js / TypeScript bindings** for `mqtt-client` via napi-rs
  (`crates/mqtt-client-node`) — server-side Node, not the browser (no
  MQTT-over-WebSocket transport yet).

## Quick start (Rust)

```rust,no_run
use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = MqttClient::new(ConnectOptions::new(
        "broker.example.com", 1883, "my-client-id", MqttVersion::V5,
    ));
    client.connect().await?;
    client.subscribe("stem/demo".into(), QoS::AtLeastOnce).await?;
    client.publish("stem/demo".into(), b"hello".to_vec(), QoS::AtLeastOnce, false).await?;
    client.disconnect().await?;
    Ok(())
}
```

See [`crates/mqtt-client/README.md`](crates/mqtt-client/README.md) and
[`crates/mqtt-broker/README.md`](crates/mqtt-broker/README.md) for full
crate-level docs.

## Examples

```sh
cargo run -p stem-mqtt-client --example pub_sub
cargo run -p stem-mqtt-client --example will_and_retain
cargo run -p stem-mqtt-client --example mqtt_versions          # MQTT 3.1.1 + 5.0 on the same broker
cargo run -p stem-mqtt-client --example topics                 # topic hierarchies, `+`/`#` wildcards
cargo run -p stem-mqtt-client --example long_lived_connection  # persistent connection, keep-alive over ~20s
cargo run -p stem-mqtt-broker --example simple_broker
cargo run -p stem-mqtt-broker --example auth_broker
```

(All `mqtt-client` examples need a broker already running — either
`simple_broker`/`auth_broker` above or the `mqtt-broker` CLI below.)

## Running the broker

```sh
cargo run -p stem-mqtt-broker --bin mqtt-broker -- --bind 0.0.0.0 --port 1883
```

Add `--ws-port 8083` to also accept MQTT-over-WebSocket connections (for
browser clients — see [`demo/`](demo)).

## Demo: browser client + Docker + Kubernetes

```sh
docker compose up --build      # broker (TCP+WS) + a Topcoat/mqtt.js demo webpage
open http://localhost:8090
```

See [`demo/README.md`](demo/README.md) for the plain-binaries path, and
[`packaging/k8s/README.md`](packaging/k8s/README.md) for a one-Pod or
Deployment+Service Kubernetes demo.

## Generating foreign-language bindings

```sh
./scripts/generate-bindings.sh kotlin   # or: swift, python
just bindings-ruby                        # ruby uses a separate generator
```

Node.js/TypeScript bindings are a separate crate, not a UniFFI target:

```sh
cd crates/mqtt-client-node && npm ci && npm run build:debug
```

UniFFI's own CLI (bundled with the `uniffi` crate) natively supports Kotlin,
Swift, and Python. Ruby bindings come from the separate
[`uniffi-bindgen-ruby`](https://github.com/mozilla/uniffi-rs) generator, so
they're wired up through `packaging/ruby/build_and_publish.sh` instead of
`scripts/generate-bindings.sh` (which will tell you as much if you pass it
`ruby`).

See [`packaging/README.md`](packaging/README.md) for details on what the
script produces and how release artifacts are laid out, and
[`packaging/python`](packaging/python), [`packaging/kotlin`](packaging/kotlin),
[`packaging/swift`](packaging/swift), [`packaging/ruby`](packaging/ruby) for
how each language's package actually gets published (PyPI, GitHub Packages
Maven, an SPM binary target, and RubyGems respectively).

## Development

Every task below is also a [`just`](https://github.com/casey/just) recipe —
run `just --list` for the full set (bindings, packaging, changelog, version
bumps, releases, multi-remote git push/pull, ...):

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Or run everything CI runs in one shot:

```sh
just check-all      # fmt + clippy + test + doc + nu script tests
./scripts/check.sh  # equivalent, no `just`/`nu` required
```

### Nushell scripts

Deterministic release/CI plumbing (tag validation, version bump, changelog,
crates.io publish, quality gate) lives in `scripts/*.nu` as tested Nushell
scripts rather than ad-hoc shell in CI YAML — see
[`scripts/tests/`](scripts/tests) (`nu scripts/tests/run_all.nu`, also
reachable as `just test-nu`).

## CI/CD

- **GitHub** (`.github/workflows/`): `ci.yml` (fmt/clippy/test/build/doc/nu/node
  tests), `release.yml` (on `vX.Y.Z` tag: cross-platform `mqtt-broker` +
  `mqtt-client` native library artifacts, all-language bindings including a
  multi-platform Node addon, GitHub Release, then crates.io + PyPI +
  GitHub Packages (Kotlin) + Swift Package Manager + RubyGems + npm
  publishing — each publish job skips gracefully if its registry secret
  isn't configured), `auto-merge.yml` (Dependabot), `dependabot.yml`
  (GitHub Actions version bumps).
- **Gitea** (`.gitea/workflows/`): mirrored `ci.yml`/`release.yml` (curl-installed
  Rust instead of marketplace actions, for portability across Gitea runner
  setups) plus `deps-update.yml` (nightly `cargo upgrade` sweep, auto-committed
  if the quality gate stays green).

## Release process

```sh
just release 0.2.0   # bump + quality gate + commit + tag + push --follow-tags
```

See the [`justfile`](justfile) for the full release/version-bump/multi-remote
(GitHub + Gitea) workflow — `just bump`, `just release-all`, `just sync-gitea`,
`just migrate-gitea`, etc.

## License

MIT — see [LICENSE](LICENSE).
