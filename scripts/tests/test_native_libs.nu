#!/usr/bin/env nu
# The bindings' native libraries are built with `cargo rustc --crate-type`, never
# declared in Cargo.toml: Cargo names a cdylib crate's rlib without a hash, so
# builds with different dependency features overwrote it and a stale rlib linked
# a second tokio into the tests ("no reactor running").
use std/assert
use runner.nu *

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

def main [] { run-tests }
