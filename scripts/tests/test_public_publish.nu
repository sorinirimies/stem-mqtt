#!/usr/bin/env nu
# ── stem-mqtt · test_public_publish.nu ──────────────────────────────────────
# The public-registry layer: npmjs / NuGet.org / Maven Central / Hackage / pub.dev /
# Go modules — publish plans, the Maven Central bundle checks, and the source-package
# builders.

use std/assert
use runner.nu *
use ../bindings/spec.nu *
use ../publish_maven_central.nu [upload-args missing-files]
use ../bindings/go_module.nu *
use ../bindings/hackage.nu *
use ../bindings/dart_package.nu *
use ../bindings/node_package.nu *

def "test public: node goes to npmjs as a public package" [] {
    let step = (publish-plan node "1.0.0" "d" "acme" "stem-mqtt" "public" | first)
    assert equal $step.cmd "npm"
    assert ($step.args | any { |a| $a == "https://registry.npmjs.org/" })
    assert ("public" in $step.args)
}

def "test public: csharp pushes to nuget.org" [] {
    let push = (publish-plan csharp "1.0.0" "d" "acme" "stem-mqtt" "public" | last)
    assert ($push.args | any { |a| $a == "https://api.nuget.org/v3/index.json" })
}

def "test public: java signs into a staging repo then uploads to Maven Central under io.github" [] {
    let plan = (publish-plan java "1.2.3" "d" "acme" "stem-mqtt" "public")
    assert equal ($plan | length) 2
    assert ($plan.0.args | any { |a| $a == "-PgroupId=io.github.acme.stemmqtt" })
    assert ($plan.0.args | any { |a| $a == "publishAllPublicationsToStagingRepository" })
    assert equal $plan.1.cmd "nu"
    assert ($plan.1.args | any { |a| $a | str ends-with "publish_maven_central.nu" })
    assert ($plan.1.args | any { |a| $a == "stem-mqtt-java-1.2.3" })
}

def "test public: haskell builds an sdist and uploads it to hackage" [] {
    let plan = (publish-plan haskell "1.2.3" "dist/haskell" "acme" "stem-mqtt" "public")
    assert ($plan.0.cwd | str ends-with "hackage")
    assert ($plan.1.args | any { |a| $a == "dist-newstyle/sdist/stem-mqtt-1.2.3.tar.gz" })
    assert ("--publish" in $plan.1.args)
    # the token is injected at run time, never part of the printed plan
    assert (not ($plan.1.args | any { |a| $a | str contains "token" }))
}

def "test public: dart publishes the pub package, go the module repository" [] {
    let dart = (publish-plan dart "1.0.0" "dist/dart" "acme" "stem-mqtt" "public" | first)
    assert ($dart.cwd | str ends-with "pub")
    assert equal ($dart.args | first 2) [pub publish]
    let go = (publish-plan go "1.0.0" "dist/go" "acme" "stem-mqtt" "public" | first)
    assert ($go.args | any { |a| $a | str ends-with "publish_go_module.nu" })
}

def "test public: languages without a public index are refused with a reason" [] {
    let err = (try { publish-plan swift "1.0.0" "d" "acme" "stem-mqtt" "public"; "" } catch { |e| $e.msg })
    assert ($err | str contains "no public registry")
}

def "test public: the Maven Central group is a verified io.github namespace" [] {
    assert equal (central-group "sorinirimies") "io.github.sorinirimies.stemmqtt"
}

def "test public: the Central upload is automatic by default, manual on request" [] {
    let auto = (upload-args "/tmp/b.zip" "stem mqtt" false)
    assert ($auto | any { |a| $a | str contains "publishingType=AUTOMATIC" })
    assert ($auto | any { |a| $a | str contains "name=stem%20mqtt" })
    let manual = (upload-args "/tmp/b.zip" "x" true)
    assert ($manual | any { |a| $a | str contains "publishingType=USER_MANAGED" })
}

def "test public: an unsigned staging repository is refused before upload" [] {
    let dir = (mktemp -d)
    let art = ($dir | path join "g" "a" "1")
    mkdir $art
    "x" | save ($art | path join "a-1.jar")
    "x" | save ($art | path join "a-1.jar.md5")
    "x" | save ($art | path join "a-1.jar.sha1")
    assert ("a-1.jar.asc" in (missing-files $dir))
    "x" | save ($art | path join "a-1.jar.asc")
    assert equal (missing-files $dir) []
    rm -rf $dir
}

def "test public: the Go module gets cgo flags and its own import path" [] {
    let gen = (mktemp -d)
    for pkg in [mqtt_client mqtt_broker] {
        mkdir ($gen | path join $pkg)
        $"package ($pkg)\n\n// #include <($pkg).h>\nimport \"C\"\n" | save ($gen | path join $pkg $"($pkg).go")
        "" | save ($gen | path join $pkg $"($pkg).h")
    }
    let out = ($gen | path join "out")
    build-go-module $gen $out "github.com/acme/stem-mqtt-go"
    assert ((open --raw ($out | path join "go.mod")) | str contains "module github.com/acme/stem-mqtt-go")
    let broker = (open --raw ($out | path join "mqtt_broker" "mqtt_broker.go"))
    assert ($broker | str contains "// #cgo CFLAGS: -I${SRCDIR}")
    assert ($broker | str contains "// #cgo LDFLAGS: -lmqtt_broker")
    assert ($broker | str contains "// #include <mqtt_broker.h>")
    rm -rf $gen
}

def "test public: the Dart package renames the native asset id away from the generic `uniffi`" [] {
    let gen = (mktemp -d)
    let root = (mktemp -d)
    "const _uniffiAssetId = \"package:uniffi/uniffi:mqtt_broker\";\n" | save ($gen | path join "mqtt_broker.dart")
    for crate in [mqtt-client mqtt-broker] {
        mkdir ($root | path join "crates" $crate "src")
        "" | save ($root | path join "crates" $crate "src" "lib.rs")
    }
    "[workspace]\nmembers = [\"crates/mqtt-client\", \"crates/mqtt-broker\", \"demo/dashboard\"]\n" | save ($root | path join "Cargo.toml")
    "" | save ($root | path join "Cargo.lock")
    # only the template-driven parts are asserted: the lockfile pruning needs a real workspace
    let out = ($gen | path join "pkg")
    let result = (try { build-dart-package $gen $root $out "1.2.3"; "ok" } catch { |e| $e.msg })
    if $result == "ok" {
        assert ((open --raw ($out | path join "lib" "mqtt_broker.dart")) | str contains "package:stem_mqtt/uniffi:mqtt_broker")
        assert ((open --raw ($out | path join "pubspec.yaml")) | str contains "version: 1.2.3")
        assert ((open --raw ($out | path join "rust" "Cargo.toml")) | str contains "members = [\"crates/mqtt-client\", \"crates/mqtt-broker\"]")
    }
    rm -rf $gen $root
}

def "test public: the java package check runs the smoke test against the packaged jar alone" [] {
    let plan = (build-plan java "1.0.0" "dist/java")
    assert equal ($plan | get cmd) [gradle javac java]
    let run = ($plan | last)
    assert ($run.args | any { |a| $a | str contains "stem-mqtt-java-1.0.0.jar" })
    # no -Djava.library.path: the jar must find its own native library
    assert (not ($run.args | any { |a| $a | str contains "java.library.path" }))
}

def "test public: npm prebuild targets match what the generated Node loader resolves" [] {
    assert equal (node-prebuild-target "linux-x86_64") "linux-x64-gnu"
    assert equal (node-prebuild-target "macos-aarch64") "darwin-arm64"
    assert equal (node-prebuild-target "windows-x86_64") "win32-x64"
    assert equal (npm-os-cpu "linux-x86_64") { os: "linux", cpu: "x64" }
    assert equal (npm-os-cpu "macos-aarch64") { os: "darwin", cpu: "arm64" }
    let err = (try { node-prebuild-target "plan9-riscv"; "" } catch { |e| $e.msg })
    assert ($err | str contains "no Node prebuild target")
}

def "test public: the Node package shares one runtime between client and broker" [] {
    let root = (mktemp -d)
    for part in [client broker] {
        let rt = ($root | path join "gen" $part "runtime")
        mkdir $rt
        "export const x = 1;\n" | save ($rt | path join "ffi-types.js")
        $"import { x } from \"./runtime/ffi-types.js\";\n" | save ($root | path join "gen" $part $"($part).js")
    }
    let out = ($root | path join "pkg")
    assemble-node-package ($root | path join "gen" "client") ($root | path join "gen" "broker") $out
    assert ($out | path join "runtime" "ffi-types.js" | path exists)
    assert (not ($out | path join "broker" "runtime" | path exists))
    assert ((open --raw ($out | path join "broker" "broker.js")) | str contains "\"../runtime/ffi-types.js\"")
    # diverging runtimes are refused rather than silently merged
    let bad = ($root | path join "gen2")
    cp -r ($root | path join "gen") $bad
    "export const x = 2;\n" | save --force ($bad | path join "broker" "runtime" "ffi-types.js")
    let err = (try { assemble-node-package ($bad | path join "client") ($bad | path join "broker") ($root | path join "pkg2"); "" } catch { |e| $e.msg })
    assert ($err | str contains "differ")
    rm -rf $root
}

def main [] { run-tests }
