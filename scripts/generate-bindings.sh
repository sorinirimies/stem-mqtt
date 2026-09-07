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

# UniFFI bindgen reads metadata symbols from the compiled library. Never let a
# workspace/profile override strip those symbols before generation (Linux/ELF
# otherwise succeeds with an empty output directory).
export CARGO_PROFILE_RELEASE_STRIP=none

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

case "$language" in
    kotlin|swift|python) ;;
    ruby)
        echo "Ruby is not an official UniFFI 0.29 backend and no production-ready generator is available." >&2
        echo "Supported languages: kotlin, swift, python" >&2
        exit 1
        ;;
    *)
        echo "unsupported language '$language' (expected kotlin, swift, or python)" >&2
        exit 1
        ;;
esac

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

if [ -z "$(find "${out_dir}" -type f -print -quit 2>/dev/null)" ]; then
    echo "error: ${language} binding generation produced no files in ${out_dir}" >&2
    exit 1
fi

echo "done: ${out_dir}"
