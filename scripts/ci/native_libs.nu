#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — the client and broker native libraries, per Rust target
# ──────────────────────────────────────────────────────────────────────────────
# Release CI builds both cdylibs for every target, once per UniFFI release the
# packaged bindings need ("workspace", or a generator's pinned release), and the
# packages take only their platform's libraries from that tree:
#
#   nu scripts/ci/native_libs.nu build <uniffi> <target> <out>
#       → <out>/<target>/{libmqtt_client,libmqtt_broker}.{so,dylib} | mqtt_*.dll
#
# `natives-tree` lays such a tree out by platform id (native/<platform>/), the
# layout publish_packages.nu packages from.
# ──────────────────────────────────────────────────────────────────────────────

use ../bindings/spec.nu [CRATES]
use ../bindings/scratch.nu [library-root]

# Rust target → platform id (`<os>-<arch>`, as spec.nu's platform-id names the host).
export const TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x86_64"
    "aarch64-unknown-linux-gnu": "linux-aarch64"
    "x86_64-apple-darwin": "macos-x86_64"
    "aarch64-apple-darwin": "macos-aarch64"
    "x86_64-pc-windows-msvc": "windows-x86_64"
}

export def target-platform [target: string]: nothing -> string {
    $TARGETS | get --optional $target | default {
        error make --unspanned { msg: $"unknown Rust target '($target)'. Known: ($TARGETS | columns | str join ', ')" }
    }
}

# The file name of a library built for a target (the host's naming does not apply).
export def target-lib-file [lib: string, target: string]: nothing -> string {
    if ($target | str contains "windows") { $"($lib).dll" } else if ($target | str contains "apple") { $"lib($lib).dylib" } else { $"lib($lib).so" }
}

# Copy a <target>/ tree into <dest>/<platform>/ (replacing what was there).
export def natives-tree [src: string, dest: string] {
    rm -rf $dest
    mkdir $dest
    for dir in (ls $src | where type == dir | get name) {
        let platform = (target-platform ($dir | path basename))
        cp -r $dir ($dest | path join $platform)
    }
}

def "main build" [uniffi: string, target: string, out: string] {
    let repo = $env.PWD
    let out = ($out | path expand)
    let root = (library-root $uniffi $repo)
    let dest = ($out | path join $target)
    mkdir $dest
    for crate in ($CRATES | columns) {
        let package = ($CRATES | get $crate).package
        let lib = ($CRATES | get $crate).lib
        do { cd $root; ^cargo rustc --release -p $package --lib --crate-type cdylib --target $target }
        let file = (target-lib-file $lib $target)
        cp ($root | path join "target" $target "release" $file) $dest
    }
    ls $dest | select name size | print
}

def main [] {
    print "usage: nu scripts/ci/native_libs.nu build <workspace|uniffi-version> <rust-target> <out>"
}
