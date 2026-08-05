#!/usr/bin/env bash
# Build a crate's cdylib and generate foreign-language bindings for it with
# its own uniffi-bindgen binary.
#
# Usage:
#   scripts/generate-bindings.sh <kotlin|swift|python> [crate] [out-dir]
#
#   crate    mqtt-client (default) or mqtt-broker
#   out-dir  bindings/<language>/<crate> by default
#
# Examples:
#   scripts/generate-bindings.sh kotlin
#   scripts/generate-bindings.sh swift mqtt-broker
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

language="${1:?usage: $0 <kotlin|swift|python> [crate] [out-dir]}"
crate="${2:-mqtt-client}"
lib_name="${crate//-/_}"
# The Cargo *package* name differs from this script's logical crate
# identifier (used for output dirs / lib basenames) since crates.io's
# "mqtt-client" was already taken by an unrelated project — our published
# package names are stem-mqtt-client / stem-mqtt-broker, but the `[lib]`
# name (and therefore the compiled .so/.dylib basename) stays mqtt_client /
# mqtt_broker, so callers of this script keep using the short names.
case "$crate" in
    mqtt-client) package="stem-mqtt-client" ;;
    mqtt-broker) package="stem-mqtt-broker" ;;
    *)           package="$crate" ;;
esac

if [ "$language" = "ruby" ]; then
    echo "ruby bindings use a separate generator — run packaging/ruby/build_and_publish.sh instead" >&2
    exit 1
fi

case "$(uname -s)" in
    Darwin) ext="dylib" ;;
    Linux)  ext="so" ;;
    *)      echo "unsupported host OS for local bindgen: $(uname -s)" >&2; exit 1 ;;
esac

out_dir="${3:-bindings/${language}/${crate}}"
lib_path="target/release/lib${lib_name}.${ext}"

echo "==> building ${crate} (release, cdylib)"
cargo build --release -p "${package}"

echo "==> generating ${language} bindings -> ${out_dir}"
mkdir -p "${out_dir}"
cargo run --release -p "${package}" --features uniffi/cli --bin uniffi-bindgen -- \
    generate --library "${lib_path}" --language "${language}" --out-dir "${out_dir}"

echo "done: ${out_dir}"
