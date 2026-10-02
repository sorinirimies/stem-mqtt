#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — runtime tests for every generated language binding
# ──────────────────────────────────────────────────────────────────────────────
# Generating bindings proves nothing about whether they *work*. This builds the
# native libraries, generates the bindings, stages each language in a scratch
# project together with its smoke test (tests/bindings/<language>/), and runs
# it against real sockets: start a broker, subscribe, publish QoS 1, receive
# it through a foreign callback, refuse a bad client through a foreign auth
# callback, observe connections through a foreign event listener, stop.
# Languages whose generators can't do callbacks run a reduced scenario (see
# `capabilities` in scripts/bindings/spec.nu and the header of each test).
#
# Usage:
#   nu scripts/test_bindings.nu                 # every language
#   nu scripts/test_bindings.nu go java         # just these
#   nu scripts/test_bindings.nu --strict        # a missing toolchain is a failure, not a skip
#   nu scripts/test_bindings.nu --no-generate   # reuse bindings from the last run
#
# Result per language: PASS · FAIL · SKIP (toolchain missing) ·
#   XFAIL (known upstream breakage still broken) · XPASS (known-broken now passes → update spec.nu)
# Exit status is non-zero on any FAIL (or SKIP with --strict, or XPASS).
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *
use bindings/source.nu *

const TESTED = [python go csharp java dart node node-livekit haskell]

def repo []: nothing -> string { $env.FILE_PWD | path dirname }

def work-dir [language: string]: nothing -> string {
    repo | path join "target" "bindings-test" $language
}

# The language's smoke-test sources.
def smoke-src [language: string]: nothing -> string {
    repo | path join "tests" "bindings" $language
}

# Root of the workspace the language's library is built in (scratch for generators
# pinned to another UniFFI release).
def build-root [spec: record]: nothing -> string {
    if $spec.uniffi == "workspace" { repo } else { repo | path join "target" $"uniffi-($spec.uniffi)" }
}

def native-lib [spec: record, crate: string]: nothing -> string {
    build-root $spec | path join "target" "release" (native-lib-name (crate-for $crate).lib)
}

# Generate both crates for `language` under <work>/gen/<crate>; skip blocked ones.
def generate [language: string, work: string] {
    let blocked = (unsupported-crates $language | columns)
    for crate in ($CRATES | columns | where { |c| $c not-in $blocked }) {
        nu scripts/generate_bindings.nu $language $crate ($work | path join "gen" $crate)
    }
}

# Merge the generated trees of both crates into one directory (broker output
# repeats the client's files; the broker's copy wins, matching the published package).
def merged [work: string, dest: string] {
    rm -rf $dest
    mkdir $dest
    for crate in [mqtt-client mqtt-broker] {
        let src = ($work | path join "gen" $crate)
        if ($src | path exists) { cp -r ...(glob ($src | path join "*")) $dest }
    }
}

def stage-libs [spec: record, dest: string, crates: list<string>] {
    mkdir $dest
    for crate in $crates { cp (native-lib $spec $crate) $dest }
}

def lib-path-env [dir: string]: nothing -> record {
    match $nu.os-info.name {
        "macos" => { DYLD_LIBRARY_PATH: $dir }
        "windows" => { PATH: ($env.PATH | prepend $dir) }
        _ => { LD_LIBRARY_PATH: $dir }
    }
}

# Locate a JDK ≥ 22 (generated Java bindings use the Foreign Function & Memory API).
def find-jdk [] {
    let candidates = ([$env.JAVA_HOME? $env.JAVA_HOME_25_X64? $env.JAVA_HOME_25_ARM64? $env.JAVA_HOME_22_X64? $env.JAVA_HOME_23_X64? $env.JAVA_HOME_24_X64? "/opt/homebrew/opt/openjdk@25/libexec/openjdk.jdk/Contents/Home"]
        | compact | where { |p| ($p | path join "bin" "javac") | path exists })
    for home in $candidates {
        let major = (^($home | path join "bin" "java") -version e>| lines | first | parse --regex '"(?<v>\d+)' | get v.0? | default "0" | into int)
        if $major >= 22 { return $home }
    }
    null
}

def broker-bin []: nothing -> string {
    cd (repo)
    ^cargo build --quiet -p stem-mqtt-broker --bin mqtt-broker
    repo | path join "target" "debug" (if $nu.os-info.name == "windows" { "mqtt-broker.exe" } else { "mqtt-broker" })
}

# ── One runner per language ──────────────────────────────────────────────────
# Each returns `{ skip: <reason> }` or the `complete` record of the test run.

def run-python [spec: record, work: string] {
    if (which python3 | is-empty) { return { skip: "python3 not found" } }
    let stage = ($work | path join "stage")
    let pkg = ($stage | path join "mqttpkg")
    rm -rf $stage
    mkdir $pkg
    "" | save --force ($pkg | path join "__init__.py")
    for crate in [mqtt-client mqtt-broker] { cp ...(glob ($work | path join "gen" $crate "*.py")) $pkg }
    stage-libs $spec $pkg [mqtt-client mqtt-broker]
    let smoke = (smoke-src python | path join "smoke.py")
    with-env { PYTHONPATH: $stage } { do { cd $stage; ^python3 $smoke } | complete }
}

def run-go [spec: record, work: string] {
    if (which go | is-empty) { return { skip: "go not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    mkdir $stage
    # The broker output contains both packages (`mqtt_client` + `mqtt_broker`).
    cp -r ($work | path join "gen" "mqtt-broker" "mqtt_client") ($stage | path join "mqtt_client")
    cp -r ($work | path join "gen" "mqtt-broker" "mqtt_broker") ($stage | path join "mqtt_broker")
    stage-libs $spec $stage [mqtt-client mqtt-broker]
    cp (smoke-src go | path join "main.go") $stage
    "module mqtt_client\n\ngo 1.21\n" | save --force ($stage | path join "mqtt_client" "go.mod")
    "module mqtt_broker\n\ngo 1.21\n\nrequire mqtt_client v0.0.0\nreplace mqtt_client => ../mqtt_client\n" | save --force ($stage | path join "mqtt_broker" "go.mod")
    "module smoke\n\ngo 1.21\n\nrequire (\n\tmqtt_client v0.0.0\n\tmqtt_broker v0.0.0\n)\nreplace mqtt_client => ./mqtt_client\nreplace mqtt_broker => ./mqtt_broker\n" | save --force ($stage | path join "go.mod")
    with-env ({
        CGO_CFLAGS: $"-I($stage | path join mqtt_client) -I($stage | path join mqtt_broker)"
        CGO_LDFLAGS: $"-L($stage) -lmqtt_client -lmqtt_broker"
    } | merge (lib-path-env $stage)) {
        do { cd $stage; ^go run . } | complete
    }
}

def run-csharp [spec: record, work: string] {
    if (which dotnet | is-empty) { return { skip: "dotnet not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    mkdir ($stage | path join "native")
    merged $work ($stage | path join "generated")
    stage-libs $spec ($stage | path join "native") [mqtt-client mqtt-broker]
    cp ...(glob (smoke-src csharp | path join "*")) $stage
    let major = (^dotnet --version | split row "." | first)
    let tfm = $"net($major).0"
    do { cd $stage; ^dotnet run $"-p:SmokeTfm=($tfm)" } | complete
}

def run-java [spec: record, work: string] {
    let jdk = (find-jdk)
    if $jdk == null { return { skip: "no JDK >= 22 found (set JAVA_HOME)" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    mkdir ($stage | path join "out")
    # The broker output includes the client component, loaded from libmqtt_broker.
    stage-libs $spec $stage [mqtt-broker]
    let sources = (glob ($work | path join "gen" "mqtt-broker" "**" "*.java"))
    let javac = ($jdk | path join "bin" "javac")
    let java = ($jdk | path join "bin" "java")
    let compiled = (do { ^$javac -d ($stage | path join out) ...$sources (smoke-src java | path join "Smoke.java") } | complete)
    if $compiled.exit_code != 0 { return $compiled }
    do { ^$java --enable-native-access=ALL-UNNAMED $"-Djava.library.path=($stage)" -cp ($stage | path join out) Smoke } | complete
}

def run-dart [spec: record, work: string] {
    if (which dart | is-empty) { return { skip: "dart not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    mkdir ($stage | path join "lib")
    stage-libs $spec ($stage | path join "native") [mqtt-broker]
    cp ...(glob ($work | path join "gen" "mqtt-broker" "*.dart")) ($stage | path join "lib")
    cp -r ...(glob (smoke-src dart | path join "*")) $stage
    let pub = (do { cd $stage; ^dart pub get } | complete)
    if $pub.exit_code != 0 { return $pub }
    do { cd $stage; ^dart run bin/smoke.dart } | complete
}

def run-node [spec: record, work: string] {
    if (which npm | is-empty) { return { skip: "node/npm not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    cp -r ($work | path join "gen" "mqtt-client") $stage
    stage-libs $spec $stage [mqtt-client]
    cp (smoke-src node | path join "smoke.mjs") $stage
    let install = (do { cd $stage; ^npm install --silent --no-audit --no-fund } | complete)
    if $install.exit_code != 0 { return $install }
    with-env { STEM_MQTT_BROKER_BIN: (broker-bin) } { do { cd $stage; ^node smoke.mjs } | complete }
}

def run-node-livekit [spec: record, work: string] {
    if (which npm | is-empty) { return { skip: "node/npm not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    cp -r ($work | path join "gen" "mqtt-client") $stage
    stage-libs $spec $stage [mqtt-client]
    cp (smoke-src node-livekit | path join "smoke.ts") $stage
    let install = (do { cd $stage; ^npm install --silent --no-audit --no-fund typescript tsx @types/node ffi-rs uniffi-bindgen-react-native } | complete)
    if $install.exit_code != 0 { return $install }
    with-env { STEM_MQTT_BROKER_BIN: (broker-bin) } { do { cd $stage; ^npx tsx smoke.ts } | complete }
}

def run-haskell [spec: record, work: string] {
    if (which cabal | is-empty) { return { skip: "cabal not found" } }
    let stage = ($work | path join "stage")
    rm -rf $stage
    mkdir ($stage | path join "native")
    cp (build-root $spec | path join "target" "release" "libmqtt_broker.a") ($stage | path join "native")
    # The runtime package lives in the generator's repo; use the patched checkout.
    let runtime = ((prepare-source (spec-for haskell)) | path join "haskell" "uniffi-runtime")
    # `generate` already emitted a Cabal package (broker output includes the client).
    cp -r ($work | path join "gen" "mqtt-broker") ($stage | path join "gen")
    let cabal_file = ($stage | path join "gen" "stem-mqtt-bindings.cabal")
    open --raw $cabal_file
    | str replace --regex '(?m)^(\s*)extra-libraries:' $"${1}extra-lib-dirs: ($stage | path join native)\n${1}extra-libraries:"
    | save --force $cabal_file
    cp -r (smoke-src haskell) ($stage | path join "smoke")
    $"packages:\n  ($runtime)\n  gen\n  smoke\n" | save --force ($stage | path join "cabal.project")
    do { cd $stage; ^cabal run smoke } | complete
}

def run-language [spec: record, work: string] {
    match $spec.language {
        "python" => (run-python $spec $work)
        "go" => (run-go $spec $work)
        "csharp" => (run-csharp $spec $work)
        "java" => (run-java $spec $work)
        "dart" => (run-dart $spec $work)
        "node" => (run-node $spec $work)
        "node-livekit" => (run-node-livekit $spec $work)
        "haskell" => (run-haskell $spec $work)
        $other => { error make { msg: $"no smoke test for '($other)'" } }
    }
}

# Classify one run into PASS / FAIL / SKIP / XFAIL / XPASS.
export def classify [result: record, broken: string, strict: bool]: nothing -> string {
    if ($result | get skip? | is-not-empty) {
        return (if $strict { "FAIL" } else { "SKIP" })
    }
    let passed = ($result.exit_code == 0 and ($result.stdout | str contains "SMOKE OK"))
    if $broken != "" {
        if $passed { "XPASS" } else { "XFAIL" }
    } else if $passed { "PASS" } else { "FAIL" }
}

def main [
    ...languages: string
    --strict        # treat a missing toolchain as a failure
    --no-generate   # reuse bindings from a previous run
] {
    let wanted = if ($languages | is-empty) { $TESTED } else { $languages | each { |l| canonical-language $l } }
    mut rows = []
    for language in $wanted {
        let spec = (spec-for $language)
        let broken = (known-broken $language)
        let work = (work-dir $language)
        mkdir $work
        print $"\n══ ($language) ══════════════════════════════════════"
        cd (repo)
        let result = (try {
            if not $no_generate { generate $language $work }
            run-language $spec $work
        } catch { |e| { exit_code: 1, stdout: "", stderr: $e.msg } })
        let verdict = (classify $result $broken $strict)
        if $verdict in [FAIL XFAIL XPASS] {
            print ($result | get stdout? | default "" | lines | last 15 | str join (char nl))
            print ($result | get stderr? | default "" | lines | last 15 | str join (char nl))
        }
        if ($result | get skip? | is-not-empty) { print $"skipped: ($result.skip)" }
        print $"($language): ($verdict)(if $broken != '' { $' — known broken: ($broken)' } else { '' })"
        $rows = ($rows | append { language: $language, result: $verdict })
    }

    print "\n── Summary ──"
    print $rows
    let bad = ($rows | where result in [FAIL XPASS])
    if ($bad | is-not-empty) {
        error make { msg: $"binding tests failed: ($bad | get language | str join ', ')" }
    }
}
