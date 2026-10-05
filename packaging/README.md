# Packaging

Release packaging for the Rust MQTT client/broker and supported language
bindings. Every generated artifact is rebuilt from the release tag; generated
sources are not committed.

## Rust artifacts

The release workflow builds:

- `mqtt-broker` CLI for Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64
- `mqtt-client` native `cdylib`/`staticlib` for the same desktop targets
- crates.io packages `stem-mqtt-client` then `stem-mqtt-broker`

## UniFFI targets

UniFFI itself generates Kotlin, Swift and Python. Go, C#, Java, Dart, Node.js
and Haskell come from community generators pinned in
[`scripts/bindings/spec.nu`](../scripts/bindings/spec.nu). Both crates export
bindings for all of them (except the broker for the `node` generator — see the
root README).

```sh
nu scripts/install_bindgens.nu                      # once
nu scripts/generate_bindings.nu <language> mqtt-client
nu scripts/generate_bindings.nu <language> mqtt-broker
```

Release jobs reject empty generated directories before creating binding
archives. This prevents the zero-byte binding archives produced by older
releases.

### GitHub Packages (and Gitea)

`nu scripts/publish_packages.nu stage|verify|publish` builds, checks and publishes per-language
packages: Maven (Java), NuGet (C#), npm (Node) and OCI artifacts on `ghcr.io`
(Go, Dart, Haskell). Native libraries are shipped for Linux
x86_64 and macOS arm64; each package carries `native/<platform>/` and the
consumer must point the generated loader at it (Go: `CGO_LDFLAGS=-L…`; Dart:
`DynamicLibrary.open`; Haskell: link `libmqtt_*.a`).

### Kotlin

`packaging/kotlin/` builds combined client+broker packages:

- JVM: `stem-mqtt-kotlin`
- Android: `stem-mqtt-android`, containing both native libraries for four ABIs

See [`kotlin/README.md`](kotlin/README.md).

### Swift

`packaging/swift/build_xcframework.sh` builds and verifies a self-contained
local Swift package containing `MqttClient` and `MqttBroker`, with macOS/iOS
XCFrameworks. See [`swift/README.md`](swift/README.md).

### Python

Maturin builds separate client and broker wheels on Linux, macOS, and Windows.
See [`python/README.md`](python/README.md).

### Ruby

Not supported: UniFFI 0.29 has no official production Ruby backend. See
[`ruby/README.md`](ruby/README.md).

## Node.js / TypeScript

Node is not a UniFFI target. `crates/mqtt-client-node` is a hand-written napi-rs
client addon built for Linux x86_64/aarch64, macOS x86_64/arm64, and Windows
x86_64. See [`node/README.md`](node/README.md).

## Docker

Broker image:

```sh
docker build -t stem-mqtt-broker -f packaging/Dockerfile .
```

### CI image

`packaging/ci/Dockerfile` bakes every toolchain (Rust, Go, .NET, JDK 22, Dart, Node, Gradle, GHC/cabal,
Nushell) and every pinned binding generator into one image (`just ci-image`), so CI jobs can skip their
setup steps and the flaky downloads that come with them. See the file's header for how to use it with
`container:` in a workflow. Untested in this repository's own CI (the runners have no Docker).
