#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — foreign-language binding specification (single source of truth)
# ──────────────────────────────────────────────────────────────────────────────
# Every language we generate bindings for, which generator produces them, the
# exact generator version we pin, which `uniffi` version that generator
# requires, and where the result can be published.
#
# Why `uniffi` versions matter: a UniFFI bindgen can only read the metadata
# embedded by the *same* UniFFI release (the "contract version" is checked at
# load time). Third-party generators each target one release, so a library
# built against UniFFI 0.31 can't be fed to a generator built for 0.32. For
# generators that disagree with the workspace we build the cdylib in a scratch
# copy of the workspace pinned to the version the generator wants — see
# `generate_bindings.nu`.
#
# Used by: scripts/generate_bindings.nu, scripts/install_bindgens.nu,
#          scripts/publish_packages.nu, scripts/tests/test_bindgens.nu
# ──────────────────────────────────────────────────────────────────────────────

# Packages we ship, keyed by logical crate name.
export const CRATES = {
    mqtt-client: { package: "stem-mqtt-client", lib: "mqtt_client" }
    mqtt-broker: { package: "stem-mqtt-broker", lib: "mqtt_broker" }
}

# `uniffi` release the workspace itself is built against (read from Cargo.toml
# at runtime by `workspace-uniffi`; this is only the documented default).
export def workspace-uniffi []: nothing -> string {
    open Cargo.toml
    | get workspace.dependencies.uniffi.version
}

# One record per supported language.
#
#   kind       builtin  → UniFFI's own generator, run via this repo's
#                         `uniffi-bindgen` binary (always matches the workspace)
#              external → separately installed third-party generator
#   uniffi     "workspace" or an exact UniFFI version the generator needs
#   tool       executable name (external only)
#   install    cargo-install arguments (external only)
#   registry   where `publish_packages.nu` can put it on GitHub Packages:
#                maven | npm | nuget | oci
#              (GitHub Packages has no native Go / Dart / Haskell / PyPI /
#               Swift / crates registry, so those travel as OCI artifacts on
#               ghcr.io — itself part of GitHub Packages.)
#   patch      repo-relative patch applied to the generator's source before installing
#              (empty = install straight from the pinned tag/rev)
#   crate_path crate inside the patched checkout to `cargo install --path`
#   maturity   stable | experimental
export def specs []: nothing -> table {
    (_raw-specs) | each { |row| { patch: "", crate_path: "" } | merge $row }
}

def _raw-specs []: nothing -> list<record> {
    [
        { language: kotlin,  kind: builtin,  uniffi: workspace, tool: "", install: [], registry: maven, maturity: stable }
        { language: swift,   kind: builtin,  uniffi: workspace, tool: "", install: [], registry: oci,   maturity: stable }
        { language: python,  kind: builtin,  uniffi: workspace, tool: "", install: [], registry: oci,   maturity: stable }
        {
            language: go, kind: external, uniffi: workspace, tool: "uniffi-bindgen-go"
            install: [uniffi-bindgen-go --git "https://github.com/NordSecurity/uniffi-bindgen-go" --tag "v0.7.1+v0.31.0"]
            registry: oci, maturity: stable
        }
        {
            language: csharp, kind: external, uniffi: workspace, tool: "uniffi-bindgen-cs"
            install: [uniffi-bindgen-cs --git "https://github.com/NordSecurity/uniffi-bindgen-cs" --tag "v0.11.0+v0.31.0"]
            registry: nuget, maturity: stable
        }
        {
            language: java, kind: external, uniffi: workspace, tool: "uniffi-bindgen-java"
            install: [uniffi-bindgen-java --version "0.4.2"]
            registry: maven, maturity: stable
        }
        {
            language: dart, kind: external, uniffi: workspace, tool: "uniffi_bindgen_dart"
            # The v0.2.1 tag ships a placeholder `uniffi_bindgen_dart`; the real CLI landed on main, so pin a main commit.
            install: [uniffi-dart --git "https://github.com/acterglobal/uniffi-dart" --rev "e2dd2bb7180609184d0c70ddde0353f5e11d4c3b" --features binary --bin uniffi_bindgen_dart]
            registry: oci, maturity: experimental
        }
        {
            language: node, kind: external, uniffi: workspace, tool: "uniffi-bindgen-node-js"
            install: [uniffi-bindgen-node-js --version "0.0.16"]
            registry: npm, maturity: experimental
        }
        {
            language: haskell, kind: external, uniffi: "0.32.0", tool: "uniffi-bindgen-haskell"
            install: [uniffi-bindgen-haskell --git "https://github.com/mercury/uniffi-bindgen-haskell" --rev "836df3601b0bf4a7970457bb53d295c538cf5fb9"]
            registry: oci, maturity: experimental
            # Upstream's constructors never lower their arguments and flat errors drop their
            # message string; both fixed by this patch (see packaging/haskell/).
            patch: "packaging/haskell/uniffi-bindgen-haskell.patch", crate_path: "crates/uniffi-bindgen-haskell"
        }
    ]
}

# Accept friendly aliases (`cs`, `c#`, `js`, …) and return the canonical name.
export def canonical-language [name: string]: nothing -> string {
    match ($name | str downcase) {
        "cs" | "c#" | "dotnet" => "csharp"
        "js" | "node-js" | "nodejs" => "node"
        "hs" => "haskell"
        "kt" => "kotlin"
        "py" => "python"
        $other => $other
    }
}

# Look up one language's spec, or raise a catchable error listing the options.
export def spec-for [language: string]: nothing -> record {
    let wanted = (canonical-language $language)
    let found = (specs | where language == $wanted)
    if ($found | is-empty) {
        error make {
            msg: $"unsupported language '($language)'. Supported: (specs | get language | str join ', ')"
        }
    }
    $found | first
}

# Resolve a logical crate name (`mqtt-client` / `mqtt-broker`) to its record.
export def crate-for [crate: string]: nothing -> record<package: string, lib: string> {
    if $crate not-in ($CRATES | columns) {
        error make { msg: $"unknown crate '($crate)'. Expected one of: ($CRATES | columns | str join ', ')" }
    }
    $CRATES | get $crate
}

# Where `generate_bindings.nu` writes a language/crate pair by default.
export def default-out-dir [language: string, crate: string]: nothing -> string {
    $"bindings/(canonical-language $language)/($crate)"
}

# Rewrite a workspace Cargo.toml so it contains only the two library crates
# and pins `uniffi` to exactly `version`. Pure text transform → unit-testable.
export def pin-workspace-toml [toml: string, version: string]: nothing -> string {
    $toml
    | str replace --regex '(?s)members\s*=\s*\[.*?\]' 'members = ["crates/mqtt-client", "crates/mqtt-broker"]'
    | str replace --regex '(?m)^(uniffi\s*=\s*\{\s*version\s*=\s*")[^"]*"' ('${1}=' + $version + '"')
}

# Native library file name for a UniFFI crate on the current OS.
export def native-lib-name [lib: string]: nothing -> string {
    match $nu.os-info.name {
        "macos" => $"lib($lib).dylib"
        "windows" => $"($lib).dll"
        _ => $"lib($lib).so"
    }
}

# Command line (after the tool name) that makes `tool` generate bindings for
# the compiled library `lib` into `out`. `lib_name` is the UniFFI crate name
# (`mqtt_client`), needed by generators that must be told which component of a
# multi-crate library to emit; `repo` is the repository root (for config files).
# Pure → unit-testable.
export def generator-args [language: string, lib: string, out: string, lib_name: string, repo: string = "."]: nothing -> list<string> {
    match (canonical-language $language) {
        "go" => [$lib "--out-dir" $out]
        "csharp" => ["--library" $lib "--config" ($repo | path join "packaging" "csharp" "uniffi.toml") "--out-dir" $out]
        "java" => ["generate" $lib "--out-dir" $out]
        "dart" => ["--library" $lib "--out-dir" $out]
        "node" => ["generate" $lib "--crate-name" $lib_name "--out-dir" $out]
        "haskell" => [
            "--library" $lib "--out-dir" $out
            # A ready-to-build Cabal package; consumers add `extra-lib-dirs` for the static library.
            "--cabal-file" "stem-mqtt-bindings.cabal"
            "--cabal-header" ($repo | path join "packaging" "haskell" "cabal-header.txt")
            "--cabal-native-library" $lib_name
        ]
        $other => { error make { msg: $"no external generator arguments defined for '($other)'" } }
    }
}

# ── GitHub Packages publishing helpers (pure, unit-tested) ───────────────────

# Identifies the host's native-library flavour, e.g. `macos-aarch64`.
export def platform-id []: nothing -> string {
    $"($nu.os-info.name)-($nu.os-info.arch)"
}

# .NET runtime identifier for a platform id (NuGet `runtimes/<rid>/native/`).
export def dotnet-rid [platform: string]: nothing -> string {
    match $platform {
        "macos-aarch64" => "osx-arm64"
        "macos-x86_64" => "osx-x64"
        "linux-x86_64" => "linux-x64"
        "linux-aarch64" => "linux-arm64"
        "windows-x86_64" => "win-x64"
        $other => { error make { msg: $"no .NET runtime identifier for platform '($other)'" } }
    }
}

# Package name used on the registry for a language, e.g. `stem-mqtt-go`.
# Client and broker ship together in one package per language (like the
# existing Kotlin package), so the name carries no crate suffix.
export def package-name [language: string]: nothing -> string {
    $"stem-mqtt-(canonical-language $language)"
}

# Where a package registry lives for a target host.
#   github → GitHub Packages (maven/npm/nuget/ghcr.io under github.com)
#   gitea  → a Gitea instance's built-in package registries (`base_url` required,
#            e.g. http://192.168.1.44:3000): maven, npm, nuget and container
#            (OCI) registries are all served from `<base>/api/packages/<owner>/…`.
#   public → the public package indexes people actually install from: npmjs.org,
#            NuGet.org, Maven Central, Hackage, pub.dev, Go modules (see `publish-plan`).
export def registry-endpoints [target: string, owner: string, base_url: string, repo: string]: nothing -> record {
    match $target {
        "public" => {
            npm: "https://registry.npmjs.org/"
            nuget: "https://api.nuget.org/v3/index.json"
            maven: "https://central.sonatype.com"
            oci_host: ""
            plain_http: false
            source_url: $"https://github.com/($owner)/($repo)"
        }
        "github" => {
            npm: "https://npm.pkg.github.com"
            nuget: $"https://nuget.pkg.github.com/($owner)/index.json"
            maven: $"https://maven.pkg.github.com/($owner)/($repo)"
            oci_host: "ghcr.io"
            plain_http: false
            source_url: $"https://github.com/($owner)/($repo)"
        }
        "gitea" => {
            if $base_url == "" { error make { msg: "--base-url is required for --target gitea (e.g. http://192.168.1.44:3000)" } }
            let base = ($base_url | str trim --right --char "/")
            {
                npm: $"($base)/api/packages/($owner)/npm/"
                nuget: $"($base)/api/packages/($owner)/nuget/index.json"
                maven: $"($base)/api/packages/($owner)/maven"
                oci_host: ($base | str replace --regex '^https?://' "")
                plain_http: ($base | str starts-with "http://")
                source_url: $"($base)/($owner)/($repo)"
            }
        }
        $other => { error make { msg: $"unknown publish target '($other)': expected github, gitea or public" } }
    }
}

# The publish plan for one staged language: a list of
# `{ cwd, cmd, args, note }` steps. Pure so `--dry-run` and tests can inspect
# exactly what would run. `stage` is the staged directory.
export def publish-plan [
    language: string
    version: string
    stage: string
    owner: string
    repo: string = "stem-mqtt"
    target: string = "github"
    base_url: string = ""
]: nothing -> list<record> {
    let spec = (spec-for $language)
    let name = (package-name $spec.language)
    let ep = (registry-endpoints $target $owner $base_url $repo)
    if $target == "public" { return (public-plan $spec $version $stage $owner $ep) }
    match $spec.registry {
        "npm" => [
            { cwd: $stage, cmd: "npm", note: $"publish to ($ep.npm)"
              args: ([publish "--registry" $ep.npm] | append (if $target == "github" { ["--access" "restricted"] } else { [] })) }
        ]
        "nuget" => [
            { cwd: $stage, cmd: "dotnet", note: "pack the NuGet package"
              args: [pack "StemMqtt.csproj" "-c" Release $"-p:Version=($version)" $"-p:PackageId=StemMqtt" "-o" "nupkg"] }
            { cwd: $stage, cmd: "dotnet", note: $"push to ($ep.nuget)"
              args: [nuget push "nupkg/*.nupkg" "--source" $ep.nuget "--skip-duplicate"] }
        ]
        "maven" => [
            { cwd: $stage, cmd: "gradle", note: $"publish to ($ep.maven)"
              args: [publish $"-Pversion=($version)" $"-PmavenUrl=($ep.maven)"] }
        ]
        "oci" => [
            { cwd: $stage, cmd: "oras", note: $"push bundle to ($ep.oci_host) \(OCI container registry\)"
              args: ([push $"($ep.oci_host)/($owner)/($name):($version)"
                     $"($name)-($version).tar.gz:application/vnd.stem-mqtt.bindings.v1.tar+gzip"
                     "--annotation" $"org.opencontainers.image.source=($ep.source_url)"
                     "--annotation" $"org.opencontainers.image.version=($version)"
                     "--annotation" $"org.opencontainers.image.description=stem-mqtt ($spec.language) bindings \(client + broker\)"]
                     | append (if $ep.plain_http { ["--plain-http"] } else { [] })) }
        ]
        $other => { error make { msg: $"unknown registry '($other)' for ($language)" } }
    }
}

# The Maven Central group: a verified `io.github.<user>` namespace (the GitHub registries use
# `com.github.…`, which Central does not accept because it can't verify that domain).
export def central-group [owner: string]: nothing -> string {
    $"io.github.($owner).stemmqtt"
}

# Plans for the public package indexes. Credentials are injected by `publish_packages.nu`
# at run time (NPM / NUGET_API_KEY / HACKAGE_TOKEN / MAVEN_CENTRAL_*), never part of the plan.
def public-plan [spec: record, version: string, stage: string, owner: string, ep: record]: nothing -> list<record> {
    let name = (package-name $spec.language)
    match $spec.language {
        "node" => [
            { cwd: $stage, cmd: "npm", note: $"publish to ($ep.npm) \(npmjs.org)"
              args: [publish "--registry" $ep.npm "--access" "public"] }
        ]
        "csharp" => [
            { cwd: $stage, cmd: "dotnet", note: "pack the NuGet package"
              args: [pack "StemMqtt.csproj" "-c" Release $"-p:Version=($version)" $"-p:PackageId=StemMqtt" "-o" "nupkg"] }
            { cwd: $stage, cmd: "dotnet", note: "push to NuGet.org"
              args: [nuget push "nupkg/*.nupkg" "--source" $ep.nuget "--skip-duplicate"] }
        ]
        "java" => [
            { cwd: $stage, cmd: "gradle", note: "build + sign the artifacts into a staging repository"
              args: [publishAllPublicationsToStagingRepository $"-Pversion=($version)" $"-PgroupId=(central-group $owner)"] }
            { cwd: $stage, cmd: "nu", note: "upload the bundle to Maven Central (Central Portal)"
              args: [($env.PWD | path join "scripts" "publish_maven_central.nu") "build/staging-repo" $"($name)-($version)"] }
        ]
        "haskell" => [
            { cwd: ($stage | path join "hackage"), cmd: "cabal", note: "build the source distribution"
              args: [sdist] }
            { cwd: ($stage | path join "hackage"), cmd: "cabal", note: "upload to Hackage"
              args: [upload "--publish" $"dist-newstyle/sdist/stem-mqtt-($version).tar.gz"] }
        ]
        "dart" => [
            { cwd: ($stage | path join "pub"), cmd: "dart", note: "publish to pub.dev (needs pub.dev automated publishing: run from GitHub Actions)"
              args: [pub publish "--force"] }
        ]
        "go" => [
            { cwd: $env.PWD, cmd: "nu", note: "push the Go module to its repository and tag it"
              args: [($env.PWD | path join "scripts" "publish_go_module.nu") $stage $version] }
        ]
        $other => { error make { msg: $"no public registry for ($other): it ships as a release asset" } }
    }
}

# ── Per-language capabilities ────────────────────────────────────────────────

# Whether foreign code can implement Rust callback interfaces (message listener,
# auth provider, event listener, enhanced-auth callbacks). Where it can't, the
# pull-style API (`enable_message_queue`/`next_message`, `enable_event_queue`/
# `next_event`) is the way to receive messages and events.
export def supports-callbacks [language: string]: nothing -> bool {
    match (canonical-language $language) {
        # uniffi-dart aborts the VM when Rust calls a callback from its own thread.
        "dart" => false
        # Generated Haskell exposes callback interfaces only as opaque handles.
        "haskell" => false
        _ => true
    }
}

# The *build* half of a package — everything `publish-plan` does except the
# upload — so CI can prove each language's package compiles/packs on every
# push without credentials. Pure → unit-tested.
export def build-plan [language: string, version: string, stage: string]: nothing -> list<record> {
    let spec = (spec-for $language)
    let name = (package-name $spec.language)
    match $spec.registry {
        "npm" => [{ cwd: $stage, cmd: "npm", note: "pack the npm package", args: [pack "--dry-run"] }]
        "nuget" => [{ cwd: $stage, cmd: "dotnet", note: "compile + pack the NuGet package"
                      args: [pack "StemMqtt.csproj" "-c" Release $"-p:Version=($version)" "-o" "nupkg"] }]
        "maven" => {
            # Run the smoke test against the *packaged jar alone* (no java.library.path): that is
            # what a consumer does, and it proves the bundled native library is found.
            let jar = $"build/libs/stem-mqtt-java-($version).jar"
            let sep = if $nu.os-info.name == "windows" { ";" } else { ":" }
            [
                { cwd: $stage, cmd: "gradle", note: "compile + assemble the jar", args: [build $"-Pversion=($version)"] }
                { cwd: $stage, cmd: "javac", note: "compile the smoke test against the jar"
                  args: ["--release" "22" "-cp" $jar "-d" "build/smoke" ($env.PWD | path join "tests" "bindings" "java" "Smoke.java")] }
                { cwd: $stage, cmd: "java", note: "run the smoke test with only the packaged jar on the classpath"
                  args: ["--enable-native-access=ALL-UNNAMED" "-cp" $"($jar)($sep)build/smoke" "Smoke"] }
            ]
        }
        "oci" => [{ cwd: $stage, cmd: "tar", note: "assemble the OCI bundle"
                    args: ["-czf" $"($name)-($version).tar.gz" sources native] }]
        $other => { error make { msg: $"unknown registry '($other)' for ($language)" } }
    }
}
