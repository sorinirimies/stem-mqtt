#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — generate foreign-language bindings
# ──────────────────────────────────────────────────────────────────────────────
# Builds a crate's cdylib and generates bindings for one language from the
# UniFFI metadata embedded in it.
#
# Usage:
#   nu scripts/generate_bindings.nu <language> [crate] [out-dir]
#
#   language  kotlin | swift | python | go | csharp | java | dart | node |
#             haskell        (see scripts/bindings/spec.nu)
#   crate     mqtt-client (default) | mqtt-broker
#   out-dir   bindings/<language>/<crate> by default
#
# Third-party generators must be installed first: `nu scripts/install_bindgens.nu`.
# Generators that need a different UniFFI release than the workspace (Haskell)
# are fed a library built in a scratch workspace pinned to that
# release — the workspace itself is never modified.
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *

# Create (or refresh) target/uniffi-<version>/ — a workspace containing only
# the two library crates with `uniffi` pinned to exactly `version` — and
# return its absolute path.
def prepare-scratch-workspace [version: string]: nothing -> string {
    let root = ($env.PWD | path join "target" $"uniffi-($version)")
    mkdir ($root | path join "crates")
    for crate in (ls crates | where name =~ 'mqtt-(client|broker)$' | get name) {
        let dest = ($root | path join $crate)
        rm -rf $dest
        cp -r $crate $dest
    }
    pin-workspace-toml (open --raw Cargo.toml) $version | save --force ($root | path join "Cargo.toml")
    # Reuse the scratch lockfile between runs so re-resolution stays offline.
    if not ($root | path join "Cargo.lock" | path exists) {
        cp Cargo.lock ($root | path join "Cargo.lock")
    }
    cd $root
    ^cargo update -p uniffi --precise $version
    $root
}

# Find a generator executable on PATH or under $BINDGEN_ROOT/bin.
def find-tool [tool: string] {
    let on_path = (which $tool)
    if ($on_path | is-not-empty) {
        return ($on_path | first | get path)
    }
    let rooted = ($env.BINDGEN_ROOT? | default "" | path join "bin" $tool)
    if ($env.BINDGEN_ROOT? | is-not-empty) and ($rooted | path exists) {
        return $rooted
    }
    error make {
        msg: $"generator '($tool)' not found — run: nu scripts/install_bindgens.nu"
    }
}

def main [
    language: string
    crate: string = "mqtt-client"
    out_dir?: string
] {
    let spec = (spec-for $language)
    let target = (crate-for $crate)
    let out = ($out_dir | default (default-out-dir $spec.language $crate))
    let out_abs = ($out | path expand)
    let repo = $env.PWD

    # UniFFI bindgen reads metadata symbols from the compiled library. Never
    # let a workspace/profile override strip them before generation (Linux/ELF
    # otherwise "succeeds" with an empty output directory).
    $env.CARGO_PROFILE_RELEASE_STRIP = "none"

    # Build the library in the workspace the generator is compatible with.
    let tool = if $spec.kind == "external" { find-tool $spec.tool } else { "" }
    let root = if $spec.uniffi == "workspace" { $env.PWD } else { prepare-scratch-workspace $spec.uniffi }

    print $"==> building ($crate) \(release, cdylib\) in ($root)"
    cd $root
    ^cargo build --release -p $target.package

    let lib = ($root | path join "target" "release" (native-lib-name $target.lib))
    if not ($lib | path exists) {
        error make { msg: $"expected native library not found: ($lib)" }
    }

    print $"==> generating ($spec.language) bindings -> ($out_abs)"
    mkdir $out_abs
    if $spec.kind == "builtin" {
        ^cargo run --release -p $target.package --features uniffi/cli --bin uniffi-bindgen -- generate --library $lib --language $spec.language --out-dir $out_abs
    } else {
        let args = (generator-args $spec.language $lib $out_abs $target.lib $repo)
        run-external $tool ...$args
    }

    # Release jobs must never publish an empty archive (older releases did).
    if (glob ($out_abs | path join "**" "*") | where { |p| ($p | path type) == "file" } | is-empty) {
        error make { msg: $"($spec.language) binding generation produced no files in ($out_abs)" }
    }
    print $"done: ($out_abs)"
}
