plugins {
    kotlin("jvm") version "2.0.20"
    `maven-publish`
}

group = "com.github.sorinirimies.stemmqtt"
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
}

tasks.test {
    useJUnitPlatform()
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
                    }
                }
            }
        }
    }
}
