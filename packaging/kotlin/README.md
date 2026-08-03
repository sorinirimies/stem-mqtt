# Kotlin/JVM packaging (GitHub Packages Maven registry)

`packaging/kotlin/` is a minimal Gradle project that packages the
UniFFI-generated Kotlin bindings for both crates, plus the release `cdylib`,
into one JVM jar and publishes it to
[GitHub Packages](https://github.com/sorinirimies/stem-mqtt/packages) (a
Maven registry scoped to this repository — no external account needed
beyond `GITHUB_TOKEN`, which every workflow already has).

## How it fits together

1. CI generates Kotlin sources: `scripts/generate-bindings.sh kotlin mqtt-client`
   (and `mqtt-broker`) → `bindings/kotlin/<crate>/`.
2. `packaging/kotlin/stage.sh <version>` copies those sources plus the
   release cdylib into `packaging/kotlin/staged/{kotlin,resources}/`.
3. `gradle publish` (via `gradle/actions/setup-gradle`, no vendored wrapper
   needed) builds a jar from `staged/` and publishes it as
   `com.github.sorinirimies.stemmqtt:mqtt-client-kotlin:<version>`.

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
// build.gradle.kts
dependencies {
    implementation("com.github.sorinirimies.stemmqtt:mqtt-client-kotlin:0.2.0")
}
```

Reading a private GitHub Package still requires an authenticated
`GITHUB_TOKEN`/PAT with `read:packages`, even though the source repo is
public — that's a GitHub Packages platform limitation, not something this
project controls.

## Building locally

```sh
./scripts/generate-bindings.sh kotlin mqtt-client
./scripts/generate-bindings.sh kotlin mqtt-broker
./packaging/kotlin/stage.sh 0.0.0-dev
cd packaging/kotlin
gradle publishToMavenLocal
```

## Caveats

This packages **one native library for the host OS/arch that built it**
(matching the CI runner it ran on, currently Linux x86_64 via
`ubuntu-latest`). A production-grade multi-platform jar would bundle one
native lib per OS/arch under classified resource paths — left as a TODO
since it requires staging artifacts from every OS runner into a single
publish job.
