# Packaging reference — stem-mqtt

One crate/language-target's build lives under `packaging/<lang>/` with its
own README documenting caveats.

| Target | Location | What it produces |
| --- | --- | --- |
| Kotlin/JVM | `packaging/kotlin/` | Plain jar, JNA resource-embedded native lib (desktop JVM only) |
| Kotlin/Android | `packaging/kotlin/android/` | Real AAR, `jniLibs/<abi>/`, cross-compiled via `cargo-ndk` |
| Swift | `packaging/swift/` | XCFramework: macOS + iOS device + iOS simulator slices |
| Python | `packaging/python/` | maturin wheels |
| Ruby | `packaging/ruby/` | gem, via `uniffi-bindgen-ruby` |
| Node | `crates/mqtt-client-node/` + `packaging/node/` | napi-rs prebuilds |

Both Kotlin packaging modules (JVM + Android) currently ship `mqtt-client`
**only**, not `mqtt-broker` — see Pitfalls below for why.

## Kotlin/Android AAR — key facts

- `packaging/kotlin/android/` is its **own independent Gradle build** (own
  `settings.gradle.kts`), not a subproject of the JVM module's build. See
  Pitfalls: sharing one Gradle build/classpath between a module applying
  `kotlin("jvm")` and a module using AGP 9's built-in Kotlin causes
  plugin-classloader collisions.
- `com.android.library` alone, no `org.jetbrains.kotlin.android` plugin —
  AGP 9's built-in Kotlin compiles the sources itself and is incompatible
  with also applying that plugin.
- Native libs cross-compiled for `arm64-v8a`/`armeabi-v7a`/`x86_64`/`x86`
  via `packaging/kotlin/stage-android.sh` (uses `cargo-ndk`; needs
  `ANDROID_NDK_HOME` set).
- JNA ships a dedicated `@aar`-classified artifact
  (`net.java.dev.jna:jna:5.14.0@aar`) — this is what makes the exact same
  generated Kotlin bindings source work unchanged on Android; only the
  native-lib delivery mechanism differs (jniLibs vs. JNA's desktop
  resource-extraction trick, which doesn't work on Android due to W^X
  policy).

## Swift XCFramework — key facts

- 3 slices: `macos-arm64_x86_64`, `ios-arm64` (device), and
  `ios-arm64_x86_64-simulator` (universal simulator lib).
- Built via `cargo build --target aarch64-apple-darwin` /
  `x86_64-apple-darwin` / `aarch64-apple-ios` / `aarch64-apple-ios-sim` /
  `x86_64-apple-ios`, `lipo`'d together where a slice needs more than one
  arch.
- `packaging/swift/build_xcframework.sh <version>` does the whole thing end
  to end, including generating the Swift bindings via `uniffi-bindgen`.

## Verification habit for any packaging change

Always try to actually build+run it locally (not just write
plausible-looking config) before committing. This repo's existing
Kotlin/Swift packaging had config bugs (missing dependency, macOS-only
XCFramework despite being called "Swift packaging") that were never
build-tested until an actual local build caught them:

- `cargo build` for the target triple, then inspect the produced binary.
- `gradle build` / `gradle assembleRelease`, then `unzip -l` the resulting
  jar/AAR to confirm the expected files are actually inside it.
- `maturin build`, then inspect the wheel contents.

## Pitfalls specific to packaging

- **mqtt-broker's Kotlin bindings hit a real upstream uniffi-rs bug**
  ([External Kotlin errors don't work, #2392](https://github.com/mozilla/uniffi-rs/issues/2392)):
  `MqttBroker::start`/`stop` return `mqtt_client::MqttError`, which is
  "external" to the `mqtt-broker` crate being bound — Kotlin bindgen
  silently drops those two methods from the generated class (emits a
  `// Sorry, the callable "start" isn't supported.` comment instead),
  leaving `MqttBroker` unable to compile. Workaround until fixed upstream
  (or until `mqtt-broker` gets its own local error type): only package
  `mqtt-client`'s Kotlin bindings, not `mqtt-broker`'s.
- **Missing kotlinx-coroutines-core**: any UniFFI Kotlin module exposing
  async-exported methods (`#[uniffi::export(async_runtime = "tokio")]`)
  needs `org.jetbrains.kotlinx:kotlinx-coroutines-core` as a dependency, or
  `compileKotlin` fails with "Unresolved reference kotlinx".
- **Don't nest the Android AGP module under the JVM module's Gradle
  build** — see "Kotlin/Android AAR" above.
- **AGP 9's built-in Kotlin** is NOT compatible with also applying the
  `org.jetbrains.kotlin.android` plugin.
- **Swift packaging was macOS-only** for a long time despite being called
  "Swift packaging" — always check `packaging/*/README.md`'s Caveats
  section before assuming a packaging target is actually complete.
