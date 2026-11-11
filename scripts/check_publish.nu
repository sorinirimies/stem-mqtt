#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — Pre-publish checks
# ──────────────────────────────────────────────────────────────────────────────
# Runs documentation checks, dry-run publish, and cargo check for all crates
# in the workspace before an actual `cargo publish`.
#
# Usage:
#   nu scripts/check_publish.nu
# ──────────────────────────────────────────────────────────────────────────────

def main [] {
    print "══════════════════════════════════════════════════════════"
    print "  stem-mqtt — Pre-publish checks"
    print "══════════════════════════════════════════════════════════"
    print ""

    # ── 1. Documentation checks ───────────────────────────────────────────────
    print "── Step 1: Documentation checks ──"
    let doc_crates = ["stem-mqtt-proto" "stem-mqtt-client" "stem-mqtt-broker" "stem-mqtt-sync"]

    for crate in $doc_crates {
        print $"  📖 Checking docs for ($crate)..."
        let result = (do { cargo doc -p $crate --no-deps } | complete)
        if $result.exit_code != 0 {
            print $"  ❌ Doc check failed for ($crate):"
            print $result.stderr
            exit 1
        }
        print $"  ✅ ($crate) docs OK"
    }
    print ""

    # ── 2. Publish dry-run for mqtt-proto (published first) ───────────────────
    # Only mqtt-proto can be dry-run: the client and the broker depend on crates that are not on
    # crates.io until the one before them has been published.
    print "── Step 2: Publish dry-run (mqtt-proto) ──"
    print "  📦 Running cargo publish --dry-run for mqtt-proto..."
    let publish_result = (do { cargo publish --dry-run -p stem-mqtt-proto } | complete)
    if $publish_result.exit_code != 0 {
        print "  ❌ Publish dry-run failed for mqtt-proto:"
        print $publish_result.stderr
        exit 1
    }
    print "  ✅ mqtt-proto publish dry-run OK"
    print ""

    # ── 3. Cargo check for every crate ────────────────────────────────────────
    print "── Step 3: Cargo check (workspace) ──"
    for crate in $doc_crates {
        print $"  🔍 Running cargo check for ($crate)..."
        let check_result = (do { cargo check -p $crate } | complete)
        if $check_result.exit_code != 0 {
            print $"  ❌ Cargo check failed for ($crate):"
            print $check_result.stderr
            exit 1
        }
        print $"  ✅ ($crate) check OK"
    }
    print ""

    # ── 4. Cargo clippy (workspace) ───────────────────────────────────────────
    print "── Step 4: Cargo clippy (workspace) ──"
    print "  🔎 Running cargo clippy --workspace..."
    let clippy_result = (do { cargo clippy --workspace -- -D warnings } | complete)
    if $clippy_result.exit_code != 0 {
        print "  ❌ Clippy found warnings/errors:"
        print $clippy_result.stderr
        exit 1
    }
    print "  ✅ Clippy passed"
    print ""

    # ── 5. Cargo test (workspace) ─────────────────────────────────────────────
    print "── Step 5: Cargo test (workspace) ──"
    print "  🧪 Running cargo test --workspace..."
    let test_result = (do { cargo test --workspace --exclude stem_mqtt_nif } | complete)
    if $test_result.exit_code != 0 {
        print "  ❌ Tests failed:"
        # cargo prints the failing tests and their panics to stdout.
        print ($test_result.stdout | lines | last 40 | str join "\n")
        print $test_result.stderr
        exit 1
    }
    print "  ✅ All tests passed"
    print ""

    # ── Summary ───────────────────────────────────────────────────────────────
    print "══════════════════════════════════════════════════════════"
    print "  ✅ All pre-publish checks passed!"
    print ""
    print "  Publish order:"
    print "    1. cargo publish -p stem-mqtt-proto    (mqtt-client and mqtt-broker depend on it)"
    print "    2. cargo publish -p stem-mqtt-client   (mqtt-broker depends on it)"
    print "    3. cargo publish -p stem-mqtt-broker"
    print "    4. cargo publish -p stem-mqtt-sync     (its dev-dependencies are the client and the broker)"
    print ""
    print "  Wait ~30 seconds between publishes for crates.io"
    print "  to index each crate before its dependents."
    print "══════════════════════════════════════════════════════════"
}
