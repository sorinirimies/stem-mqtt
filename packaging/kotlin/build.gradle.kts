plugins {
    kotlin("jvm") version "2.0.20"
    `maven-publish`
    signing
}

// GitHub Packages / Gitea use `com.github.sorinirimies.stemmqtt`; Maven Central only accepts a
// verified namespace, which for a GitHub account is `io.github.<user>` — pass
// `-PgroupId=io.github.sorinirimies.stemmqtt` (see scripts/publish_maven_central.nu).
group = (findProperty("groupId") as String?) ?: "com.github.sorinirimies.stemmqtt"
version = (System.getenv("PACKAGE_VERSION") ?: "0.0.0-dev")

repositories {
    mavenCentral()
}

dependencies {
    implementation("net.java.dev.jna:jna:5.14.0")
    // Required by UniFFI's generated suspend-fn bridging for our
    // async-exported methods (MqttClient.connect/publish/..., MqttBroker
    // .start/stop use `#[uniffi::export(async_runtime = "tokio")]`).
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    testImplementation(kotlin("test-junit5"))
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

sourceSets {
    main {
        kotlin.srcDir("staged/kotlin")
        resources.srcDir("staged/resources")
    }
}

java {
    withSourcesJar()
    // Maven Central requires a javadoc jar next to the sources jar.
    withJavadocJar()
}

tasks.test {
    useJUnitPlatform()
}

publishing {
    repositories {
        // Local staging repository: Maven Central is fed a zip of this directory
        // (scripts/publish_maven_central.nu), not published to directly.
        maven {
            name = "staging"
            url = uri(layout.buildDirectory.dir("staging-repo"))
        }
        maven {
            // GitHub Packages by default; `-PmavenUrl=<base>/api/packages/<owner>/maven`
            // points the same publication at a Gitea instance's Maven registry.
            name = "Packages"
            url = uri((findProperty("mavenUrl") as String?) ?: "https://maven.pkg.github.com/sorinirimies/stem-mqtt")
            credentials {
                username = System.getenv("PACKAGES_USER") ?: System.getenv("GITHUB_ACTOR") ?: "sorinirimies"
                password = System.getenv("PACKAGES_TOKEN") ?: System.getenv("GITHUB_TOKEN")
            }
        }
    }
    publications {
        create<MavenPublication>("stemMqtt") {
            from(components["java"])
            artifactId = "stem-mqtt-kotlin"
            pom {
                name.set("stem-mqtt Kotlin bindings")
                description.set(
                    "UniFFI-generated Kotlin bindings for the stem-mqtt client and broker (native libs bundled)."
                )
                url.set("https://github.com/sorinirimies/stem-mqtt")
                licenses {
                    license {
                        name.set("MIT")
                        url.set("https://opensource.org/licenses/MIT")
                    }
                }
                developers {
                    developer {
                        id.set("sorinirimies")
                        name.set("Sorin Irimies")
                    }
                }
                scm {
                    url.set("https://github.com/sorinirimies/stem-mqtt")
                    connection.set("scm:git:https://github.com/sorinirimies/stem-mqtt.git")
                    developerConnection.set("scm:git:ssh://git@github.com/sorinirimies/stem-mqtt.git")
                }
            }
        }
    }
}

// Maven Central requires every artifact to be PGP-signed. Signing only happens when a key is
// provided (SIGNING_KEY = ASCII-armoured private key, SIGNING_PASSWORD), so local builds and the
// GitHub/Gitea registries are unaffected.
System.getenv("SIGNING_KEY")?.let { key ->
    signing {
        useInMemoryPgpKeys(key, System.getenv("SIGNING_PASSWORD"))
        sign(publishing.publications["stemMqtt"])
    }
}
