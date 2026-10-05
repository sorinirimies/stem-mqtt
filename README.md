# stem-mqtt

MQTT 3.1.1 / MQTT 5.0 client and broker, written in Rust and exposed via
[UniFFI](https://mozilla.github.io/uniffi-rs/) to Kotlin, Swift, Python, Go, C#, Java, Dart,
Node.js and Haskell — every binding runtime-tested against a real broker in CI — plus a
hand-written [napi-rs](https://napi.rs/) Node.js client addon.

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

Every published package name is prefixed `stem-mqtt-` —
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
    // JVM/desktop, client + broker:
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-kotlin:<version>")
    // Android AAR, client + broker, with all four Android ABIs:
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-android:<version>")
}
```

Reading a GitHub Package requires an authenticated `GITHUB_TOKEN`/PAT with
`read:packages`, including for a public repository. Full details:
[`packaging/kotlin/README.md`](packaging/kotlin/README.md).

### Swift

Download `StemMqttSwift-<version>.zip` from the matching GitHub Release,
extract it, then add the contained `StemMqttSwift` directory as a local Swift
package. It includes client and broker products plus prebuilt XCFrameworks for
macOS, iOS devices, and iOS simulators.

```swift
import MqttClient
import MqttBroker
```

Full details: [`packaging/swift/README.md`](packaging/swift/README.md).

### Python (PyPI)

```sh
pip install stem-mqtt-client   # client
pip install stem-mqtt-broker   # broker
```

```python
import mqtt_client  # import name unaffected by the package rename

options = mqtt_client.ConnectOptions(
    host="localhost", port=1883, client_id="demo",
    version=mqtt_client.MqttVersion.V5,
    clean_start=True, keep_alive_secs=30,
    username=None, password=None, will=None,
    connect_timeout_secs=10, operation_timeout_secs=15,
    auto_reconnect=True, reconnect_backoff_secs=1,
    reconnect_max_backoff_secs=30, tls=None,
)
client = mqtt_client.MqttClient(options)
```

Not yet published (requires the `PYPI_API_TOKEN` repository secret to be
configured — the release workflow's `publish-python` job skips gracefully
until then). Full details: [`packaging/python/README.md`](packaging/python/README.md).

### Ruby

Not currently supported. UniFFI 0.29 has no official Ruby backend, and no
production-ready Ruby generator is available. See
[`packaging/ruby/README.md`](packaging/ruby/README.md).

## Workspace layout

| Crate | Description |
| --- | --- |
| [`crates/mqtt-client`](crates/mqtt-client) | Async, `tokio`-based MQTT client. Also hosts the wire-protocol codec (`mqtt_client::protocol`) shared by the broker. |
| [`crates/mqtt-broker`](crates/mqtt-broker) | Standalone MQTT broker (library + `mqtt-broker` CLI binary), reusing the client crate's codec instead of duplicating it. |
| [`tests/bindings`](tests/bindings) | One end-to-end smoke test per language, run by `scripts/test_bindings.nu`. |
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
- **Wire codec for all control packet types** — CONNECT/CONNACK, PUBLISH and
  its ack chain, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PING, DISCONNECT,
  and MQTT 5 AUTH. Enhanced-authentication flows beyond packet encoding are
  not currently implemented.
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
  `uniffi-bindgen` binary for Kotlin, Swift and Python; Go, C#, Java, Dart,
  Node.js and Haskell are generated through pinned community generators.
- **Hardened by default** — packet-size limits, bounded per-client send queues,
  keep-alive timeout detection, credential-redacting `Debug`, and foreign
  callbacks that can't crash I/O tasks (see `CHANGELOG.md`).
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

Everything is driven by [Nushell](https://www.nushell.sh/) scripts; the generator
table (languages, pinned generator versions, required UniFFI release, registry)
lives in one file, [`scripts/bindings/spec.nu`](scripts/bindings/spec.nu).

```sh
nu scripts/generate_bindings.nu kotlin            # kotlin | swift | python (built into UniFFI)
nu scripts/install_bindgens.nu                    # install the pinned third-party generators
nu scripts/generate_bindings.nu go mqtt-broker    # go | csharp | java | dart | node | node-livekit | haskell
just bindings-all                                 # or: just bindings-third-party
```

| Language | Generator | Runtime-tested | Status |
| --- | --- | :-: | --- |
| Kotlin, Swift | UniFFI (built in) | CI packaging tests | stable |
| Python | UniFFI (built in) | ✅ | stable |
| Go | [uniffi-bindgen-go](https://github.com/NordSecurity/uniffi-bindgen-go) | ✅ client + broker + all callbacks | stable |
| C# | [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) | ✅ client + broker + all callbacks | stable |
| Java (JDK 22+) | [uniffi-bindgen-java](https://github.com/IronCoreLabs/uniffi-bindgen-java) | ✅ client + broker + all callbacks | stable |
| Dart | [uniffi-dart](https://github.com/acterglobal/uniffi-dart) | ✅ client + broker through the pull-style API ² | experimental |
| Node.js | [uniffi-bindgen-node-js](https://github.com/criccomini/uniffi-bindgen-node-js) | ✅ client + listener callback; broker ❌ ¹ | experimental |
| Node.js (early dev.) | [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node) | ❌ broken ³ | not published |
| Haskell | [uniffi-bindgen-haskell](https://github.com/mercury/uniffi-bindgen-haskell) + [our patch](packaging/haskell) (PR [#3](https://github.com/mercury/uniffi-bindgen-haskell/pull/3)) | ✅ client + broker through the pull-style API ² | experimental |

¹ That generator rejects UniFFI *external types* and the broker imports `QoS`
from the client crate.
² Dart: foreign callbacks cannot be invoked from Rust's own threads (the VM aborts). Haskell: callback
interfaces are exposed only as opaque handles. So `MqttMessageListener`, `MqttAuthProvider`,
`MqttBrokerEventListener` and the enhanced-auth callbacks can't be implemented there — use the
**pull-style API** that exists for exactly this: `enable_message_queue` / `next_message(timeout_ms)` on the
client and `enable_event_queue` / `next_event(timeout_ms)` on the broker. Their smoke tests receive
messages and observe broker events that way. Authentication uses the built-in `allow_anonymous` rule.
³ Emits TypeScript referencing an undefined `FfiConverterBytes`; no client can be constructed.
⁴ Upstream's Haskell generator had three bugs that made every binding unusable: constructors never
lowered their arguments (generated code didn't compile), flat-error variants dropped their message
(decoder failed with "left N trailing bytes"), and records sharing a field name broke the public module.
[`packaging/haskell/uniffi-bindgen-haskell.patch`](packaging/haskell/uniffi-bindgen-haskell.patch)
fixes all three (and is proposed upstream, see [`packaging/haskell`](packaging/haskell)); `install_bindgens.nu` builds the generator from the pinned revision with the patch applied.

³ is tracked as `known-broken` in `scripts/bindings/spec.nu`: its smoke test keeps running and
reports `XFAIL`; the day upstream fixes it it reports `XPASS` and fails CI so the flag gets removed.

UniFFI bindgens can only read metadata from the UniFFI release they were built
for, and the third-party generators disagree (most target 0.31, livekit Node
0.30, Haskell 0.32). `generate_bindings.nu` therefore builds the library for
such a generator in a scratch workspace under `target/uniffi-<version>/` pinned
to the right release; your workspace is never touched.

The hand-written napi-rs addon (`crates/mqtt-client-node`) remains the
production Node client:

```sh
cd crates/mqtt-client-node && npm ci && npm run build:debug
```

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
header of each test). Verdicts: `PASS`, `FAIL`, `SKIP` (toolchain missing), `XFAIL`/`XPASS`
(known upstream breakage). CI (Gitea `.gitea/workflows/ci.yml`, Linux runner; mirrored in
`.github/workflows/ci.yml`) runs the same command per language (`test-bindings-runtime`), builds each
package (`test-packaging`: `dotnet pack`, `gradle build`, `npm pack`, OCI bundle) and runs the Kotlin
Gradle test; the release workflow runs the runtime test before packaging each language.

### GitHub Packages

```sh
nu scripts/publish_packages.nu stage <language>                  # bindings + native libs -> dist/<language>
nu scripts/publish_packages.nu publish <language> <version> --dry-run
```

GitHub Packages hosts Maven, npm, NuGet and containers only, so each language
goes where it fits and the rest ship as OCI artifacts on `ghcr.io`:

| Language | Package |
| --- | --- |
| Kotlin | Maven (existing Gradle flow, `packaging/kotlin`) |
| Java | Maven — `com.github.sorinirimies.stemmqtt:stem-mqtt-java` |
| C# | NuGet — `StemMqtt` |
| Node.js | npm — `@sorinirimies/stem-mqtt-node` |
| Go, Dart, Haskell | `ghcr.io/sorinirimies/stem-mqtt-<language>:<version>` (`oras pull`) |

The release workflow stages each package on Linux and macOS, merges the native
libraries, and publishes once.

See [`packaging/README.md`](packaging/README.md) for release artifact layout,
and [`packaging/python`](packaging/python),
[`packaging/kotlin`](packaging/kotlin), and
[`packaging/swift`](packaging/swift) for language-specific packaging.

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

- **GitHub** (`.github/workflows/`): `ci.yml` (fmt/clippy/test/build/doc/nu/node
  tests), `release.yml` (on `vX.Y.Z` tag: cross-platform `mqtt-broker` +
  `mqtt-client` native library artifacts, validated Kotlin/Swift/Python
  bindings, multi-platform Python wheels, a verified Swift client+broker
  package, a multi-platform Node addon, GitHub Release, then crates.io + PyPI +
  GitHub Packages (Kotlin) + npm publishing — token-gated registry jobs skip
  when their secret isn't configured), `auto-merge.yml` (Dependabot), `dependabot.yml`
  (GitHub Actions version bumps).
- **Gitea** (`.gitea/workflows/`): the CI above including the per-language binding runtime tests and
  package builds (it's the Linux runner these run on), plus a reduced Linux/Windows
  broker-binary release workflow (Apple/mobile and registry publishing remain
  GitHub-hosted), and `deps-update.yml` for dependency updates.

## Release process

```sh
just release <version>   # preflight + bump + quality gate + tag + push
```

See the [`justfile`](justfile) for the full release/version-bump/multi-remote
(GitHub + Gitea) workflow — `just bump`, `just release-all`, `just sync-gitea`,
`just migrate-gitea`, etc.

## License

MIT — see [LICENSE](LICENSE).
