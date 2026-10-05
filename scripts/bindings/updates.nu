#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — pin / upstream comparison for the third-party generators
# ──────────────────────────────────────────────────────────────────────────────
# Pure helpers (unit-tested) plus the network lookups used by
# scripts/check_bindgen_updates.nu.

use spec.nu *

# What a generator's install arguments pin: {kind: tag|rev|version, value, source}.
# `source` is the git URL (tag/rev) or the crate name (version).
export def pin-of [spec: record]: nothing -> record {
    let args = $spec.install
    let value_after = {|flag| $args | skip until { |a| $a == $flag } | get 1 }
    if ("--tag" in $args) {
        { kind: "tag", value: (do $value_after "--tag"), source: (do $value_after "--git") }
    } else if ("--rev" in $args) {
        { kind: "rev", value: (do $value_after "--rev"), source: (do $value_after "--git") }
    } else {
        { kind: "version", value: (do $value_after "--version"), source: ($args | first) }
    }
}

# The UniFFI release a `vX.Y.Z+vA.B.C` tag was built for (`A.B.C`), or "" if the
# tag doesn't follow that convention.
export def uniffi-target [tag: string]: nothing -> string {
    $tag | parse --regex '\+v(?<uniffi>\d+\.\d+\.\d+)$' | get uniffi.0? | default ""
}

# Sort key for a tag or version: the numeric components before any `+build`
# suffix, zero-padded so plain string comparison orders them numerically
# (nu can't compare lists, and "0.10" must sort after "0.9").
export def version-key [tag: string]: nothing -> string {
    $tag
    | split row "+" | first
    | parse --regex '(\d+)' | get capture0
    | each { |n| $n | fill --alignment right --character "0" --width 8 }
    | str join "."
}

# The highest tag in `tags` (by the generator's own version, ignoring the UniFFI suffix).
export def newest [tags: list<string>]: nothing -> string {
    if ($tags | is-empty) { return "" }
    $tags
    | each { |t| { tag: $t, key: (version-key $t) } }
    | sort-by key
    | last
    | get tag
}

# Does `candidate` come after `pinned`?
export def is-newer [pinned: string, candidate: string]: nothing -> bool {
    if $candidate == "" or $candidate == $pinned { return false }
    (version-key $candidate) > (version-key $pinned)
}

# Tag names from `git ls-remote --tags` output (annotated-tag `^{}` duplicates dropped).
export def tags-from-ls-remote [output: string]: nothing -> list<string> {
    $output
    | lines
    | each { |l| $l | split row "\t" | get 1? | default "" }
    | where { |r| $r starts-with "refs/tags/" and not ($r | str ends-with "^{}") }
    | each { |r| $r | str replace "refs/tags/" "" }
}

# Look up the newest upstream tag/rev/version for one pin. Network access.
export def latest-upstream [pin: record]: nothing -> string {
    match $pin.kind {
        "tag" => {
            let out = (^git ls-remote --tags $pin.source | complete)
            if $out.exit_code != 0 { return "" }
            newest (tags-from-ls-remote $out.stdout)
        }
        "rev" => {
            let out = (^git ls-remote $pin.source HEAD | complete)
            if $out.exit_code != 0 { return "" }
            $out.stdout | split row "\t" | first | str trim
        }
        _ => {
            let out = (^curl -fsS -m 20 -A "stem-mqtt-update-check" $"https://crates.io/api/v1/crates/($pin.source)" | complete)
            if $out.exit_code != 0 { return "" }
            ($out.stdout | from json | get versions | where yanked == false | get num | get 0? | default "")
        }
    }
}
