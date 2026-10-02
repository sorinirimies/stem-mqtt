#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — build third-party generators from (patched) source
# ──────────────────────────────────────────────────────────────────────────────
# Most generators install straight from a tag with `cargo install --git`. A
# generator that needs local fixes (currently uniffi-bindgen-haskell) is
# instead cloned at its pinned revision, patched with the `patch` file named in
# scripts/bindings/spec.nu, and installed from that checkout — so the fix is
# reviewable in this repo and disappears the day upstream merges it.

use spec.nu *

# Directory the generator's source is checked out in.
export def source-dir [spec: record]: nothing -> string {
    $env.FILE_PWD | path dirname | path join "target" "bindgen-src" $spec.tool
}

# Value following `flag` in the install argument list (e.g. `--rev`).
export def install-arg [spec: record, flag: string]: nothing -> string {
    $spec.install | skip until { |a| $a == $flag } | get 1
}

# Clone (if needed), check out the pinned rev, and apply the patch. Idempotent.
export def prepare-source [spec: record]: nothing -> string {
    let dir = (source-dir $spec)
    if not ($dir | path exists) {
        mkdir ($dir | path dirname)
        ^git clone --quiet (install-arg $spec "--git") $dir
    }
    ^git -C $dir checkout --quiet --force (install-arg $spec "--rev")
    ^git -C $dir reset --quiet --hard
    if $spec.patch != "" {
        let patch = ($env.FILE_PWD | path dirname | path join $spec.patch)
        ^git -C $dir apply $patch
    }
    $dir
}
