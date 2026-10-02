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
#   go, dart, haskell, node-livekit, swift, python → OCI (ghcr.io)
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
#       Run the registry's publish commands. `--dry-run` prints the plan only.
#
# Credentials come from the environment (GITHUB_TOKEN / NODE_AUTH_TOKEN /
# GITHUB_ACTOR / ORG_GRADLE_PROJECT_*), never from arguments.
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *

def default-stage [language: string]: nothing -> string {
    $"dist/(canonical-language $language)"
}

# Generate both crates and merge them into <stage>/sources (the broker's
# output repeats the client's generated files verbatim; identical names simply
# overwrite each other).
def stage-sources [language: string, stage: string] {
    let sources = ($stage | path join "sources")
    rm -rf $sources
    mkdir $sources
    let blocked = (unsupported-crates $language | columns)
    for crate in ($CRATES | columns | where { |c| $c not-in $blocked }) {
        let dir = (default-out-dir $language $crate)
        rm -rf $dir
        nu scripts/generate_bindings.nu $language $crate $dir
        cp -r ...(glob ($dir | path join "*")) $sources
    }
}

# Copy this host's native libraries into <stage>/native/<platform>/.
def stage-native [language: string, stage: string] {
    let spec = (spec-for $language)
    let root = if $spec.uniffi == "workspace" { $env.PWD } else { $env.PWD | path join "target" $"uniffi-($spec.uniffi)" }
    let dest = ($stage | path join "native" (platform-id))
    mkdir $dest
    let blocked = (unsupported-crates $language | columns)
    for crate in ($CRATES | columns | where { |c| $c not-in $blocked }) {
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
            # The generator already emits a package.json; retarget it at GitHub Packages.
            # npm packages are rooted at the stage dir, so hoist the sources.
            cp -r ...(glob ($stage | path join "sources" "*")) $stage
            rm -rf ($stage | path join "sources")
            let pkg_path = ($stage | path join "package.json")
            open $pkg_path
            | upsert name "@OWNER/stem-mqtt-node"
            | upsert publishConfig { registry: "https://npm.pkg.github.com" }
            | upsert files ["*.js" "*.d.ts" "runtime" "native"]
            | save --force $pkg_path
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
            r#'plugins { `java-library`; `maven-publish` }
// The generated bindings use the Foreign Function & Memory API (JDK 22+): build
// with any JDK >= 22 targeting release 22 (no toolchain download needed).
tasks.withType<JavaCompile> { options.release.set(22) }
sourceSets { main { java.srcDir("sources"); resources.srcDir("native-resources") } }
group = "com.github.sorinirimies.stemmqtt"
publishing {
    publications { create<MavenPublication>("java") { artifactId = "stem-mqtt-java"; from(components["java"]) } }
    repositories { maven {
        url = uri("https://maven.pkg.github.com/${project.property("ghOwner")}/${project.property("ghRepo")}")
        credentials { username = System.getenv("GITHUB_ACTOR"); password = System.getenv("GITHUB_TOKEN") }
    } }
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
    if $spec.registry == "maven" {
        let res = ($stage | path join "native-resources")
        rm -rf $res
        cp -r ($stage | path join "native") $res
    }
    if $spec.registry == "oci" {
        ^tar -czf ($stage | path join $"($name)-($version).tar.gz") -C $stage sources native
    }

}

# Generate bindings + native libs for one language into a stage directory.
def "main stage" [language: string, --out: string] {
    let language = (canonical-language $language)
    if $language == "kotlin" {
        error make { msg: "Kotlin is published by packaging/kotlin (Gradle) — see packaging/kotlin/README.md" }
    }
    let broken = (known-broken $language)
    if $broken != "" {
        error make { msg: $"refusing to stage ($language): ($broken)" }
    }
    let stage = ($out | default (default-stage $language))
    mkdir $stage
    stage-sources $language $stage
    stage-native $language $stage
    write-descriptor $language $stage
    print $"staged ($language) -> ($stage) \(platform: (platform-id)\)"
}

# Publish a staged language to GitHub Packages.
def "main publish" [
    language: string
    version: string
    --from: string
    --owner: string = "sorinirimies"
    --repo: string = "stem-mqtt"
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

    let plan = (publish-plan $language $version $stage $owner $repo)
    for step in $plan {
        print $"==> ($step.note): ($step.cmd) ($step.args | str join ' ')"
        if not $dry_run {
            # NuGet wants the token as an argument; read it from the environment
            # here (not in the pure plan) so it never lands in a printed plan.
            let args = if $step.cmd == "dotnet" and ("push" in $step.args) {
                $step.args | append ["--api-key" ($env.GITHUB_TOKEN? | default "")]
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
    let broken = (known-broken $language)
    if $broken != "" { error make { msg: $"refusing to verify ($language): ($broken)" } }
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
