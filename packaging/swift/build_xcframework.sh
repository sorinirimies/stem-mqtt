#!/usr/bin/env bash
# Builds an XCFramework bundling mqtt-client for macOS (arm64 + x86_64) and
# generates the matching Swift bindings, ready to ship as an SPM binary
# target.
#
# Usage: packaging/swift/build_xcframework.sh <version>
#
# Output: packaging/swift/dist/MqttClient.xcframework.zip
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
crate="mqtt-client"
lib_name="mqtt_client"
dist="packaging/swift/dist"
work="packaging/swift/.build"

rm -rf "$dist" "$work"
mkdir -p "$dist" "$work"

echo "==> building ${crate} (release, staticlib) for both macOS targets"
cargo build --release -p "${crate}" --target aarch64-apple-darwin
cargo build --release -p "${crate}" --target x86_64-apple-darwin

echo "==> creating a universal (arm64 + x86_64) static lib"
mkdir -p "$work/universal"
lipo -create \
    "target/aarch64-apple-darwin/release/lib${lib_name}.a" \
    "target/x86_64-apple-darwin/release/lib${lib_name}.a" \
    -output "$work/universal/lib${lib_name}.a"

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

echo "==> building MqttClient.xcframework"
xcodebuild -create-xcframework \
    -library "$work/universal/lib${lib_name}.a" -headers "$headers_dir" \
    -output "$dist/MqttClient.xcframework"

cp "$work/swift"/*.swift "$dist/" 2>/dev/null || true

echo "==> zipping for release + checksum"
(cd "$dist" && zip -r "MqttClient-${version}.xcframework.zip" "MqttClient.xcframework")

echo "done: $dist/MqttClient-${version}.xcframework.zip"
