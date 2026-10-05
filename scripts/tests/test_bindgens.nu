#!/usr/bin/env nu
# ── stem-mqtt · test_bindgens.nu ────────────────────────────────────────────
# Tests for scripts/bindings/spec.nu — the binding-generator table, the
# scratch-workspace Cargo.toml rewrite, and the GitHub Packages publish plan.

use std/assert
use runner.nu *
use ../bindings/spec.nu *
use ../test_bindings.nu classify
use ../bindings/updates.nu *

def "test bindgens: every requested language is supported" [] {
    let have = (specs | get language)
    for lang in [go csharp dart java node node-livekit haskell kotlin swift python] {
        assert ($lang in $have) $"missing language ($lang)"
    }
}

def "test bindgens: external generators pin an explicit version" [] {
    for spec in (specs | where kind == external) {
        let pinned = ($spec.install | any { |a| $a in ["--tag" "--rev" "--version"] })
        assert $pinned $"($spec.language) generator is not pinned"
    }
}

def "test bindgens: aliases resolve" [] {
    assert equal (canonical-language "cs") "csharp"
    assert equal (canonical-language "C#") "csharp"
    assert equal (canonical-language "hs") "haskell"
    assert equal (canonical-language "go") "go"
}

def "test bindgens: unknown language lists the options" [] {
    let msg = (try { spec-for cobol; "" } catch { |e| $e.msg })
    assert ($msg | str contains "unsupported language")
    assert ($msg | str contains "haskell")
}

def "test bindgens: workspace uniffi satisfies every generator that claims to" [] {
    # Generators marked `workspace` must be able to read what the workspace
    # builds, so their pinned tag/version must name the same minor release.
    let ws = (workspace-uniffi | split row "." | first 2 | str join ".")
    for spec in (specs | where kind == external and uniffi == workspace) {
        let tags = ($spec.install | where { |a| $a =~ '\+v\d' })
        for tag in $tags {
            assert ($tag | str contains $"+v($ws)") $"($spec.language): ($tag) vs workspace uniffi ($ws)"
        }
    }
}

def "test bindgens: pin-workspace-toml pins uniffi and trims members" [] {
    let toml = (open --raw Cargo.toml)
    let pinned = (pin-workspace-toml $toml "0.32.0")
    assert ($pinned | str contains 'version = "=0.32.0"')
    assert ($pinned | str contains 'members = ["crates/mqtt-client", "crates/mqtt-broker"]')
    assert (not ($pinned | str contains "mqtt-client-node"))
    assert (not ($pinned | str contains "demo/dashboard"))
}

def "test bindgens: generator-args name the library and output dir" [] {
    let args = (generator-args go "/x/libmqtt_client.dylib" "/out" mqtt_client)
    assert ("/x/libmqtt_client.dylib" in $args)
    assert ("/out" in $args)
    let node = (generator-args node "/x/lib.dylib" "/out" mqtt_broker)
    assert ("mqtt_broker" in $node)
}

def "test bindgens: haskell is installed from a patched, pinned checkout" [] {
    let spec = (spec-for haskell)
    assert ($spec.patch | path exists) "patch file missing"
    assert ($spec.install | any { |a| $a == "--rev" })
    assert equal $spec.crate_path "crates/uniffi-bindgen-haskell"
}

def "test bindgens: the haskell patch applies to the pinned revision" [] {
    # Guards against the patch rotting when the pin moves.
    let spec = (spec-for haskell)
    let rev = ($spec.install | skip until { |a| $a == "--rev" } | get 1)
    let patch = ($spec.patch | path expand)
    let dir = (mktemp -d)
    let cloned = (do { ^git clone --quiet ($spec.install | skip until { |a| $a == "--git" } | get 1) $dir } | complete)
    if $cloned.exit_code != 0 { return }  # offline: nothing to check
    ^git -C $dir checkout --quiet $rev
    let applied = (do { ^git -C $dir apply --check $patch } | complete)
    rm -rf $dir
    assert equal $applied.exit_code 0 $"patch no longer applies: ($applied.stderr)"
}

def "test bindgens: node generator refuses the broker with a reason" [] {
    let blocked = (unsupported-crates node)
    assert ("mqtt-broker" in ($blocked | columns))
    assert ((unsupported-crates go | columns | is-empty))
}

def "test bindgens: every registry produces a publish plan" [] {
    for lang in [java csharp node go dart haskell] {
        let plan = (publish-plan $lang "1.2.3" "dist/x" "acme")
        assert ($plan | is-not-empty) $"no plan for ($lang)"
        assert ($plan | all { |s| $s.cmd != "" })
    }
}

def "test bindgens: OCI plan targets ghcr.io under the owner with the version" [] {
    let step = (publish-plan go "1.2.3" "dist/go" "acme" | first)
    assert equal $step.cmd "oras"
    assert ($step.args | any { |a| $a == "ghcr.io/acme/stem-mqtt-go:1.2.3" })
    assert ($step.args | any { |a| $a | str contains "image.source=https://github.com/acme/stem-mqtt" })
}

def "test bindgens: nuget and maven plans use the GitHub Packages endpoints" [] {
    let nuget = (publish-plan csharp "1.0.0" "d" "acme" | last)
    assert ($nuget.args | any { |a| $a == "https://nuget.pkg.github.com/acme/index.json" })
    let maven = (publish-plan java "1.0.0" "d" "acme" | first)
    assert equal $maven.cmd "gradle"
    assert ($maven.args | any { |a| $a == "-PmavenUrl=https://maven.pkg.github.com/acme/stem-mqtt" })
}

def "test bindgens: dotnet RIDs map every shipped platform" [] {
    assert equal (dotnet-rid "linux-x86_64") "linux-x64"
    assert equal (dotnet-rid "macos-aarch64") "osx-arm64"
    assert equal (dotnet-rid "windows-x86_64") "win-x64"
}

def "test bindgens: known-broken languages carry a reason and the broken maturity" [] {
    assert ((known-broken node-livekit) != "")
    assert equal (spec-for node-livekit | get maturity) "broken"
    # Haskell works through our patched generator.
    assert equal (known-broken haskell) ""
    assert equal (known-broken go) ""
}

def "test bindgens: every runtime-tested language has a smoke test on disk" [] {
    for lang in [python go csharp java dart node node-livekit haskell] {
        assert ((glob $"tests/bindings/($lang)/*" | length) > 0) $"no smoke test for ($lang)"
    }
}

def "test bindgens: callback-less generators are declared" [] {
    assert (not (capabilities dart | get callbacks))
    assert (not (capabilities haskell | get callbacks))
    assert (capabilities go | get callbacks)
    assert (not (capabilities node | get broker))
}

def "test bindgens: classify maps results to verdicts" [] {
    let ok = { exit_code: 0, stdout: "SMOKE OK\n", stderr: "" }
    let bad = { exit_code: 1, stdout: "", stderr: "boom" }
    let silent = { exit_code: 0, stdout: "no marker", stderr: "" }
    assert equal (classify $ok "" false) "PASS"
    assert equal (classify $bad "" false) "FAIL"
    assert equal (classify $silent "" false) "FAIL"
    assert equal (classify $bad "upstream bug" false) "XFAIL"
    assert equal (classify $ok "upstream bug" false) "XPASS"
    assert equal (classify { skip: "no go" } "" false) "SKIP"
    assert equal (classify { skip: "no go" } "" true) "FAIL"
}

def "test bindgens: build-plan covers every registry without uploading" [] {
    for lang in [java csharp node go dart haskell] {
        let plan = (build-plan $lang "1.0.0" "dist/x")
        assert ($plan | is-not-empty) $"no build plan for ($lang)"
        for step in $plan {
            # A build plan must never contain an upload.
            assert (not ($step.args | any { |a| $a in ["publish" "push"] })) $"($lang) build plan uploads: ($step.args)"
        }
    }
}

def "test bindgens: build-plan uses the native build tool of each registry" [] {
    assert equal (build-plan csharp "1.0.0" "d" | first | get cmd) "dotnet"
    assert equal (build-plan java "1.0.0" "d" | first | get cmd) "gradle"
    assert equal (build-plan node "1.0.0" "d" | first | get cmd) "npm"
    assert equal (build-plan haskell "1.0.0" "d" | first | get cmd) "tar"
}

def "test bindgens: package names are per language and prefixed" [] {
    assert equal (package-name go) "stem-mqtt-go"
    assert equal (package-name cs) "stem-mqtt-csharp"
}

def "test bindgens: pins are classified as tag, rev or crates.io version" [] {
    assert equal (pin-of (spec-for go) | get kind) "tag"
    assert equal (pin-of (spec-for haskell) | get kind) "rev"
    assert equal (pin-of (spec-for java) | get kind) "version"
    assert equal (pin-of (spec-for java) | get source) "uniffi-bindgen-java"
    assert equal (pin-of (spec-for go) | get value) "v0.7.1+v0.31.0"
}

def "test bindgens: uniffi target is read from the tag suffix" [] {
    assert equal (uniffi-target "v0.11.0+v0.31.0") "0.31.0"
    assert equal (uniffi-target "v0.10.0+v0.29.4") "0.29.4"
    assert equal (uniffi-target "uniffi-bindgen-node@0.1.5") ""
}

def "test bindgens: newest tag ignores the uniffi suffix and sorts numerically" [] {
    let tags = ["v0.5.0+v0.29.5" "v0.7.1+v0.31.0" "v0.10.0+v0.29.4" "v0.7.0+v0.31.0"]
    assert equal (newest $tags) "v0.10.0+v0.29.4"   # 0.10 > 0.7, not lexical
    assert equal (newest []) ""
}

def "test bindgens: is-newer compares versions, not strings" [] {
    assert (is-newer "v0.9.0+v0.28.3" "v0.10.0+v0.31.0")
    assert (not (is-newer "v0.7.1+v0.31.0" "v0.7.1+v0.31.0"))
    assert (not (is-newer "v0.7.1+v0.31.0" "v0.6.0+v0.30.0"))
    assert (not (is-newer "0.4.2" ""))
}

def "test bindgens: ls-remote output is reduced to plain tag names" [] {
    let out = "abc\trefs/tags/v1.0.0\ndef\trefs/tags/v1.0.0^{}\n123\trefs/tags/v1.1.0\n"
    assert equal (tags-from-ls-remote $out) ["v1.0.0" "v1.1.0"]
}

def "test bindgens: the gitea target points every registry at the instance" [] {
    let base = "http://192.168.1.44:3000/"
    let npm = (publish-plan node "1.0.0" "d" "sorin" "stem-mqtt" "gitea" $base | first)
    assert ($npm.args | any { |a| $a == "http://192.168.1.44:3000/api/packages/sorin/npm/" })
    assert (not ($npm.args | any { |a| $a == "--access" })) "gitea npm has no --access flag"
    let nuget = (publish-plan csharp "1.0.0" "d" "sorin" "stem-mqtt" "gitea" $base | last)
    assert ($nuget.args | any { |a| $a == "http://192.168.1.44:3000/api/packages/sorin/nuget/index.json" })
    let maven = (publish-plan java "1.0.0" "d" "sorin" "stem-mqtt" "gitea" $base | first)
    assert ($maven.args | any { |a| $a == "-PmavenUrl=http://192.168.1.44:3000/api/packages/sorin/maven" })
    let oci = (publish-plan go "1.0.0" "d" "sorin" "stem-mqtt" "gitea" $base | first)
    assert ($oci.args | any { |a| $a == "192.168.1.44:3000/sorin/stem-mqtt-go:1.0.0" })
    assert ("--plain-http" in $oci.args) "an http:// registry needs --plain-http"
    let https = (publish-plan go "1.0.0" "d" "sorin" "stem-mqtt" "gitea" "https://git.example.com" | first)
    assert (not ("--plain-http" in $https.args))
}

def "test bindgens: the gitea target requires a base url and unknown targets are refused" [] {
    let missing = (try { publish-plan go "1" "d" "o" "r" "gitea" ""; "" } catch { |e| $e.msg })
    assert ($missing | str contains "--base-url")
    let unknown = (try { publish-plan go "1" "d" "o" "r" "gitlab" ""; "" } catch { |e| $e.msg })
    assert ($unknown | str contains "unknown publish target")
}

def "test bindgens: the default target is unchanged GitHub Packages" [] {
    let oci = (publish-plan go "1.0.0" "d" "acme" | first)
    assert ($oci.args | any { |a| $a == "ghcr.io/acme/stem-mqtt-go:1.0.0" })
    assert (not ("--plain-http" in $oci.args))
}

def main [] { run-tests }
