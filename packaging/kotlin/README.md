# Kotlin packaging (client + broker)

UniFFI-generated Kotlin bindings for both `mqtt-client` and `mqtt-broker` ship
as combined packages through GitHub Packages:

| Target | Maven coordinate | Contents |
| --- | --- | --- |
| JVM/desktop | `com.github.sorinirimies.stemmqtt:stem-mqtt-kotlin:<version>` | Both generated APIs plus `libmqtt_client` and `libmqtt_broker` for publishing host |
| Android | `com.github.sorinirimies.stemmqtt:stem-mqtt-android:<version>` | Both APIs plus both native libraries for arm64-v8a, armeabi-v7a, x86_64, x86 |

The broker owns `MqttBrokerError`; exported `start()`/`stop()` no longer return
an error imported from `mqtt-client`. This avoids UniFFI's external-Kotlin-error
limitation and keeps both lifecycle methods in generated Kotlin.

JVM and Android are independent Gradle builds. Combining AGP 9's built-in
Kotlin support and `kotlin("jvm")` in one Gradle build causes plugin classloader
collisions.

## Build locally

```sh
# JVM jar
just package-kotlin-jvm
cd packaging/kotlin && gradle clean build

# Android AAR
export ANDROID_NDK_HOME=/path/to/android-ndk
export ANDROID_HOME=/path/to/android-sdk
just package-kotlin-android
cd packaging/kotlin/android && gradle clean assembleRelease
```

`stage.sh` refuses empty generated sources. `stage-android.sh` refuses any ABI
missing either native library.

## Consume

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        mavenCentral()
        maven {
            url = uri("https://maven.pkg.github.com/sorinirimies/stem-mqtt")
            credentials {
                username = providers.gradleProperty("gpr.user")
                    .getOrElse(System.getenv("GITHUB_ACTOR") ?: "")
                password = providers.gradleProperty("gpr.token")
                    .getOrElse(System.getenv("GITHUB_TOKEN") ?: "")
            }
        }
    }
}
```

```kotlin
// Desktop JVM
implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-kotlin:<version>")

// Android
implementation("com.github.sorinirimies.stemmqtt:stem-mqtt-android:<version>")
```

GitHub Packages requires an authenticated token with `read:packages`, including
for public repositories.

## JVM platform caveat

The JVM jar currently embeds both native libraries for its publishing host
(Linux x86_64 in CI). Android includes all four supported ABIs. A universal
desktop jar would require collecting Linux/macOS/Windows native artifacts into
one publication.
