#!/usr/bin/env bash
# Stage UniFFI-generated Kotlin bindings plus client/broker native libraries
# into packaging/kotlin/staged/ for JVM jar and Android AAR builds.
#
# Usage: packaging/kotlin/stage.sh <version>
#
# Expects both generators to have run first:
#   scripts/generate-bindings.sh kotlin mqtt-client
#   scripts/generate-bindings.sh kotlin mqtt-broker
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
out="packaging/kotlin/staged"
rm -rf "$out"
mkdir -p "$out/kotlin" "$out/resources"

crates=(mqtt-client mqtt-broker)

for crate in "${crates[@]}"; do
    src="bindings/kotlin/${crate}"
    if [ -z "$(find "$src" -type f -print -quit 2>/dev/null)" ]; then
        echo "error: no generated Kotlin bindings under ${src}" >&2
        echo "run: scripts/generate-bindings.sh kotlin ${crate}" >&2
        exit 1
    fi
    echo "==> staging Kotlin sources for ${crate}"
    cp -r "$src"/. "$out/kotlin/"
done

# Native libraries, named/path-qualified the way UniFFI's Kotlin/JNA loader
# expects. JVM CI currently publishes its native host (Linux x86_64); Android
# gets all four ABIs from stage-android.sh.
case "$(uname -s)" in
    Darwin) ext="dylib"; os_dir="darwin" ;;
    Linux)  ext="so"; os_dir="linux-x86-64" ;;
    *) echo "unsupported host OS for staging: $(uname -s)" >&2; exit 1 ;;
esac

mkdir -p "$out/resources/${os_dir}"
for crate in "${crates[@]}"; do
    case "$crate" in
        mqtt-client) package="stem-mqtt-client" ;;
        mqtt-broker) package="stem-mqtt-broker" ;;
    esac
    lib_name="${crate//-/_}"
    lib_path="target/release/lib${lib_name}.${ext}"
    if [ ! -f "$lib_path" ]; then
        echo "==> building ${crate} release cdylib"
        cargo build --release -p "$package"
    fi
    test -s "$lib_path" || { echo "error: missing native library ${lib_path}" >&2; exit 1; }
    cp "$lib_path" "$out/resources/${os_dir}/"
done

for required in mqtt_client.kt mqtt_broker.kt; do
    if [ -z "$(find "$out/kotlin" -name "$required" -type f -print -quit)" ]; then
        echo "error: staged Kotlin source ${required} missing" >&2
        exit 1
    fi
done

echo "staged client + broker version ${version} -> ${out}"
