# Packaging

Release packaging for the Rust MQTT client/broker and supported language
bindings. Every generated artifact is rebuilt from the release tag; generated
sources are not committed.

## Rust artifacts

The release workflow builds:

- `mqtt-broker` CLI for Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64
- `mqtt-client` native `cdylib`/`staticlib` for the same desktop targets
- crates.io packages `stem-mqtt-client` then `stem-mqtt-broker`

## Elixir and Erlang (Hex)

`packaging/elixir` is the Hex package `stem_mqtt`: Elixir and Erlang modules over a Rustler NIF
(`crates/mqtt-beam`). The GitHub release builds the NIF per target (`build-nif`); `publish-hex` then
checksums those release assets, runs ExUnit against them and publishes (`ELIXIR_HEX_TOKEN`). Only
GitHub publishes to Hex, because the package downloads its NIFs from the GitHub release; the NIF
assets are exempt from release pruning. `just test-elixir` builds from source and runs the tests.

## UniFFI targets

UniFFI itself generates Kotlin, Swift and Python. Go, C#, Java, Dart, Node.js
and Haskell come from community generators pinned in
[`scripts/bindings/spec.nu`](../scripts/bindings/spec.nu). Both crates export
bindings for all of them.

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
(Go, Dart, Haskell).

Every package carries only its platform's native libraries:

- **npm, Maven, RubyGems:** one package per platform (npm optional dependencies restricted by
  `os`/`cpu`, Maven classifier jars, platform gems) for Linux x86_64/aarch64, macOS x86_64/arm64 and
  Windows x86_64, built by the release's `build-native-libs` job (`scripts/ci/native_libs.nu`).
- **Python, Hex, Kotlin/Android, Swift:** per-platform wheels, per-target NIF downloads, per-ABI
  `jniLibs` (split by the Play Store), an XCFramework (Apple's per-architecture slices).
- **NuGet and the OCI bundles:** the publishing host's libraries (Linux x86_64) under
  `native/<platform>/`; the consumer points the generated loader at them (Go: `CGO_LDFLAGS=-L…`;
  Dart: `DynamicLibrary.open`; Haskell: link `libmqtt_*.a`).

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

Per-platform gems `stem-mqtt-client` / `stem-mqtt-broker` on rubygems.org, from UniFFI's built-in
Ruby bindings. See [`ruby/README.md`](ruby/README.md).

## Node.js / TypeScript

Node is a UniFFI target like the others (`uniffi-bindgen-node-js`): one npm package,
`stem-mqtt` on npmjs.org (`@<owner>/stem-mqtt-node` on GitHub Packages / Gitea), with two entry points —
`/client` and `/broker` — each a generated ESM package sharing one `runtime/`. The native libraries are in
one package per platform (`stem-mqtt-<platform>-<arch>[-gnu]`), optional dependencies npm installs only
where they match. (It replaces the hand-written napi-rs client addon.)

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
