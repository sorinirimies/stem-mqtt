#!/usr/bin/env bash
# Builds an XCFramework bundling mqtt-client for macOS, iOS device, and the
# iOS simulator, and generates the matching Swift bindings — ready to ship
# as an SPM binary target.
#
#   macOS         -> universal (arm64 + x86_64) static lib
#   iOS device    -> arm64 static lib (Apple dropped 32-bit/x86 devices)
#   iOS simulator -> universal (Apple Silicon arm64 + Intel x86_64) static lib
#
# Usage: packaging/swift/build_xcframework.sh <version>
#
# Output: packaging/swift/dist/MqttClient-<version>.xcframework.zip
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
crate="mqtt-client"
lib_name="mqtt_client"
dist="packaging/swift/dist"
work="packaging/swift/.build"

rm -rf "$dist" "$work"
mkdir -p "$dist" "$work"

# rustup targets required for the iOS device + simulator slices. macOS
# targets (aarch64-apple-darwin / x86_64-apple-darwin) are assumed already
# installed, as they were before this script grew iOS support.
ios_targets=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
for t in "${ios_targets[@]}"; do
    rustup target add "$t" >/dev/null 2>&1 || true
done

echo "==> building ${crate} (release, staticlib) for macOS + iOS device + iOS simulator"
cargo build --release -p "${crate}" --target aarch64-apple-darwin
cargo build --release -p "${crate}" --target x86_64-apple-darwin
cargo build --release -p "${crate}" --target aarch64-apple-ios
cargo build --release -p "${crate}" --target aarch64-apple-ios-sim
cargo build --release -p "${crate}" --target x86_64-apple-ios

echo "==> creating universal (lipo'd) static libs: macOS (arm64+x86_64), iOS simulator (arm64+x86_64)"
mkdir -p "$work/macos-universal" "$work/ios-simulator-universal"
lipo -create \
    "target/aarch64-apple-darwin/release/lib${lib_name}.a" \
    "target/x86_64-apple-darwin/release/lib${lib_name}.a" \
    -output "$work/macos-universal/lib${lib_name}.a"
lipo -create \
    "target/aarch64-apple-ios-sim/release/lib${lib_name}.a" \
    "target/x86_64-apple-ios/release/lib${lib_name}.a" \
    -output "$work/ios-simulator-universal/lib${lib_name}.a"
# iOS device has only one slice (arm64) — no lipo needed, used directly
# from target/aarch64-apple-ios/release/ below.

echo "==> generating Swift bindings"
cargo build --release -p "${crate}" --features uniffi/cli
mkdir -p "$work/swift"
cargo run --release -p "${crate}" --features uniffi/cli --bin uniffi-bindgen -- \
    generate --library "target/release/lib${lib_name}.dylib" \
    --language swift --out-dir "$work/swift"

echo "==> assembling the XCFramework headers module"
headers_dir="$work/headers"
mkdir -p "$headers_dir"
cp "$work/swift"/*.h "$headers_dir/" 2>/dev/null || true
cp "$work/swift"/*.modulemap "$headers_dir/module.modulemap" 2>/dev/null || true

echo "==> building MqttClient.xcframework (macOS + iOS device + iOS simulator)"
xcodebuild -create-xcframework \
    -library "$work/macos-universal/lib${lib_name}.a" -headers "$headers_dir" \
    -library "target/aarch64-apple-ios/release/lib${lib_name}.a" -headers "$headers_dir" \
    -library "$work/ios-simulator-universal/lib${lib_name}.a" -headers "$headers_dir" \
    -output "$dist/MqttClient.xcframework"

cp "$work/swift"/*.swift "$dist/" 2>/dev/null || true

echo "==> zipping for release + checksum"
(cd "$dist" && zip -r "MqttClient-${version}.xcframework.zip" "MqttClient.xcframework")

echo "done: $dist/MqttClient-${version}.xcframework.zip"
