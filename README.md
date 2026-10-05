# stem-mqtt

An **MQTT 3.1.1 / 5.0 client and broker written in Rust** — async (`tokio`), TLS, WebSocket,
QoS 0/1/2, shared subscriptions, enhanced authentication — usable as a Rust library, as a
standalone broker binary, and from **Kotlin, Swift, Python, Go, C#, Java, Dart, Node.js and
Haskell** through generated [UniFFI](https://mozilla.github.io/uniffi-rs/) bindings. Client *and*
broker ship in every language, one generator per language, each runtime-tested against a real
broker in CI.

[![CI](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](Cargo.toml)

| Crate | Version | Downloads | Docs |
| --- | --- | --- | --- |
| `stem-mqtt-client` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-client.svg)](https://crates.io/crates/stem-mqtt-client) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-client.svg)](https://crates.io/crates/stem-mqtt-client) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-client)](https://docs.rs/stem-mqtt-client) |
| `stem-mqtt-broker` | [![crates.io](https://img.shields.io/crates/v/stem-mqtt-broker.svg)](https://crates.io/crates/stem-mqtt-broker) | [![downloads](https://img.shields.io/crates/d/stem-mqtt-broker.svg)](https://crates.io/crates/stem-mqtt-broker) | [![docs.rs](https://img.shields.io/docsrs/stem-mqtt-broker)](https://docs.rs/stem-mqtt-broker) |

- [Quick start](#quick-start-rust) · [Installation](#installation) · [Features](#features) ·
  [Running the broker](#running-the-broker) · [Other languages](#generating-foreign-language-bindings) ·
  [Publishing](#publishing) · [Development](#development)

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

Full crate docs: [`crates/mqtt-client`](crates/mqtt-client/README.md),
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

A pluggable auth provider rejects one login and accepts another, and the broker's event listener logs each connection.

**Long-lived connection**

![Long-lived connection](examples/vhs/generated/long-lived-connection-demo.gif)

One connection held open across several keep-alive intervals with a heartbeat every two seconds — the shape of a background service or IoT device.

Recorded with [VHS](https://github.com/charmbracelet/vhs): `just vhs-tape <name>` or `just vhs-all`
(tapes in [`examples/vhs`](examples/vhs); each tape runs the example of the same name).

## Installation

Every published package is prefixed `stem-mqtt-` (`mqtt-client` / `mqtt-broker` are taken by unrelated
projects on crates.io and PyPI); import names in code are unchanged. A package is available once a
release has published it to that registry — see [Publishing](#publishing) for what goes where.

| Language | Install | Import |
| --- | --- | --- |
| **Rust** | `cargo add stem-mqtt-client stem-mqtt-broker` | `use mqtt_client::…; use mqtt_broker::…;` |
| Python | `pip install stem-mqtt-client stem-mqtt-broker` | `import mqtt_client, mqtt_broker` |
| Node.js | `npm i @sorinirimies/stem-mqtt-node` | `@sorinirimies/stem-mqtt-node/client` · `/broker` |
| Kotlin (JVM/Android) | `io.github.sorinirimies.stemmqtt:stem-mqtt-kotlin` (Maven Central) | `import uniffi.mqtt_client.*` |
| Java (JDK 22+) | `io.github.sorinirimies.stemmqtt:stem-mqtt-java` (Maven Central) | `import uniffi.mqtt_client.*;` |
| C# | `dotnet add package StemMqtt` | `using uniffi.mqtt_client;` |
| Go | `go get github.com/sorinirimies/stem-mqtt-go` | `…/stem-mqtt-go/mqtt_client` |
| Dart | `dart pub add stem_mqtt` | `package:stem_mqtt/mqtt_client.dart` |
| Haskell | `cabal install stem-mqtt` | `import UniFFI.MqttClient` |
| Swift | `StemMqttSwift-<version>.zip` from the GitHub Release | `import MqttClient` |

Everything except the Rust crates ships the same two components — a client and a broker — so each
language section below shows only what's specific to it.

### Rust

Covered in [Quick start](#quick-start-rust). The client crate also hosts the wire-protocol codec
(`mqtt_client::protocol`) that the broker reuses. MSRV: Rust 1.75.

### Python

```python
import mqtt_client

client = mqtt_client.MqttClient(mqtt_client.ConnectOptions(
    host="localhost", port=1883, client_id="demo", version=mqtt_client.MqttVersion.V5,
    clean_start=True, keep_alive_secs=30, username=None, password=None, will=None,
    connect_timeout_secs=10, operation_timeout_secs=15, auto_reconnect=True,
    reconnect_backoff_secs=1, reconnect_max_backoff_secs=30, tls=None,
))
```

Complete scenario: [`tests/bindings/python`](tests/bindings/python). Details: [`packaging/python`](packaging/python/README.md).

### Node.js / TypeScript

```ts
import * as mqtt from "@sorinirimies/stem-mqtt-node/client";
import * as brk from "@sorinirimies/stem-mqtt-node/broker";

const broker = new brk.MqttBroker({ bind_address: "127.0.0.1", port: 1883, /* … */ });
await broker.start();

const client = new mqtt.MqttClient({ host: "127.0.0.1", port: 1883, client_id: "demo", version: mqtt.MqttVersion.V5, /* … */ });
await client.connect();
await client.subscribe("demo/topic", mqtt.QoS.AtLeastOnce);
```

Server-side Node only (no browser build). Generated by
[`uniffi-bindgen-node-js`](https://github.com/criccomini/uniffi-bindgen-node-js); a complete example is
[`tests/bindings/node/smoke.mjs`](tests/bindings/node/smoke.mjs). (This replaces the earlier hand-written
napi-rs addon `stem-mqtt-client`, which had no broker.)

### Kotlin (JVM and Android)

```kotlin
// build.gradle.kts — JVM/desktop (client + broker):
implementation("io.github.sorinirimies.stemmqtt:stem-mqtt-kotlin:<version>")
// Android AAR (client + broker, all four ABIs), from GitHub Packages:
implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-android:<version>")
```

GitHub Packages (and your Gitea Maven registry) use the group `com.github.sorinirimies.stemmqtt` and need an
authenticated token even for public packages; Maven Central needs none. Details:
[`packaging/kotlin/README.md`](packaging/kotlin/README.md).

### Java

Needs **JDK 22+** (Foreign Function & Memory API); run with `--enable-native-access=ALL-UNNAMED`.
`implementation("io.github.sorinirimies.stemmqtt:stem-mqtt-java:<version>")`; example:
[`tests/bindings/java/Smoke.java`](tests/bindings/java/Smoke.java).

### C#

`dotnet add package StemMqtt`; `uniffi.mqtt_client` and `uniffi.mqtt_broker` define their own `QoS`, so
alias them (`using ClientQoS = uniffi.mqtt_client.QoS;`). Example:
[`tests/bindings/csharp/Program.cs`](tests/bindings/csharp/Program.cs).

### Go

cgo module; the native libraries `libmqtt_client` / `libmqtt_broker` are installed separately
(release assets or `cargo build --release`) and found through `CGO_LDFLAGS`. Details:
[`packaging/go/README.md`](packaging/go/README.md); example: [`tests/bindings/go/main.go`](tests/bindings/go/main.go).

### Dart

`dart pub add stem_mqtt`. A native-assets build hook compiles the bundled Rust with `cargo`, so a Rust
toolchain is required. Dart can't receive Rust callbacks — use the [pull-style API](#generating-foreign-language-bindings).
Details: [`packaging/dart/pub/README.md`](packaging/dart/pub/README.md).

### Haskell

`build-depends: stem-mqtt`. A source package: it builds its bundled Rust with `cargo` at install time (Rust
toolchain required). Callbacks aren't available — use the pull-style API.
Details: [`packaging/haskell/hackage/README.md`](packaging/haskell/hackage/README.md).

### Swift

Download `StemMqttSwift-<version>.zip` from the matching GitHub Release, extract it, and add the
`StemMqttSwift` directory as a local Swift package (client and broker products, XCFrameworks for macOS,
iOS and the iOS simulator). Details: [`packaging/swift/README.md`](packaging/swift/README.md).

> **Ruby** isn't supported: UniFFI has no official Ruby backend and no production-ready generator exists
> ([`packaging/ruby`](packaging/ruby/README.md)).

## Features

- **MQTT 3.1.1 and MQTT 5.0** — one codec, version negotiated per connection
  (`MqttVersion::V311` / `MqttVersion::V5`).
- **QoS 0, 1, 2** — at-most-once, at-least-once, and the full exactly-once
  four-part handshake (PUBLISH → PUBREC → PUBREL → PUBCOMP), with real
  redelivery (DUP=1 resend on timeout) on both the client and broker side
  — not just a single fire-and-hope attempt.
- **Wire codec for all control packet types** — CONNECT/CONNACK, PUBLISH and
  its ack chain, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PING, DISCONNECT,
  and MQTT 5 AUTH — with the enhanced-authentication exchange built on top
  (see *Authentication* below).
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
- **Authentication** — pluggable username/password (`MqttAuthProvider`) and MQTT 5
  **enhanced authentication** (multi-round challenge/response: SCRAM, Kerberos, OAuth, …) via
  `MqttEnhancedAuthProvider` on the broker and `MqttAuthHandler` on the client.
- **Shared subscriptions** (`$share/<group>/<filter>`) — one delivery per group, round-robin —
  routed through a **topic-trie subscription index** instead of a per-publish scan of every session.
- **Session expiry** (`session_expiry_secs`) for persistent sessions that never return.
- **Event observation** (`MqttBrokerEventListener`) and, for runtimes that can't receive callbacks
  (Dart, Haskell), a **pull-style API**: `enable_message_queue` / `next_message` on the client,
  `enable_event_queue` / `next_event` on the broker.
- **UniFFI bindings** — both crates build as `cdylib`/`staticlib` and ship a
  `uniffi-bindgen` binary for Kotlin, Swift and Python; Go, C#, Java, Dart,
  Node.js and Haskell are generated through pinned community generators — all of
  them runtime-tested in CI against a real broker.
- **Hardened by default** — packet-size limits, bounded per-client send queues,
  keep-alive timeout detection, credential-redacting `Debug`, foreign callbacks
  that can't crash I/O tasks, and a decoder that is fuzzed (randomised tests in
  every build + `cargo-fuzz` targets in [`fuzz/`](fuzz)) — see `CHANGELOG.md`.

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
| [`crates/mqtt-client`](crates/mqtt-client) | Async, `tokio`-based MQTT client; also hosts the wire-protocol codec (`mqtt_client::protocol`) shared with the broker. |
| [`crates/mqtt-broker`](crates/mqtt-broker) | Standalone broker (library + `mqtt-broker` CLI), reusing the client's codec. |
| [`demo/`](demo) | Browser client (`web/`) and a Rust + Topcoat dashboard (`dashboard/`). |
| [`tests/bindings`](tests/bindings) | One end-to-end smoke test per language, run by `scripts/test_bindings.nu`. |
| [`packaging/`](packaging) | Per-language package templates, Dockerfile and Kubernetes manifests. |
| [`scripts/`](scripts) | Nushell scripts for bindings, packaging, publishing and releases (tested in `scripts/tests`). |
| [`fuzz/`](fuzz) | `cargo-fuzz` targets for the packet decoder and topic matching. |

There is deliberately no separate "core" crate: `mqtt-client::protocol` is a pure, synchronous codec with
no networking or async dependencies, so it is the shared foundation for both client and broker.

## Generating foreign-language bindings

Everything is driven by [Nushell](https://www.nushell.sh/) scripts; the generator
table (languages, pinned generator versions, required UniFFI release, registry)
lives in one file, [`scripts/bindings/spec.nu`](scripts/bindings/spec.nu).

```sh
nu scripts/generate_bindings.nu kotlin            # kotlin | swift | python (built into UniFFI)
nu scripts/install_bindgens.nu                    # install the pinned third-party generators
nu scripts/generate_bindings.nu go mqtt-broker    # go | csharp | java | dart | node | haskell
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
| Haskell | [uniffi-bindgen-haskell](https://github.com/mercury/uniffi-bindgen-haskell) + [our patch](packaging/haskell) (PR [#3](https://github.com/mercury/uniffi-bindgen-haskell/pull/3)) | ✅ client + broker through the pull-style API ¹ | experimental |

¹ Dart: foreign callbacks cannot be invoked from Rust's own threads (the VM aborts). Haskell: callback
interfaces are exposed only as opaque handles. So `MqttMessageListener`, `MqttAuthProvider`,
`MqttBrokerEventListener` and the enhanced-auth callbacks can't be implemented there — use the
**pull-style API** that exists for exactly this: `enable_message_queue` / `next_message(timeout_ms)` on the
client and `enable_event_queue` / `next_event(timeout_ms)` on the broker. Their smoke tests receive
messages and observe broker events that way. Authentication uses the built-in `allow_anonymous` rule.
² Upstream's Haskell generator had three bugs that made every binding unusable: constructors never
lowered their arguments (generated code didn't compile), flat-error variants dropped their message
(decoder failed with "left N trailing bytes"), and records sharing a field name broke the public module.
[`packaging/haskell/uniffi-bindgen-haskell.patch`](packaging/haskell/uniffi-bindgen-haskell.patch)
fixes all three (and is proposed upstream, see [`packaging/haskell`](packaging/haskell)); `install_bindgens.nu` builds the generator from the pinned revision with the patch applied.

**One solution per language, client and broker everywhere.** Each language has exactly one generator
(no parallel hand-written addons), and each component is self-contained: the broker owns its own `QoS`
enum instead of importing the client's, so no generator ever meets a cross-crate "external type" (the
Node generator rejects them) and every language gets a standalone broker package.

UniFFI bindgens can only read metadata from the UniFFI release they were built
for, and the third-party generators disagree (most target 0.31, Haskell 0.32). `generate_bindings.nu` therefore builds the library for
such a generator in a scratch workspace under `target/uniffi-<version>/` pinned
to the right release; your workspace is never touched.

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
header of each test). Verdicts: `PASS`, `FAIL`, `SKIP` (toolchain missing). CI (Gitea `.gitea/workflows/ci.yml`, Linux runner; mirrored in
`.github/workflows/ci.yml`) runs the same command per language (`test-bindings-runtime`), builds each
package (`test-packaging`: `dotnet pack`, `gradle build`, `npm pack`, OCI bundle) and runs the Kotlin
Gradle test; the release workflow runs the runtime test before packaging each language.

## Publishing

```sh
nu scripts/publish_packages.nu stage <language>                       # bindings + native libs -> dist/<language>
nu scripts/publish_packages.nu publish <language> <version> --dry-run  # print the plan; --target github|gitea|public
```

### Public registries

The Gitea release workflow publishes to the indexes people install from (`--target public`). Each job needs
its own repository secret and **skips quietly without it**:

| Language | Registry | Package | Secret(s) | Notes |
| --- | --- | --- | --- | --- |
| Rust | crates.io | `stem-mqtt-client`, `stem-mqtt-broker` | `CRATES_IO_TOKEN` | |
| Python | PyPI | `stem-mqtt-client`, `stem-mqtt-broker` | `PYPI_API_TOKEN` | manylinux_2_28 wheels (zig) |
| Node.js | npmjs.org | `@<owner>/stem-mqtt-node` | `NPM_TOKEN` | the npm scope must exist |
| C# | NuGet.org | `StemMqtt` | `NUGET_API_KEY` | |
| Kotlin / Java | Maven Central | `io.github.<owner>.stemmqtt:stem-mqtt-kotlin` / `…-java` | `MAVEN_CENTRAL_USERNAME`, `MAVEN_CENTRAL_PASSWORD`, `SIGNING_KEY`, `SIGNING_PASSWORD` | verify the `io.github.<owner>` namespace first |
| Haskell | Hackage | `stem-mqtt` | `HACKAGE_TOKEN` | source package, builds the bundled Rust with `cargo` |
| Dart | pub.dev | `stem_mqtt` | — (OIDC) | from GitHub Actions (`publish-pubdev.yml`); first release by hand |
| Go | Go modules | `github.com/<owner>/stem-mqtt-go` | `GO_MODULE_TOKEN` | pushes the module to its own repo and tags it |
| Swift | — | release asset (XCFrameworks) | — | needs macOS |

The Haskell and Dart packages are exactly what their runtime tests exercise. The Node, C#, Java, Kotlin and
Go packages bundle or expect native libraries only for the platforms the release runner builds (Linux x86_64
today); other platforms use the release assets or a source build. Maven Central and Hackage releases cannot
be deleted — read the job logs of the first release.

### Project registries

Besides the public indexes, every language also goes to GitHub Packages (Maven, npm, NuGet, and OCI bundles
on `ghcr.io` for Go, Dart and Haskell) and/or your Gitea instance's own registries
(`--target gitea --base-url http://host:3000`, secret `PACKAGES_TOKEN` with `write:package`). Kotlin's Android AAR
and the Swift XCFrameworks need the GitHub workflow (NDK / macOS). A manual workflow
(`.github/workflows/publish-packages.yml`) dry-runs or publishes a throw-away version such as `0.0.0-rc.1`
without cutting a release. See [`packaging/README.md`](packaging/README.md) for the artifact layout.

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

### Bindings, packages and CI

```sh
just install-bindgens          # the pinned third-party generators (scripts/bindings/spec.nu)
just test-bindings             # runtime-test every language (or: just test-bindings go java)
just package-verify-all        # compile + pack every publishable package, no upload
just ci-bindings               # all of the above, strict — what the Gitea CI runs
just clean-bindings            # remove generated artifacts and scratch dirs
just check-bindgen-updates     # are the pinned generators behind their upstream? (weekly in CI)
just ci-image                  # CI image with every toolchain + generator pre-installed
```

CI runs on Gitea (`.gitea/workflows/ci.yml`, Linux runner; also mirrored in
`.github/workflows/ci.yml`): fmt, clippy, test, doc, the nu tests, then one
`test-bindings-runtime` job per language, the Kotlin Gradle test, and one `test-packaging` job per
package. A weekly scheduled run catches upstream generator drift.

### Nushell scripts

Deterministic release/CI plumbing (tag validation, version bump, changelog,
crates.io publish, quality gate) lives in `scripts/*.nu` as tested Nushell
scripts rather than ad-hoc shell in CI YAML — see
[`scripts/tests/`](scripts/tests) (`nu scripts/tests/run_all.nu`, also
reachable as `just test-nu`).

## CI/CD

- **Gitea** (`.gitea/workflows/`) — the Linux runner that does the real work: `ci.yml` (fmt, clippy, tests,
  docs, nu tests, a runtime test and a package build per language, toolchain setup retried once on network
  flakes), `release.yml` (on a `vX.Y.Z` tag: broker binaries for Linux and Windows, Gitea release, then crates.io
  and every registry above), `deps-update.yml`.
- **GitHub** (`.github/workflows/`) — mirrors CI and adds what needs GitHub-hosted runners: macOS/iOS/Android
  builds, the Swift package, GitHub Packages, PyPI wheels for several platforms, and `publish-pubdev.yml`.

## Release process

```sh
just release <version>   # preflight + bump + quality gate + tag + push
```

See the [`justfile`](justfile) for the multi-remote (GitHub + Gitea) workflow: `just bump`, `just release-all`,
`just sync-gitea`, `just migrate-gitea`, …

## License

MIT — see [LICENSE](LICENSE).
