# Kotlin packaging (GitHub Packages Maven registry)

`packaging/kotlin/` packages the UniFFI-generated Kotlin bindings for
`mqtt-client` in two flavors, both published to
[GitHub Packages](https://github.com/sorinirimies/stem-mqtt/packages) (a
Maven registry scoped to this repository — no external account needed
beyond `GITHUB_TOKEN`, which every workflow already has):

| Module | Artifact | What it is |
| --- | --- | --- |
| `packaging/kotlin/` (JVM) | `com.github.sorinirimies.stemmqtt:stem-mqtt-client-kotlin` | Plain Kotlin/JVM jar. Native lib embedded as a JNA-resource-path classpath resource (works on desktop JVM only). |
| `packaging/kotlin/android/` | `com.github.sorinirimies.stemmqtt:stem-mqtt-client-android` | Real Android AAR. Native libs under `jniLibs/<abi>/` for `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`. |

These are two **independent Gradle builds** (the Android module has its own
`settings.gradle.kts`), not a multi-project build — mixing AGP 9's built-in
Kotlin support with the JVM module's own `kotlin("jvm")` plugin application
in one shared Gradle build/classpath causes plugin-classloader collisions
(`Could not create an instance of type KotlinAndroidTarget`). Isolating them
into separate builds sidesteps this entirely.

## Why the JVM jar can't just work on Android too

JNA's normal "extract the native lib from a jar resource to a temp file,
then `dlopen` it" trick doesn't work on modern Android: apps can't execute
code extracted to arbitrary app-data paths (W^X policy, enforced since
Android 10). Native libs there *must* ship inside the APK's own
`jniLibs/<abi>/` directory so the OS extracts and loads them itself at
install time — hence a separate AAR, not a repackaged jar. JNA itself still
works identically once the `.so` is reachable that way (JNA ships a
dedicated `@aar`-classified artifact for exactly this), and both modules
reuse the *exact same* generated Kotlin bindings source
(`packaging/kotlin/staged/kotlin`) — only the native-lib delivery mechanism
and packaging format differ.

## `mqtt-broker` isn't packaged here

Only `mqtt-client` — `mqtt-broker`'s Kotlin bindings hit a real upstream
uniffi-rs bug:
[External Kotlin errors don't work (#2392)](https://github.com/mozilla/uniffi-rs/issues/2392).
`MqttBroker::start`/`stop` return `mqtt_client::MqttError`, a type "external"
to the `mqtt-broker` crate being bound, so Kotlin bindgen silently drops
those two methods from the generated class (emits a
`// Sorry, the callable "start" isn't supported.` comment instead), leaving
`MqttBroker` an incomplete implementation of its own interface — it doesn't
compile. Swift/Python/Ruby bindings don't hit this; it's specific to the
Kotlin backend. Fix options if this is needed later: track upstream #2392,
or give `mqtt-broker` its own local error type for `start`/`stop` instead of
reusing `mqtt-client`'s. (This is the same reason `packaging/swift/` only
packages `mqtt-client` — for a different reason there, see its README — so
the convention was already established.)

## How the JVM module fits together

1. CI generates Kotlin sources: `scripts/generate-bindings.sh kotlin mqtt-client`
   → `bindings/kotlin/mqtt-client/`.
2. `packaging/kotlin/stage.sh <version>` copies those sources plus the
   release cdylib into `packaging/kotlin/staged/{kotlin,resources}/`.
3. `gradle publish` (via `gradle/actions/setup-gradle`, no vendored wrapper
   needed) builds a jar from `staged/` and publishes it as
   `com.github.sorinirimies.stemmqtt:stem-mqtt-client-kotlin:<version>`.

## How the Android module fits together

1. Same Kotlin sources as the JVM module (`packaging/kotlin/staged/kotlin`)
   — no separate binding-generation step needed.
2. `packaging/kotlin/stage-android.sh` cross-compiles `mqtt-client` for
   `arm64-v8a`/`armeabi-v7a`/`x86_64`/`x86` via
   [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk) and stages the `.so`
   files into `packaging/kotlin/android/staged-jniLibs/<abi>/`.
3. `gradle publish` (run from `packaging/kotlin/android/`) uses AGP 9's
   built-in Kotlin support (no `org.jetbrains.kotlin.android` plugin needed
   — see the note below) to assemble and publish the AAR as
   `com.github.sorinirimies.stemmqtt:stem-mqtt-client-android:<version>`.

### A note on AGP 9's built-in Kotlin

AGP 9 compiles Kotlin sources itself by default and is **not compatible**
with also applying the separate `org.jetbrains.kotlin.android` plugin (see
[the AGP 9 release notes](https://developer.android.com/build/releases/agp-9-0-0-release-notes#android-gradle-plugin-built-in-kotlin)).
`packaging/kotlin/android/build.gradle.kts` therefore applies only
`com.android.library` — no Kotlin plugin at all.

## Consuming it

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        maven {
            url = uri("https://maven.pkg.github.com/sorinirimies/stem-mqtt")
            credentials {
                username = providers.gradleProperty("gpr.user").getOrElse(System.getenv("GITHUB_ACTOR") ?: "")
                password = providers.gradleProperty("gpr.token").getOrElse(System.getenv("GITHUB_TOKEN") ?: "")
            }
        }
    }
}
```

```kotlin
// build.gradle.kts — JVM/desktop app
dependencies {
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-client-kotlin:0.2.0")
}
```

```kotlin
// build.gradle.kts — Android app
dependencies {
    implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-client-android:0.2.0")
}
```

Reading a private GitHub Package still requires an authenticated
`GITHUB_TOKEN`/PAT with `read:packages`, even though the source repo is
public — that's a GitHub Packages platform limitation, not something this
project controls.

## Building locally

Requires a JDK, `gradle`, and — for the Android module — the Android SDK
(`ANDROID_HOME`), the NDK, and [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk)
(`cargo install cargo-ndk --locked`; on macOS, `brew install --cask android-ndk`
gets you an NDK at `/opt/homebrew/share/android-ndk`).

```sh
# JVM jar
just package-kotlin-jvm            # or: the two commands it wraps, see justfile
cd packaging/kotlin && gradle publishToMavenLocal

# Android AAR
export ANDROID_NDK_HOME=/opt/homebrew/share/android-ndk   # adjust to your install
just package-kotlin-android        # or: ./packaging/kotlin/stage-android.sh
cd packaging/kotlin/android && gradle publishToMavenLocal
```

## Caveats

The JVM jar packages **one native library for the host OS/arch that built
it** (matching the CI runner it ran on, currently Linux x86_64 via
`ubuntu-latest`). A production-grade multi-platform jar would bundle one
native lib per desktop OS/arch under classified resource paths — left as a
TODO since it requires staging artifacts from every OS runner into a single
publish job (the Android module doesn't have this problem: it always
bundles all 4 ABIs, since cross-compiling all of them from one CI runner is
cheap and that's what `stage-android.sh` already does).
