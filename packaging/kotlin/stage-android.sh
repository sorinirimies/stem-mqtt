#!/usr/bin/env bash
# Cross-compile mqtt-client + mqtt-broker for every Android ABI via cargo-ndk,
# staging both .so files into packaging/kotlin/android/staged-jniLibs/<abi>/.
#
# Requires cargo-ndk plus ANDROID_NDK_HOME (or ANDROID_NDK_ROOT).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

command -v cargo-ndk >/dev/null 2>&1 || {
    echo "❌ cargo-ndk not found. Install with: cargo install cargo-ndk --locked" >&2
    exit 1
}
: "${ANDROID_NDK_HOME:="${ANDROID_NDK_ROOT:-}"}"
if [ -z "$ANDROID_NDK_HOME" ]; then
    echo "❌ ANDROID_NDK_HOME (or ANDROID_NDK_ROOT) not set." >&2
    exit 1
fi
export ANDROID_NDK_HOME

out="packaging/kotlin/android/staged-jniLibs"
rm -rf "$out"
mkdir -p "$out"

abis=(arm64-v8a armeabi-v7a x86_64 x86)
echo "==> cross-compiling mqtt-client + mqtt-broker for: ${abis[*]}"
cargo ndk \
    -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 \
    -o "$out" \
    build --release -p stem-mqtt-client -p stem-mqtt-broker

for abi in "${abis[@]}"; do
    for lib in mqtt_client mqtt_broker; do
        path="$out/$abi/lib${lib}.so"
        test -s "$path" || { echo "error: missing Android library ${path}" >&2; exit 1; }
    done
done

echo "staged Android client + broker libs -> ${out}/{${abis[*]}}/"
