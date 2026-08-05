#!/usr/bin/env bash
# Stages generated Kotlin bindings + the release cdylib into
# packaging/kotlin/staged/ so `gradle publish` (run from packaging/kotlin/)
# can package them into one JVM jar.
#
# Usage: packaging/kotlin/stage.sh <version>
#
# Expects `scripts/generate-bindings.sh kotlin mqtt-client` to have already
# been run (the release workflow does this before calling this script) —
# see .github/workflows/release.yml's `publish-kotlin` job.
#
# mqtt-client only — mqtt-broker's Kotlin bindings hit a real upstream
# uniffi-rs bug (https://github.com/mozilla/uniffi-rs/issues/2392,
# "External Kotlin errors don't work"): `MqttBroker::start`/`stop` return
# `mqtt_client::MqttError`, which is "external" to the mqtt-broker crate
# being bound, so Kotlin bindgen silently drops those two methods from the
# generated class entirely (emits a `// Sorry, ... isn't supported.`
# comment instead), leaving `MqttBroker` unable to compile. Swift/Python/
# Ruby bindings don't hit this (see packaging/swift, packaging/python,
# packaging/ruby) — it's specific to the Kotlin backend. Re-add mqtt-broker
# here once that's fixed upstream, or once mqtt-broker defines its own
# non-external error type for start/stop.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
out="packaging/kotlin/staged"
rm -rf "$out"
mkdir -p "$out/kotlin" "$out/resources"

crate="mqtt-client"
package="stem-mqtt-client" # Cargo package name (see scripts/generate-bindings.sh for why this differs from `crate`)
lib_name="${crate//-/_}"
src="bindings/kotlin/${crate}"
if [ -d "$src" ]; then
    echo "==> staging Kotlin sources for ${crate}"
    cp -r "$src"/. "$out/kotlin/"
fi

# Native library, named the way UniFFI's Kotlin JVM loader expects
# (platform-qualified resource path; adjust per target when building a
# true multi-platform jar with one native lib per OS/arch).
case "$(uname -s)" in
    Darwin) ext="dylib"; os_dir="darwin" ;;
    Linux)  ext="so"; os_dir="linux-x86-64" ;;
    *) echo "unsupported host OS for staging: $(uname -s)" >&2; exit 1 ;;
esac
lib_path="target/release/lib${lib_name}.${ext}"
if [ -f "$lib_path" ]; then
    mkdir -p "$out/resources/${os_dir}"
    cp "$lib_path" "$out/resources/${os_dir}/"
else
    echo "==> building ${crate} release cdylib (not found at ${lib_path})"
    cargo build --release -p "${package}"
    mkdir -p "$out/resources/${os_dir}"
    cp "$lib_path" "$out/resources/${os_dir}/"
fi

echo "staged version ${version} -> ${out}"
