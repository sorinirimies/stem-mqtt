// Android AAR module — the UniFFI Kotlin bindings (shared with the plain
// JVM module one level up) packaged as a proper Android library instead of
// a JNA-resource-extraction jar.
//
// Why this can't just be another resource directory in the JVM jar's
// classpath: JNA's normal "extract native lib from a jar resource to a
// temp file, then dlopen it" trick doesn't work on modern Android — apps
// can't execute code extracted to arbitrary app-data paths (W^X policy,
// enforced since Android 10). Native libs there *must* ship inside the
// APK's own `jniLibs/<abi>/` directory so the OS extracts and loads them
// itself at install time. Hence: a separate AAR, not a repackaged jar.
//
// JNA itself still works identically once the .so is reachable that way —
// this module reuses the exact same generated Kotlin bindings as the JVM
// module (`../staged/kotlin`), just with an Android-flavored JNA artifact
// (`@aar` classifier) and jniLibs instead of embedded jar resources.
plugins {
    id("com.android.library") version "9.3.1"
    `maven-publish`
}

group = "com.github.sorinirimies.stemmqtt"
version = (System.getenv("PACKAGE_VERSION") ?: "0.0.0-dev")

android {
    namespace = "com.github.sorinirimies.stemmqtt"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
    }

    sourceSets {
        getByName("main") {
            // Reuses the same UniFFI-generated Kotlin sources the JVM
            // module stages (packaging/kotlin/stage.sh) — the bindings
            // themselves are platform-agnostic Kotlin/JNA code; only the
            // native-lib delivery mechanism differs per platform.
            kotlin.directories.add("../staged/kotlin")
            // Populated by packaging/kotlin/stage-android.sh, one
            // libmqtt_client.so per ABI (mqtt-broker isn't packaged here —
            // see stage.sh for why).
            jniLibs.directories.add("staged-jniLibs")
        }
    }

    publishing {
        singleVariant("release") {
            withSourcesJar()
        }
    }
}

dependencies {
    // JNA ships a dedicated Android-flavored artifact (the `@aar`
    // classifier) alongside its normal desktop jar — this is what makes
    // the same UniFFI-generated bindings source work unchanged on Android.
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    // Required by UniFFI's generated suspend-fn bridging for our
    // async-exported methods (MqttClient.connect/publish/..., MqttBroker
    // .start/stop use `#[uniffi::export(async_runtime = "tokio")]`).
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
}

publishing {
    repositories {
        maven {
            name = "GitHubPackages"
            url = uri("https://maven.pkg.github.com/sorinirimies/stem-mqtt")
            credentials {
                username = System.getenv("GITHUB_ACTOR") ?: "sorinirimies"
                password = System.getenv("GITHUB_TOKEN")
            }
        }
    }
    publications {
        register<MavenPublication>("release") {
            groupId = "com.github.sorinirimies.stemmqtt"
            artifactId = "stem-mqtt-client-android"
            afterEvaluate {
                from(components["release"])
            }
            pom {
                name.set("stem-mqtt Kotlin/Android bindings")
                description.set(
                    "UniFFI-generated Kotlin bindings for the stem-mqtt client, " +
                        "packaged as an Android AAR (jniLibs for arm64-v8a/armeabi-v7a/x86_64/x86)."
                )
                url.set("https://github.com/sorinirimies/stem-mqtt")
                licenses {
                    license {
                        name.set("MIT")
                    }
                }
            }
        }
    }
}
