# Swift packaging (client + broker)

`build_xcframework.sh` produces one self-contained Swift package archive with
both UniFFI APIs:

- `MqttClient` — client wrapper + `MqttClientFFI.xcframework`
- `MqttBroker` — broker wrapper + `MqttBrokerFFI.xcframework`

Each XCFramework contains macOS arm64/x86_64, iOS arm64, and iOS Simulator
arm64/x86_64 slices. The generated Swift wrappers are separate targets because
UniFFI emits crate-local helper names that would collide if both generated
files were compiled in one Swift module.

## Build locally

Requires macOS, Xcode command-line tools, Swift, and Rust targets for macOS/iOS:

```sh
./packaging/swift/build_xcframework.sh 0.2.3
# -> packaging/swift/dist/StemMqttSwift-0.2.3.zip
```

The script builds both Rust crates for every Apple target, generates both Swift
wrappers, assembles both XCFrameworks, then runs `swift build` against the
packaged result. A release fails if either wrapper or native framework cannot
compile together.

## Consume

Download `StemMqttSwift-<version>.zip` from the matching GitHub Release, extract
it, then add the extracted `StemMqttSwift` directory as a local Swift package.
Import either or both products:

```swift
import MqttClient
import MqttBroker
```

A remote `.package(url:from:)` declaration is intentionally not advertised.
SwiftPM reads `Package.swift` from the immutable git tag, while release-asset
checksums only exist after that tag's CI build. The release therefore ships a
verified self-contained package archive instead of committing a manifest whose
checksum points at a future artifact.
