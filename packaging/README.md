# Packaging

How `stem-mqtt` is built into distributable artifacts. There is no packaging
tooling to install locally beyond `cargo` and (for cross-compiled Linux
targets) [`cross`](https://github.com/cross-rs/cross) — everything here is
driven by plain scripts and GitHub Actions, not a bespoke build system.

## `mqtt-broker` binary

The [`mqtt-broker`](../crates/mqtt-broker) CLI binary is built for:

| Target | How |
| --- | --- |
| `x86_64-unknown-linux-gnu` | native `cargo build` on `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | `cross build` (see [`Cross.toml`](Cross.toml)) |
| `x86_64-apple-darwin` | native `cargo build` on `macos-latest` |
| `aarch64-apple-darwin` | native `cargo build` on `macos-latest` |

`.github/workflows/release.yml` builds all four on every `vX.Y.Z` tag,
packages each as `mqtt-broker-<target>.tar.gz` (binary + README + LICENSE),
and attaches them to the GitHub release.

## UniFFI bindings

Both `mqtt-client` and `mqtt-broker` build as `cdylib`/`staticlib` and ship a
`uniffi-bindgen` binary. [`../scripts/generate-bindings.sh`](../scripts/generate-bindings.sh)
wraps the two-step "build the cdylib, then run its own bindgen against it"
process:

```sh
../scripts/generate-bindings.sh <kotlin|swift|python> [crate] [out-dir]
```

Release builds generate bindings for both crates in all three languages and
publish them as `bindings-<language>.zip` alongside the broker binaries.
Bindings are **not** committed to the repo — always regenerate them from the
matching release/commit, since UniFFI embeds a checksum tying generated code
to the exact `cdylib` it was generated from.

### Consuming the bindings

- **Kotlin/Android** — bundle `libmqtt_client.so` (per-ABI) under
  `jniLibs/<abi>/` alongside the generated `.kt` file.
- **Swift/iOS** — wrap the generated `.swift` + `.h`/modulemap and the
  `libmqtt_client.a` static lib in an XCFramework.
- **Python** — the generated `.py` module loads the `cdylib` at import time;
  keep it next to the module or set `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`.

## Docker (broker only)

No published image yet. Build one locally:

```sh
cargo build --release -p mqtt-broker --bin mqtt-broker
docker build -t stem-mqtt-broker -f packaging/Dockerfile .
```
