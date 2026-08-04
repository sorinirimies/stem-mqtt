# Swift packaging (XCFramework + Swift Package Manager)

SPM has no external registry like npm/PyPI/crates.io — the package
"registry" entry *is* this git repository, referenced by tag. Consumers add
it as a Swift Package dependency directly:

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/sorinirimies/stem-mqtt", from: "0.2.0"),
]
```

pointing at `packaging/swift/Package.swift`, which declares a
`.binaryTarget(url:checksum:)` for a prebuilt `MqttClient.xcframework`
(macOS arm64 + x86_64, universal).

## How a release updates it

1. `packaging/swift/build_xcframework.sh <version>` — builds `mqtt-client`
   for macOS (arm64 + x86_64), iOS device (arm64), and the iOS simulator
   (arm64 + x86_64, universal), generates the Swift bindings via
   `uniffi-bindgen`, `lipo`s each multi-arch slice into a universal static
   lib, and assembles `MqttClient.xcframework` with
   `xcodebuild -create-xcframework` (3 slices: `macos-arm64_x86_64`,
   `ios-arm64`, `ios-arm64_x86_64-simulator`).
2. The zipped XCFramework is attached to the GitHub release as an asset.
3. `packaging/swift/update_manifest.sh <tag>` computes its checksum
   (`swift package compute-checksum`) and rewrites `Package.swift` to point
   at that release asset + checksum.
4. CI commits the updated `Package.swift` back to `main`.

Both scripts only run on macOS (`xcodebuild`/`lipo`/`swift` aren't available
elsewhere), which is why `publish-swift` in
`.github/workflows/release.yml` uses `runs-on: macos-latest`.

## Building locally

```sh
./packaging/swift/build_xcframework.sh 0.0.0-dev
# -> packaging/swift/dist/MqttClient-0.0.0-dev.xcframework.zip
```

## Caveats

- `mqtt-broker`'s Swift bindings aren't packaged here (a broker embedded in
  an iOS/macOS app is a less common use case than the client); follow the
  same pattern with a second XCFramework if needed.
