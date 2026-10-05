#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — stage and publish language bindings to GitHub Packages
# ──────────────────────────────────────────────────────────────────────────────
# GitHub Packages natively hosts Maven, npm, NuGet, RubyGems and containers
# (ghcr.io) — there is no Go, Dart, Haskell, PyPI, Swift or crates registry.
# So every language goes where it natively fits, and the rest travel as OCI
# artifacts on ghcr.io (also GitHub Packages), pulled with `oras pull`:
#
#   java            → Maven   (maven.pkg.github.com)
#   csharp          → NuGet   (nuget.pkg.github.com)
#   node            → npm     (npm.pkg.github.com)
#   go, dart, haskell, swift, python → OCI (ghcr.io)
#   kotlin          → published by the existing Gradle flow (packaging/kotlin)
#
# Two steps, so CI can build native libraries on every OS and publish once:
#
#   nu scripts/publish_packages.nu stage <language> [--out dist/<language>]
#       Generate client+broker bindings, merge them, add this host's native
#       libraries under native/<platform>/, and write the registry's package
#       descriptor. Run on each OS; merge the resulting `native/` trees.
#
#   nu scripts/publish_packages.nu publish <language> <version> [--from DIR]
#                                          [--owner O] [--repo R] [--dry-run]
#                                          [--target github|gitea --base-url URL]
#       Run the registry's publish commands. `--dry-run` prints the plan only.
#
# `--target gitea --base-url http://host:3000` publishes to a Gitea instance's own
# package registries (Maven, npm, NuGet, container) instead of GitHub Packages.
#
# Credentials come from the environment (PACKAGES_TOKEN / PACKAGES_USER, falling back
# to GITHUB_TOKEN / GITHUB_ACTOR; NODE_AUTH_TOKEN for npm), never from arguments.
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *
use bindings/source.nu *
use bindings/hackage.nu *
use bindings/dart_package.nu *

def default-stage [language: string]: nothing -> string {
    $"dist/(canonical-language $language)"
}

# Generate both crates into <stage>/sources.
#
# Most generators emit the broker's output as a superset of the client's (the
# broker library embeds the client component), so the two trees are merged and
# identical names simply overwrite each other. Node is the exception: each
# component is its own self-contained npm package (`index.js`, `runtime/`,
# `package.json`, loading its own native library), so they stay side by side as
# `sources/client` and `sources/broker`.
def stage-sources [language: string, stage: string] {
    let sources = ($stage | path join "sources")
    rm -rf $sources
    mkdir $sources
    let per_component = ($language == "node")
    for crate in ($CRATES | columns) {
        let dir = (default-out-dir $language $crate)
        rm -rf $dir
        nu scripts/generate_bindings.nu $language $crate $dir
        if $per_component {
            let dest = ($sources | path join ($crate | str replace "mqtt-" ""))
            mkdir $dest
            cp -r ...(glob ($dir | path join "*")) $dest
        } else {
            cp -r ...(glob ($dir | path join "*")) $sources
        }
    }
}

# Copy this host's native libraries into <stage>/native/<platform>/.
def stage-native [language: string, stage: string] {
    let spec = (spec-for $language)
    let root = if $spec.uniffi == "workspace" { $env.PWD } else { $env.PWD | path join "target" $"uniffi-($spec.uniffi)" }
    let dest = ($stage | path join "native" (platform-id))
    mkdir $dest
    for crate in ($CRATES | columns) {
        let lib = ($CRATES | get $crate).lib
        cp ($root | path join "target" "release" (native-lib-name $lib)) $dest
        # Haskell links the static library (the cdylib is only read for metadata).
        if $language == "haskell" {
            cp ($root | path join "target" "release" $"lib($lib).a") $dest
        }
    }
}

def write-descriptor [language: string, stage: string] {
    let spec = (spec-for $language)
    match $spec.registry {
        "npm" => {
            # One package, two entry points: `<pkg>/client` and `<pkg>/broker`, each
            # a generated ESM package sitting next to its native library.
            let koffi = (open ($stage | path join "sources" "client" "package.json") | get dependencies.koffi)
            {
                name: "@OWNER/stem-mqtt-node"
                version: "0.0.0"
                description: "MQTT 3.1.1 / 5.0 client and broker (Rust core, UniFFI Node bindings)"
                type: "module"
                engines: { node: ">=16" }
                dependencies: { koffi: $koffi }
                exports: { "./client": "./client/index.js", "./broker": "./broker/index.js" }
                files: ["client" "broker"]
                publishConfig: { registry: "https://npm.pkg.github.com" }
            } | save --force ($stage | path join "package.json")
            for part in [client broker] {
                cp -r ($stage | path join "sources" $part) ($stage | path join $part)
            }
            rm -rf ($stage | path join "sources")
        }
        "nuget" => {
            r#'<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <PackageId>StemMqtt</PackageId>
    <Description>MQTT 3.1.1 / 5.0 client and broker (Rust core, UniFFI C# bindings)</Description>
    <RepositoryUrl>https://github.com/OWNER/stem-mqtt</RepositoryUrl>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="sources/**/*.cs" />
    <!-- native/<platform>/ is re-laid out as runtimes/<rid>/native/ at publish time -->
    <None Include="runtimes/**" Pack="true" PackagePath="runtimes" />
  </ItemGroup>
</Project>
'# | save --force ($stage | path join "StemMqtt.csproj")
        }
        "maven" => {
            r#'plugins { `java-library`; `maven-publish`; signing }
// The generated bindings use the Foreign Function & Memory API (JDK 22+): build
// with any JDK >= 22 targeting release 22 (no toolchain download needed).
tasks.withType<JavaCompile> { options.release.set(22) }
java { withSourcesJar(); withJavadocJar() }   // Maven Central requires both
// Generated doc comments (copied from the Rust docs) aren't valid HTML; don't let doclint fail the jar.
tasks.withType<Javadoc> { (options as StandardJavadocDocletOptions).addStringOption("Xdoclint:none", "-quiet") }
sourceSets { main { java.srcDir("sources"); resources.srcDir("native-resources") } }
// GitHub Packages / Gitea: com.github.sorinirimies.stemmqtt. Maven Central needs a verified
// namespace (io.github.<user>.…), passed as -PgroupId by scripts/publish_maven_central.nu.
group = (findProperty("groupId") as String?) ?: "com.github.sorinirimies.stemmqtt"
publishing {
    publications {
        create<MavenPublication>("java") {
            artifactId = "stem-mqtt-java"
            from(components["java"])
            pom {
                name.set("stem-mqtt Java bindings")
                description.set("UniFFI-generated Java (FFM) bindings for the stem-mqtt MQTT client and broker; native libs bundled.")
                url.set("https://github.com/OWNER/stem-mqtt")
                licenses { license { name.set("MIT"); url.set("https://opensource.org/licenses/MIT") } }
                developers { developer { id.set("OWNER"); name.set("Sorin Irimies") } }
                scm {
                    url.set("https://github.com/OWNER/stem-mqtt")
                    connection.set("scm:git:https://github.com/OWNER/stem-mqtt.git")
                }
            }
        }
    }
    repositories {
        // Zipped and uploaded to Maven Central by scripts/publish_maven_central.nu.
        maven { name = "staging"; url = uri(layout.buildDirectory.dir("staging-repo")) }
        maven {
            // -PmavenUrl is passed by `publish_packages.nu publish` (GitHub Packages or a Gitea
            // instance); a plain `gradle build` never touches it.
            url = uri((project.findProperty("mavenUrl") as String?) ?: "https://invalid.example/unset")
            credentials {
                username = System.getenv("PACKAGES_USER") ?: System.getenv("GITHUB_ACTOR")
                password = System.getenv("PACKAGES_TOKEN") ?: System.getenv("GITHUB_TOKEN")
            }
        }
    }
}
// Signing only when a key is provided (Maven Central); registries and local builds are unaffected.
System.getenv("SIGNING_KEY")?.let { key ->
    signing {
        useInMemoryPgpKeys(key, System.getenv("SIGNING_PASSWORD"))
        sign(publishing.publications["java"])
    }
}
'# | save --force ($stage | path join "build.gradle.kts")
            "rootProject.name = \"stem-mqtt-java\"\n" | save --force ($stage | path join "settings.gradle.kts")
        }
        _ => {}
    }
}

# Registry-specific preparation that depends on the final version/owner:
# retarget descriptors, lay native libraries out the way the registry expects.
def prepare-package [language: string, version: string, stage: string, owner: string] {
    let spec = (spec-for $language)
    let name = (package-name $language)
    if $spec.registry == "npm" {
        open ($stage | path join "package.json")
        | upsert name $"@($owner)/($name)" | upsert version $version
        | save --force ($stage | path join "package.json")
        # Each generated package loads `lib<crate>.<ext>` from its own directory.
        for platform in (ls ($stage | path join "native") | get name) {
            for pair in [{ part: "client", lib: "mqtt_client" } { part: "broker", lib: "mqtt_broker" }] {
                cp ...(glob ($platform | path join $"*($pair.lib)*")) ($stage | path join $pair.part)
            }
        }
    }
    if $spec.registry == "nuget" {
        let proj = ($stage | path join "StemMqtt.csproj")
        open --raw $proj | str replace --all "OWNER" $owner | save --force $proj
        for platform in (ls ($stage | path join "native") | get name | path basename) {
            let rid = (dotnet-rid $platform)
            let dest = ($stage | path join "runtimes" $rid "native")
            mkdir $dest
            cp ...(glob ($stage | path join "native" $platform "*")) $dest
        }
    }
    # The source packages carry their version in their own manifests.
    if $language == "haskell" {
        let cabal = ($stage | path join "hackage" "stem-mqtt.cabal")
        open --raw $cabal | str replace --regex '(?m)^version:\s+.*$' $"version:            ($version)" | save --force $cabal
    }
    if $language == "dart" {
        let pubspec = ($stage | path join "pub" "pubspec.yaml")
        open --raw $pubspec | str replace --regex '(?m)^version:.*$' $"version: ($version)" | save --force $pubspec
        let changelog = ($stage | path join "pub" "CHANGELOG.md")
        open --raw $changelog | str replace --regex '(?m)^## .*$' $"## ($version)" | save --force $changelog
    }
    if $spec.registry == "maven" {
        let gradle = ($stage | path join "build.gradle.kts")
        open --raw $gradle | str replace --all "OWNER" $owner | save --force $gradle
        let res = ($stage | path join "native-resources")
        rm -rf $res
        cp -r ($stage | path join "native") $res
    }
    if $spec.registry == "oci" {
        ^tar -czf ($stage | path join $"($name)-($version).tar.gz") -C $stage sources native
    }

}

# Hackage and pub.dev take source packages that build their own Rust core at install time, so
# they are assembled from the generated bindings + the Rust sources instead of prebuilt libraries.
def stage-source-package [language: string, stage: string] {
    let spec = (spec-for $language)
    let rust_root = if $spec.uniffi == "workspace" { $env.PWD } else { $env.PWD | path join "target" $"uniffi-($spec.uniffi)" }
    let gen = ($stage | path join "sources")
    if $language == "haskell" {
        build-hackage-package $gen $rust_root (prepare-source $spec) ($stage | path join "hackage") "0.0.0"
    } else if $language == "dart" {
        build-dart-package $gen $rust_root ($stage | path join "pub") "0.0.0"
    }
}

# Generate bindings + native libs for one language into a stage directory.
def "main stage" [language: string, --out: string] {
    let language = (canonical-language $language)
    if $language == "kotlin" {
        error make { msg: "Kotlin is published by packaging/kotlin (Gradle) — see packaging/kotlin/README.md" }
    }
    let stage = ($out | default (default-stage $language))
    mkdir $stage
    stage-sources $language $stage
    stage-native $language $stage
    write-descriptor $language $stage
    stage-source-package $language $stage
    print $"staged ($language) -> ($stage) \(platform: (platform-id)\)"
}

# Publish a staged language to GitHub Packages.
def "main publish" [
    language: string
    version: string
    --from: string
    --owner: string = "sorinirimies"
    --repo: string = "stem-mqtt"
    --target: string = "github"   # github | gitea | public
    --base-url: string = ""       # Gitea instance, e.g. http://192.168.1.44:3000
    --dry-run
] {
    let language = (canonical-language $language)
    let stage = ($from | default (default-stage $language) | path expand)
    if not ($stage | path exists) {
        error make { msg: $"nothing staged at ($stage) — run: nu scripts/publish_packages.nu stage ($language)" }
    }
    let spec = (spec-for $language)
    let name = (package-name $language)

    prepare-package $language $version $stage $owner
    if $target == "public" {
        let ep = (registry-endpoints $target $owner $base_url $repo)
        if $spec.registry == "npm" {
            open ($stage | path join "package.json")
            | upsert publishConfig { registry: $ep.npm, access: "public" }
            | save --force ($stage | path join "package.json")
        }
    }

    let plan = (publish-plan $language $version $stage $owner $repo $target $base_url)
    for step in $plan {
        print $"==> ($step.note): ($step.cmd) ($step.args | str join ' ')"
        if not $dry_run {
            # NuGet wants the token as an argument; read it from the environment
            # here (not in the pure plan) so it never lands in a printed plan.
            let token = ($env.PACKAGES_TOKEN? | default ($env.GITHUB_TOKEN? | default ""))
            let args = if $step.cmd == "dotnet" and ("push" in $step.args) {
                let key = if $target == "public" { $env.NUGET_API_KEY? | default "" } else { $token }
                $step.args | append ["--api-key" $key]
            } else if $step.cmd == "cabal" and ("upload" in $step.args) {
                $step.args | append ["--token" ($env.HACKAGE_TOKEN? | default "")]
            } else { $step.args }
            cd $step.cwd
            run-external $step.cmd ...$args
        }
    }
}

# Prove a language's package builds (compiles / packs) without publishing it.
# Stages first if needed. Used by CI on every push.
def "main verify" [language: string, --version: string = "0.0.0-ci", --out: string] {
    let language = (canonical-language $language)
    let stage = ($out | default (default-stage $language) | path expand)
    if not ($stage | path exists) { main stage $language --out $stage }
    prepare-package $language $version $stage "local"
    for step in (build-plan $language $version $stage) {
        print $"==> ($step.note): ($step.cmd) ($step.args | str join ' ')"
        cd $step.cwd
        run-external $step.cmd ...$step.args
    }
    print $"($language): package builds"
}

def main [] {
    print "usage: nu scripts/publish_packages.nu <stage|publish> ..."
    print "  stage   <language> [--out DIR]"
    print "  verify  <language> [--version V]   # build/pack only, no upload"
    print "  publish <language> <version> [--from DIR] [--owner O] [--repo R] [--dry-run]"
}
