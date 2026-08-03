# stem-mqtt workspace — task runner
# Install just:      cargo install just
# Install git-cliff: cargo install git-cliff
# Install nu:        cargo install nu --locked
# Usage: just <task>
# ── Default ───────────────────────────────────────────────────────────────────

default:
    @just --list

# ── Tool checks ───────────────────────────────────────────────────────────────

_check-git-cliff:
    @command -v git-cliff >/dev/null 2>&1 || { \
        echo "❌ git-cliff not found. Install with: cargo install git-cliff"; exit 1; \
    }

_check-nu:
    @command -v nu >/dev/null 2>&1 || { \
        echo "❌ nu (nushell) not found. Install: https://www.nushell.sh"; exit 1; \
    }

_check-cross:
    @command -v cross >/dev/null 2>&1 || { \
        echo "❌ cross not found. Install with: cargo install cross --git https://github.com/cross-rs/cross"; exit 1; \
    }

# Install all recommended development tools
install-tools:
    @echo "Installing development tools…"
    @command -v git-cliff >/dev/null 2>&1 || cargo install git-cliff --locked
    @command -v nu >/dev/null 2>&1 || cargo install nu --locked
    @echo "✅ All tools installed!"

# ── Build ─────────────────────────────────────────────────────────────────────

# Build the entire workspace (dev)
build:
    cargo build --workspace

# Build only the client + protocol codec crate (dev)
build-client:
    cargo build -p mqtt-client

# Build only the broker crate + CLI binary (dev)
build-broker:
    cargo build -p mqtt-broker

# Build release binaries (currently just the mqtt-broker CLI)
build-release:
    cargo build --release -p mqtt-broker --bin mqtt-broker

# Build every example in the workspace
build-examples:
    cargo build --workspace --examples

# ── Run ───────────────────────────────────────────────────────────────────────

# Run the standalone mqtt-broker CLI on 0.0.0.0:1883
run-broker:
    cargo run -p mqtt-broker --bin mqtt-broker

# Run an example by name (e.g. `just run-example pub_sub`)
run-example name:
    #!/usr/bin/env sh
    if cargo run -p mqtt-client --example {{ name }} 2>/dev/null; then exit 0; fi
    cargo run -p mqtt-broker --example {{ name }}

# ── Test ──────────────────────────────────────────────────────────────────────

# Run the full workspace test suite
test:
    cargo test --workspace --locked --all-features --all-targets

# Test only the client + protocol codec
test-client:
    cargo test -p mqtt-client --all-features

# Test only the broker (includes the client<->broker integration suite)
test-broker:
    cargo test -p mqtt-broker --all-features

# Run just the client<->broker integration tests
test-integration:
    cargo test -p mqtt-broker --test integration

# Run Nu script tests
test-nu: _check-nu
    nu scripts/tests/run_all.nu

# Run both Rust and Nu tests
test-all-nu: test test-nu
    @echo "✅ All Rust and Nu tests passed!"

# ── Code quality ──────────────────────────────────────────────────────────────

# Check without building
check:
    cargo check --workspace

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy across the workspace
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run all quality checks (format, clippy, test, doc, nu) — must pass before a release.
# Auto-formats first, then verifies no changes remain (catches unstaged format diffs).
check-all: fmt clippy test doc test-nu
    @echo "🔍 Verifying formatting is clean…"
    cargo fmt --all -- --check
    @echo "✅ All checks passed!"

# Full pre-release quality gate — everything in check-all plus a release build.
check-release: check-all build-release
    @echo "✅ Release quality gate passed (fmt + clippy + test + doc + nu + release build)!"

# ── UniFFI bindings ───────────────────────────────────────────────────────────

# Generate bindings for one language, one crate (kotlin | swift | python | ruby)
bindings language crate="mqtt-client":
    ./scripts/generate-bindings.sh {{ language }} {{ crate }}

# Generate Kotlin bindings for both crates
bindings-kotlin:
    ./scripts/generate-bindings.sh kotlin mqtt-client
    ./scripts/generate-bindings.sh kotlin mqtt-broker

# Generate Swift bindings for both crates
bindings-swift:
    ./scripts/generate-bindings.sh swift mqtt-client
    ./scripts/generate-bindings.sh swift mqtt-broker

# Generate Python bindings for both crates
bindings-python:
    ./scripts/generate-bindings.sh python mqtt-client
    ./scripts/generate-bindings.sh python mqtt-broker

# Generate Ruby bindings for both crates (requires uniffi-bindgen-ruby)
bindings-ruby:
    ./scripts/generate-bindings.sh ruby mqtt-client
    ./scripts/generate-bindings.sh ruby mqtt-broker

# Generate bindings for every supported language, both crates
bindings-all: bindings-kotlin bindings-swift bindings-python bindings-ruby
    @echo "✅ All language bindings generated under bindings/"

# ── Packaging (cross-compiled mqtt-broker binaries) ──────────────────────────

# Cross-compile the broker for aarch64 Linux (requires `cross`)
package-linux-aarch64: _check-cross
    cross build --release -p mqtt-broker --bin mqtt-broker --target aarch64-unknown-linux-gnu

# Build a Docker image for the broker (packaging/Dockerfile)
package-docker: build-release
    docker build -t stem-mqtt-broker -f packaging/Dockerfile .

# ── Documentation ─────────────────────────────────────────────────────────────

# Generate and open docs for the client crate
doc-client:
    cargo doc --no-deps -p mqtt-client --open

# Generate and open docs for the broker crate
doc-broker:
    cargo doc --no-deps -p mqtt-broker --open

# Generate docs for the full workspace (no browser)
doc:
    cargo doc --no-deps --workspace --all-features

# ── Changelog ─────────────────────────────────────────────────────────────────

# Regenerate the full CHANGELOG.md from all tags
changelog: _check-git-cliff
    @echo "Generating full changelog…"
    git-cliff --output CHANGELOG.md
    @echo "✅ CHANGELOG.md updated."

# Prepend only unreleased commits to CHANGELOG.md
changelog-unreleased: _check-git-cliff
    git-cliff --unreleased --prepend CHANGELOG.md
    @echo "✅ Unreleased changes prepended."

# Preview changelog for the next release without writing the file
changelog-preview: _check-git-cliff
    @git-cliff --unreleased

# Show the latest tagged release entry (no file write)
changelog-latest: _check-git-cliff
    @git-cliff --latest

# ── Version bump ─────────────────────────────────────────────────────────────

# Validate that a version string will produce a valid vX.Y.Z tag.
validate-tag version: _check-nu
    @nu scripts/ci/validate_tag.nu "v{{ version }}" 2>&1 >/dev/null

# Fail fast if the requested version is the same as the current one.
_check-version-changed version: _check-nu
    #!/usr/bin/env sh
    current=$(nu scripts/version.nu)
    if [ "$current" = "{{ version }}" ]; then
        echo "❌ Version {{ version }} is already the current version. Nothing to bump."
        exit 1
    fi
    echo "✅ Version will change: $current → {{ version }}"

# Bump the workspace version, regenerate Cargo.lock + CHANGELOG.md, commit and tag.
# Validation runs first (cheap), quality gate runs second (expensive).
bump version: (validate-tag version) (_check-version-changed version) check-release _check-git-cliff
    nu scripts/bump_version.nu --yes {{ version }}

# ── Publish (crates.io) ───────────────────────────────────────────────────────

# Run the full pre-publish readiness check (fmt, clippy, tests, docs, dry-run)
check-publish: _check-nu
    nu scripts/check_publish.nu

# Dry-run publish for both crates (in dependency order)
publish-dry: check-all
    @echo "Dry-run: mqtt-client"
    cargo publish --dry-run -p mqtt-client
    @echo "Dry-run: mqtt-broker"
    cargo publish --dry-run -p mqtt-broker

# Publish both crates in dependency order: mqtt-client → mqtt-broker.
publish: check-all publish-client publish-broker
    @echo "✅ mqtt-client and mqtt-broker published to crates.io!"

# Publish mqtt-client (mqtt-broker depends on it — must go first)
publish-client:
    @echo "📦 Publishing mqtt-client…"
    cargo publish -p mqtt-client
    @echo "⏳ Waiting 30 s for the index to propagate…"
    sleep 30

# Publish mqtt-broker
publish-broker:
    @echo "📦 Publishing mqtt-broker…"
    cargo publish -p mqtt-broker

# Show what would be released without making any changes
release-preview: _check-git-cliff
    @echo "Current version: $(just version)"
    @echo ""
    @echo "Unreleased commits:"
    @git-cliff --unreleased
    @echo ""
    @echo "Workspace version:"
    @grep -A5 '^\[workspace\.package\]' Cargo.toml | grep '^version'
    @echo ""
    @echo "Published crates:  mqtt-client  •  mqtt-broker"

# ── Housekeeping ──────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean

# Update all dependencies (Cargo.lock only)
update:
    cargo update

# Update dependencies, run the full quality gate, then commit and push if all green.
update-deps:
    @echo "⬆️  Updating dependencies…"
    cargo update
    @echo "🔍 Running quality gate…"
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --locked --all-features --all-targets
    @echo "✅ All checks passed — committing dependency updates…"
    git add Cargo.lock
    git diff --cached --quiet || git commit -m "chore: update dependencies"
    git push origin main
    @echo "✅ Dependency updates pushed to GitHub."

# Show outdated dependencies (requires cargo-outdated)
outdated:
    cargo outdated

# Show the current workspace version
version: _check-nu
    @nu scripts/version.nu

# Show all configured remotes
remotes:
    @git remote -v

# ── Git remotes & pushing ────────────────────────────────────────────────────

# Push the current branch to GitHub (origin)
push:
    git push origin main

# Push the current branch to Gitea
push-gitea:
    git push gitea main

# Push the current branch to all remotes (continues on failure)
push-all:
    #!/usr/bin/env sh
    failed=""
    git push origin main   || failed="$failed origin"
    git push gitea main    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to push to:$failed"
    else
        echo "✅ Pushed to GitHub and Gitea!"
    fi

# Force-push the current branch to all remotes
push-all-force:
    #!/usr/bin/env sh
    failed=""
    git push --force origin main   || failed="$failed origin"
    git push --force gitea main    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to force-push to:$failed"
    else
        echo "✅ Force-pushed to GitHub and Gitea!"
    fi

# Pull the current branch from GitHub (origin)
pull:
    git pull origin main

# Pull the current branch from Gitea
pull-gitea:
    git pull gitea main

# Pull the current branch from all remotes (continues on failure)
pull-all:
    #!/usr/bin/env sh
    failed=""
    git pull origin main   || failed="$failed origin"
    git pull gitea main    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to pull from:$failed"
    else
        echo "✅ Pulled from GitHub and Gitea!"
    fi

# Push all tags to GitHub
push-tags:
    git push origin --tags

# Push all tags to all remotes (continues on failure)
push-tags-all:
    #!/usr/bin/env sh
    failed=""
    git push origin --tags   || failed="$failed origin"
    git push gitea --tags    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to push tags to:$failed"
    else
        echo "✅ Tags pushed to all remotes!"
    fi

# ── Release workflows ─────────────────────────────────────────────────────────

# Bump, commit, tag, then push to GitHub — triggers the Release workflow.
release version: (bump version)
    @echo "Pushing release v{{ version }} to GitHub…"
    git push --follow-tags origin main
    @echo "✅ Release v{{ version }} pushed — Release workflow will trigger automatically."
    @echo "   https://github.com/$(git remote get-url origin | sed 's/.*github.com[:/]//' | sed 's/\.git//')/actions"

# Bump, commit, tag, then push to Gitea only.
release-gitea version: (bump version)
    @echo "Pushing release v{{ version }} to Gitea…"
    git push --follow-tags gitea main
    @echo "✅ Release v{{ version }} live on Gitea."

# Bump, commit, tag, then push to all remotes (continues on failure).
release-all version: (bump version)
    #!/usr/bin/env sh
    echo "Pushing release v{{ version }} to all remotes…"
    failed=""
    git push --follow-tags origin main   || failed="$failed origin"
    git push --follow-tags gitea main    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Release v{{ version }} failed to push to:$failed"
    else
        echo "✅ Release v{{ version }} pushed to GitHub and Gitea!"
    fi

# Push the latest commit and all tags to every remote (no bump, continues on failure).
push-release-all: check-all
    #!/usr/bin/env sh
    failed=""
    git push --follow-tags origin main   || failed="$failed origin"
    git push --follow-tags gitea main    || failed="$failed gitea"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to push to:$failed"
    else
        echo "✅ Latest commit + tags pushed to all remotes."
    fi

# Manually re-trigger the Release workflow for an existing tag via the gh CLI.
release-retrigger version:
    @command -v gh >/dev/null 2>&1 || { \
        echo "❌ GitHub CLI (gh) not found. Install from https://cli.github.com"; exit 1; \
    }
    @echo "Manually dispatching Release workflow for tag v{{ version }}…"
    gh workflow run release.yml --field tag=v{{ version }}
    @echo "✅ Dispatched — check progress at: https://github.com/$(gh repo view --json nameWithOwner -q .nameWithOwner)/actions"

# Force-sync Gitea with GitHub
sync-gitea:
    git push gitea main --force
    git push gitea --tags --force
    @echo "✅ Gitea force-synced with GitHub."

# Add a Gitea remote and optionally push — interactive (nu script)
setup-gitea url: _check-nu
    nu scripts/setup_gitea.nu {{ url }}

# Migrate this project to dual GitHub + Gitea hosting (interactive)
migrate-gitea: _check-nu
    nu scripts/migrate_to_gitea.nu
