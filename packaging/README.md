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

UniFFI 0.29 officially supports Kotlin, Swift, and Python. Both stem-mqtt crates
export bindings for all three languages.

```sh
./scripts/generate-bindings.sh <kotlin|swift|python> mqtt-client
./scripts/generate-bindings.sh <kotlin|swift|python> mqtt-broker
```

Release jobs reject empty generated directories before creating binding
archives. This prevents the zero-byte binding archives produced by older
releases.

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
