# stem-mqtt

Full MQTT 3.1.1 / MQTT 5.0 client and broker, written in Rust and exposed
to Kotlin, Swift, Python, and Ruby via [UniFFI](https://mozilla.github.io/uniffi-rs/),
plus Node.js/TypeScript via a hand-written [napi-rs](https://napi.rs/) addon
(JavaScript isn't a UniFFI target).

[![CI](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/sorinirimies/stem-mqtt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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
  four-part handshake (PUBLISH → PUBREC → PUBREL → PUBCOMP).
- **All standard packet types** — CONNECT/CONNACK, PUBLISH and its ack
  chain, SUBSCRIBE/SUBACK, UNSUBSCRIBE/UNSUBACK, PING, DISCONNECT, and
  MQTT 5 AUTH.
- **Last Will and Testament**, retained messages, keep-alive pings.
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
cargo run -p mqtt-client --example pub_sub
cargo run -p mqtt-client --example will_and_retain
cargo run -p mqtt-broker --example simple_broker
cargo run -p mqtt-broker --example auth_broker
```

(`pub_sub`/`will_and_retain` need a broker already running — either
`simple_broker`/`auth_broker` above or the `mqtt-broker` CLI below.)

## Running the broker

```sh
cargo run -p mqtt-broker --bin mqtt-broker -- --bind 0.0.0.0 --port 1883
```

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
