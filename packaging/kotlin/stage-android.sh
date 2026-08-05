#!/usr/bin/env bash
# Cross-compiles mqtt-client + mqtt-broker for every Android ABI via
# cargo-ndk, staging the resulting .so files into
# packaging/kotlin/android/staged-jniLibs/<abi>/ for the :android AAR
# module (packaging/kotlin/android/build.gradle.kts) to bundle.
#
# Requires:
#   - `cargo install cargo-ndk --locked`
#   - Android NDK, with $ANDROID_NDK_HOME (or $ANDROID_NDK_ROOT) set
#     (CI: nttld/setup-ndk; locally: `brew install --cask android-ndk` on
#     macOS, then `export ANDROID_NDK_HOME=/opt/homebrew/share/android-ndk`)
#
# Usage: packaging/kotlin/stage-android.sh
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

# cargo-ndk's own ABI names.
abis=(arm64-v8a armeabi-v7a x86_64 x86)

# mqtt-client only — see stage.sh for why mqtt-broker's Kotlin bindings
# aren't packaged (upstream uniffi-rs Kotlin-bindgen bug with external
# error types, https://github.com/mozilla/uniffi-rs/issues/2392).
echo "==> cross-compiling mqtt-client for: ${abis[*]}"
cargo ndk \
    -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 \
    -o "$out" \
    build --release -p stem-mqtt-client

echo "staged Android native libs -> ${out}/{${abis[*]}}/"
