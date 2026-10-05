#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — are the pinned binding generators out of date?
# ──────────────────────────────────────────────────────────────────────────────
# Compares every third-party generator's pin in scripts/bindings/spec.nu with its
# upstream (newest git tag, branch head, or crates.io release) and reports which
# have moved on — and, for tag-pinned ones, which UniFFI release the newer tag
# targets, since bumping a generator usually means bumping the workspace's UniFFI.
#
# Usage:
#   nu scripts/check_bindgen_updates.nu                  # report
#   nu scripts/check_bindgen_updates.nu --fail-on-update # exit 1 if anything is stale
#
# Run weekly by CI (scheduled run) so drift is noticed before it breaks a build.
# ──────────────────────────────────────────────────────────────────────────────

use bindings/spec.nu *
use bindings/updates.nu *

def main [--fail-on-update] {
    let workspace_uniffi = (workspace-uniffi)
    let rows = (specs | where kind == external | each { |spec|
        let pin = (pin-of $spec)
        let latest = (latest-upstream $pin)
        let stale = ($latest != "" and (if $pin.kind == "rev" { $latest != $pin.value } else { is-newer $pin.value $latest }))
        {
            language: $spec.language
            pinned: ($pin.value | str substring 0..16)
            latest: (if $latest == "" { "(lookup failed)" } else { $latest | str substring 0..16 })
            targets_uniffi: (if $pin.kind == "tag" { uniffi-target $latest } else { "" })
            status: (if $latest == "" { "unknown" } else if $stale { "UPDATE AVAILABLE" } else { "current" })
        }
    })
    print $"workspace uniffi: ($workspace_uniffi)"
    print $rows

    let stale = ($rows | where status == "UPDATE AVAILABLE")
    if ($stale | is-not-empty) {
        print $"\n($stale | length) generator\(s\) have newer upstream releases: ($stale | get language | str join ', ')"
        print "Bump the pin in scripts/bindings/spec.nu, then run: just test-bindings"
        if $fail_on_update { exit 1 }
    }
}
