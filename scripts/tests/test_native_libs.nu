#!/usr/bin/env nu
# The bindings' native libraries are built with `cargo rustc --crate-type`, never
# declared in Cargo.toml: Cargo names a cdylib crate's rlib without a hash, so
# builds with different dependency features overwrote it and a stale rlib linked
# a second tokio into the tests ("no reactor running").
use std/assert
use runner.nu *
use ../ci/native_libs.nu [TARGETS target-platform target-lib-file natives-tree]

const ROOT = path self ../..

# Files that build the native libraries, and the crate type each one needs.
const BUILDERS = [
    [file needs];
    ["scripts/generate_bindings.nu" "cdylib,staticlib"]
    ["packaging/kotlin/stage.sh" cdylib]
    ["packaging/kotlin/stage-android.sh" cdylib]
    ["packaging/swift/build_xcframework.sh" staticlib]
    ["packaging/haskell/hackage/Setup.hs" staticlib]
    ["packaging/dart/pub/hook/build.dart" cdylib]
    [".github/workflows/release.yml" "cdylib,staticlib"]
    ["scripts/ci/native_libs.nu" cdylib]
]

def read [file: string]: nothing -> string { open --raw ($ROOT | path join $file) | decode utf-8 }

def "test native libs: the crates declare only the rlib" [] {
    for c in [mqtt-client mqtt-broker] {
        let lib = read $"crates/($c)/Cargo.toml" | from toml | get lib
        assert equal $lib.crate-type ["lib"] $"($c) must not declare cdylib/staticlib"
    }
}

def "test native libs: every builder asks for its crate type" [] {
    for b in $BUILDERS {
        let text = read $b.file
        assert ($text | str contains "rustc") $"($b.file) must build with cargo rustc"
        assert ($text =~ $"crate-type['\",]*\\s*['\"]?($b.needs)") $"($b.file) must pass --crate-type ($b.needs)"
    }
}

def "test native libs: targets map to the platform ids packages use" [] {
    use ../bindings/spec.nu [node-prebuild-target]
    assert equal (target-platform "aarch64-apple-darwin") "macos-aarch64"
    assert equal (target-platform "x86_64-pc-windows-msvc") "windows-x86_64"
    assert error { target-platform "riscv64gc-unknown-linux-gnu" }
    # Every platform has an npm package target.
    for t in ($TARGETS | columns) { node-prebuild-target (target-platform $t) | ignore }
}

def "test native libs: library file names follow the target, not the host" [] {
    assert equal (target-lib-file "mqtt_client" "x86_64-pc-windows-msvc") "mqtt_client.dll"
    assert equal (target-lib-file "mqtt_client" "x86_64-apple-darwin") "libmqtt_client.dylib"
    assert equal (target-lib-file "mqtt_broker" "aarch64-unknown-linux-gnu") "libmqtt_broker.so"
}

def "test native libs: a target tree becomes a platform tree, replacing the host libraries" [] {
    let tmp = (mktemp --directory)
    let src = ($tmp | path join "src")
    mkdir ($src | path join "aarch64-unknown-linux-gnu") ($src | path join "x86_64-apple-darwin")
    "x" | save ($src | path join "aarch64-unknown-linux-gnu" "libmqtt_client.so")
    "x" | save ($src | path join "x86_64-apple-darwin" "libmqtt_client.dylib")
    let dest = ($tmp | path join "native")
    mkdir ($dest | path join "host-only")
    natives-tree $src $dest
    assert equal (ls $dest | get name | path basename | sort) ["linux-aarch64" "macos-x86_64"]
    assert ($dest | path join "macos-x86_64" "libmqtt_client.dylib" | path exists)
    rm -rf $tmp
}

def main [] { run-tests }
