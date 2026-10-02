#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — install the third-party UniFFI binding generators
# ──────────────────────────────────────────────────────────────────────────────
# Installs the exact generator versions pinned in scripts/bindings/spec.nu with
# `cargo install --locked`. UniFFI's own Kotlin/Swift/Python generator needs no
# installation (it is this repo's `uniffi-bindgen` binary).
#
# Usage:
#   nu scripts/install_bindgens.nu                    # every third-party generator
#   nu scripts/install_bindgens.nu go csharp java     # just these
#   nu scripts/install_bindgens.nu --root ~/.bindgens # install under a prefix
#   nu scripts/install_bindgens.nu --force            # reinstall even if present
#
# With `--root DIR` the binaries land in DIR/bin; export BINDGEN_ROOT=DIR (or
# add DIR/bin to PATH) so generate_bindings.nu finds them.
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *
use bindings/source.nu *

def main [
    ...languages: string
    --root: string   # cargo install --root
    --force          # reinstall even if the tool is already on PATH
] {
    let wanted = if ($languages | is-empty) {
        specs | where kind == external | get language
    } else {
        $languages | each { |l| (spec-for $l).language }
    }

    mut failed = []
    for language in $wanted {
        let spec = (spec-for $language)
        if $spec.kind != "external" {
            print $"($language): built into UniFFI — nothing to install"
            continue
        }
        if (not $force) and (which $spec.tool | is-not-empty) {
            print $"($language): ($spec.tool) already installed — skipping \(use --force to reinstall\)"
            continue
        }
        print $"==> installing ($spec.tool) for ($language)"
        let root_args = if $root == null { [] } else { ["--root" $root] }
        let force_args = if $force { ["--force"] } else { [] }
        let result = if $spec.patch != "" {
            # Needs local fixes: build from the pinned rev with our patch applied.
            let src = (prepare-source $spec)
            do { ^cargo install --locked ...$force_args ...$root_args --path ($src | path join $spec.crate_path) } | complete
        } else {
            do { ^cargo install --locked ...$force_args ...$root_args ...$spec.install } | complete
        }
        if $result.exit_code != 0 {
            print -e $"FAILED: ($language)\n($result.stderr | lines | last 8 | str join (char nl))"
            $failed = ($failed | append $language)
        }
    }

    if ($failed | is-not-empty) {
        error make { msg: $"generator install failed for: ($failed | str join ', ')" }
    }
}
