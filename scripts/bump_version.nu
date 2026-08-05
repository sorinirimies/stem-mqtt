#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — Bump workspace version
# ──────────────────────────────────────────────────────────────────────────────
# Usage:
#   nu scripts/bump_version.nu <new_version> [--yes]
#
# Example:
#   nu scripts/bump_version.nu 0.2.0 --yes
#
# What it does:
#   1. Validates the supplied semantic version string.
#   2. Updates `workspace.package.version` in the root Cargo.toml.
#   3. Updates the `mqtt-client` internal dependency version wherever it appears.
#   4. Updates the version badge in README.md (if present).
#   5. Runs `cargo fmt`, `cargo clippy`, and `cargo test`.
#   6. Generates / updates the CHANGELOG via git-cliff (if installed).
#   7. Creates a Git commit and an annotated tag.
# ──────────────────────────────────────────────────────────────────────────────

# ── Helpers ───────────────────────────────────────────────────────────────────

# Validate that a string looks like a semver (MAJOR.MINOR.PATCH with optional pre-release).
def validate_version [version: string] {
    let pattern = '^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$'
    if ($version | find --regex $pattern | is-empty) {
        print $"(ansi red)Error:(ansi reset) '($version)' is not a valid semantic version."
        exit 1
    }
}

# Replace the workspace.package.version line in the root Cargo.toml.
def update_workspace_version [version: string] {
    let cargo = (open Cargo.toml --raw)
    let updated = ($cargo | str replace --regex 'version\s*=\s*"[^"]+"' $'version      = "($version)"' )
    $updated | save --force Cargo.toml
    print $"(ansi green)✓(ansi reset) Updated workspace.package.version → ($version)"
}

# Update the mqtt-client internal dependency version in the root Cargo.toml.
def update_internal_dep_version [version: string] {
    let cargo = (open Cargo.toml --raw)
    let lines = ($cargo | lines)
    let updated_lines = ($lines | each {|line|
        if ($line | find --regex '^mqtt-client\s*=' | is-not-empty) {
            $line | str replace --regex 'version\s*=\s*"[^"]+"' $'version = "($version)"'
        } else {
            $line
        }
    })
    $updated_lines | str join "\n" | save --force Cargo.toml
    print $"(ansi green)✓(ansi reset) Updated mqtt-client dependency → ($version)"
}

# Update the crates.io version badge in README.md (if the badge exists).
def update_readme_badge [version: string] {
    if not ("README.md" | path exists) {
        print $"(ansi yellow)⚠(ansi reset) README.md not found — skipping badge update."
        return
    }
    let readme = (open README.md --raw)
    if ($readme =~ 'version-[0-9]+\.[0-9]+\.[0-9]+-blue') {
        let updated = (
            $readme
            | str replace --all --regex 'version-[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+)?-blue' $"version-($version)-blue"
        )
        $updated | save --force README.md
        print $"(ansi green)✓(ansi reset) Updated README.md version badge."
    } else {
        print $"(ansi yellow)⚠(ansi reset) No version badge found in README.md — skipping."
    }
}

# Update crates/mqtt-client-node/package.json's "version" field so the repo
# stays in sync with the workspace version between releases. Not strictly
# required for publishing (release.yml's publish-node job re-sets it via
# `npm version` right before `npm publish`), but leaving it stale in git is
# confusing to anyone browsing the repo.
def update_node_package_version [version: string] {
    let path = "crates/mqtt-client-node/package.json"
    if not ($path | path exists) {
        print $"(ansi yellow)⚠(ansi reset) ($path) not found — skipping."
        return
    }
    let pkg = (open $path --raw)
    let updated = ($pkg | str replace --regex '"version":\s*"[^"]+"' $'"version": "($version)"')
    $updated | save --force $path
    print $"(ansi green)✓(ansi reset) Updated ($path) version → ($version)"
}

# ── Main ──────────────────────────────────────────────────────────────────────

def main [
    new_version: string,  # New version in X.Y.Z format
    --yes (-y),           # Skip confirmation prompt (non-interactive)
] {
    print ""
    print $"(ansi cyan)══════════════════════════════════════════════════════════════(ansi reset)"
    print $"(ansi cyan)  stem-mqtt — Bump Version(ansi reset)"
    print $"(ansi cyan)══════════════════════════════════════════════════════════════(ansi reset)"
    print ""

    let current_version = (open Cargo.toml | get workspace.package.version)
    print $"  Current version : (ansi yellow)($current_version)(ansi reset)"
    print $"  New version     : (ansi green)($new_version)(ansi reset)"
    print ""

    if $current_version == $new_version {
        print $"(ansi yellow)⚠(ansi reset) Version is already ($new_version). Nothing to do."
        exit 0
    }

    validate_version $new_version
    print $"(ansi green)✓(ansi reset) Version string validated."

    update_workspace_version $new_version
    update_internal_dep_version $new_version
    update_readme_badge $new_version
    update_node_package_version $new_version

    print ""
    print $"(ansi cyan)── cargo fmt ───────────────────────────────────────────────(ansi reset)"
    run-external "cargo" "fmt" "--all"
    print $"(ansi green)✓(ansi reset) cargo fmt completed."

    print ""
    print $"(ansi cyan)── cargo clippy ────────────────────────────────────────────(ansi reset)"
    run-external "cargo" "clippy" "--workspace" "--all-targets" "--" "-D" "warnings"
    print $"(ansi green)✓(ansi reset) cargo clippy passed."

    print ""
    print $"(ansi cyan)── cargo test ──────────────────────────────────────────────(ansi reset)"
    run-external "cargo" "test" "--workspace"
    print $"(ansi green)✓(ansi reset) cargo test passed."

    print ""
    print $"(ansi cyan)── cargo update ────────────────────────────────────────────(ansi reset)"
    run-external "cargo" "update" "-p" "stem-mqtt-client" "-p" "stem-mqtt-broker"
    print $"(ansi green)✓(ansi reset) Cargo.lock updated."

    print ""
    print $"(ansi cyan)── changelog ───────────────────────────────────────────────(ansi reset)"
    if (which git-cliff | is-not-empty) {
        run-external "git-cliff" "--output" "CHANGELOG.md" "--tag" $"v($new_version)"
        print $"(ansi green)✓(ansi reset) CHANGELOG.md updated via git-cliff."
    } else {
        print $"(ansi yellow)⚠(ansi reset) git-cliff not found — skipping changelog generation."
    }

    print ""
    print $"(ansi cyan)── git commit & tag ────────────────────────────────────────(ansi reset)"
    run-external "git" "add" "-A"
    run-external "git" "commit" "-m" $"chore: bump version to ($new_version)"
    run-external "git" "tag" "-a" $"v($new_version)" "-m" $"Release v($new_version)"
    print $"(ansi green)✓(ansi reset) Committed and tagged v($new_version)."

    print ""
    print $"(ansi green)══════════════════════════════════════════════════════════════(ansi reset)"
    print $"(ansi green)  stem-mqtt version bumped to ($new_version) 🚀(ansi reset)"
    print $"(ansi green)══════════════════════════════════════════════════════════════(ansi reset)"
    print ""
    print "  Next steps:"
    print $"    git push origin main --tags"
    print ""
}
