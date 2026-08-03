// swift-tools-version:5.9
// Placeholder manifest. `packaging/swift/update_manifest.sh` overwrites this
// file with a real `.binaryTarget(url:checksum:)` pointing at the
// XCFramework attached to the most recent release — see that script and
// the `publish-swift` job in `.github/workflows/release.yml`.
import PackageDescription

let package = Package(
    name: "MqttClient",
    platforms: [.macOS(.v11), .iOS(.v13)],
    products: [
        .library(name: "MqttClient", targets: ["MqttClient"]),
    ],
    targets: []
)
