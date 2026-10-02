# stem-mqtt workspace — task runner
# Install just:      cargo install just
# Install vhs:       brew install vhs  OR  go install github.com/charmbracelet/vhs@latest
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

_check-cargo-ndk:
    @command -v cargo-ndk >/dev/null 2>&1 || { \
        echo "❌ cargo-ndk not found. Install with: cargo install cargo-ndk --locked"; exit 1; \
    }
    @[ -n "${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}" ] || { \
        echo "❌ ANDROID_NDK_HOME (or ANDROID_NDK_ROOT) not set."; \
        echo "   macOS: brew install --cask android-ndk && export ANDROID_NDK_HOME=/opt/homebrew/share/android-ndk"; \
        exit 1; \
    }

_check-vhs:
    @command -v vhs >/dev/null 2>&1 || { \
        echo "❌ vhs not found."; \
        echo "   macOS:      brew install vhs"; \
        echo "   Any:        go install github.com/charmbracelet/vhs@latest"; \
        exit 1; \
    }

_check-topcoat:
    @command -v topcoat >/dev/null 2>&1 || { \
        echo "❌ topcoat CLI not found. Install with: cargo install topcoat-cli"; exit 1; \
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
    cargo build -p stem-mqtt-client

# Build only the broker crate + CLI binary (dev)
build-broker:
    cargo build -p stem-mqtt-broker

# Build release binaries (currently just the mqtt-broker CLI)
build-release:
    cargo build --release -p stem-mqtt-broker --bin mqtt-broker

# Build every example in the workspace
build-examples:
    cargo build --workspace --examples

# ── Run ───────────────────────────────────────────────────────────────────────

# Run the standalone mqtt-broker CLI on 0.0.0.0:1883
run-broker:
    cargo run -p stem-mqtt-broker --bin mqtt-broker

# Run an example by name (e.g. `just run-example pub_sub`)
run-example name:
    #!/usr/bin/env sh
    if cargo run -p stem-mqtt-client --example {{ name }} 2>/dev/null; then exit 0; fi
    cargo run -p stem-mqtt-broker --example {{ name }}

# ── Test ──────────────────────────────────────────────────────────────────────

# Run the full workspace test suite
test:
    cargo test --workspace --locked --all-features --all-targets

# Test only the client + protocol codec
test-client:
    cargo test -p stem-mqtt-client --all-features

# Test only the broker (includes the client<->broker integration suite)
test-broker:
    cargo test -p stem-mqtt-broker --all-features

# Run just the client<->broker integration tests
test-integration:
    cargo test -p stem-mqtt-broker --test integration

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

# Generate bindings for one language, one crate.
# language: kotlin swift python go csharp java dart node node-livekit haskell
bindings language crate="mqtt-client":
    nu scripts/generate_bindings.nu {{ language }} {{ crate }}

# Generate one language's bindings for BOTH crates
bindings-lang language:
    nu scripts/generate_bindings.nu {{ language }} mqtt-client
    nu scripts/generate_bindings.nu {{ language }} mqtt-broker

# Generate Kotlin bindings for both crates
bindings-kotlin: (bindings-lang "kotlin")

# Generate Swift bindings for both crates
bindings-swift: (bindings-lang "swift")

# Generate Python bindings for both crates
bindings-python: (bindings-lang "python")

# Generate bindings for every officially supported UniFFI language
bindings-all: bindings-kotlin bindings-swift bindings-python
    @echo "✅ Kotlin, Swift, and Python bindings generated under bindings/"

# Install the pinned third-party generators (Go, C#, Java, Dart, Node, Haskell)
install-bindgens *languages:
    nu scripts/install_bindgens.nu {{ languages }}

# Generate Go, C#, Java, Dart, Haskell + experimental Node bindings (see scripts/bindings/spec.nu for the Node broker caveat)
bindings-third-party: (bindings-lang "go") (bindings-lang "csharp") (bindings-lang "java") (bindings-lang "dart") (bindings-lang "haskell") (bindings-lang "node-livekit")
    nu scripts/generate_bindings.nu node mqtt-client
    @echo "✅ Go, C#, Java, Dart, Node, Haskell bindings generated under bindings/"

# Runtime-test the generated bindings (real broker + clients through each language). `just test-bindings go java`
test-bindings *languages:
    nu scripts/test_bindings.nu {{ languages }}

# Run what the Gitea CI runs for bindings (needs go, dotnet, JDK 22+, dart, node, cabal on PATH)
ci-bindings: install-bindgens
    nu scripts/test_bindings.nu --strict
    just package-verify-all

# Build (compile + pack) one language's package WITHOUT publishing it
package-verify language:
    nu scripts/publish_packages.nu verify {{ language }}

# Build every publishable language's package (java csharp node go dart haskell)
package-verify-all:
    #!/usr/bin/env sh
    set -eu
    for lang in java csharp node go dart haskell; do
        nu scripts/publish_packages.nu verify "$lang"
    done
    echo "✅ every package builds"

# Remove generated bindings, staged packages and binding-test scratch (keeps the Rust cache)
clean-bindings:
    rm -rf bindings dist target/bindings-test target/bindgen-src target/uniffi-0.30.0 target/uniffi-0.32.0
    rm -rf packaging/kotlin/staged packaging/kotlin/android/staged-jniLibs packaging/swift/.build packaging/swift/dist
    @echo "🧹 generated binding artifacts removed"

# Stage one language's package (bindings + this host's native libs) under dist/
package-stage language:
    nu scripts/publish_packages.nu stage {{ language }}

# Show what publishing a staged language to GitHub Packages would run
publish-dry-run language version:
    nu scripts/publish_packages.nu publish {{ language }} {{ version }} --dry-run

# ── Node.js / TypeScript (napi-rs, not a UniFFI target) ────────────────

# Install npm deps for the Node addon
node-install:
    cd crates/mqtt-client-node && npm ci

# Build the Node addon (fast, dev profile, current platform only)
build-node: node-install
    cd crates/mqtt-client-node && npm run build:debug

# Build the Node addon (release profile, current platform)
build-node-release: node-install
    cd crates/mqtt-client-node && npm run build

# Run the Node smoke test against a broker already listening on :18830
test-node: build-node
    cd crates/mqtt-client-node && node scripts/smoke_test.mjs

# ── Packaging (cross-compiled mqtt-broker binaries) ──────────────────────────

# Cross-compile the broker for aarch64 Linux (requires `cross`)
package-linux-aarch64: _check-cross
    cross build --release -p stem-mqtt-broker --bin mqtt-broker --target aarch64-unknown-linux-gnu

# Stage the Kotlin/JVM client+broker package (generates bindings + builds
# release cdylibs + stages packaging/kotlin/staged/) — run `gradle build` in
# packaging/kotlin/ afterwards to build the jar.
package-kotlin-jvm:
    nu scripts/generate_bindings.nu kotlin mqtt-client
    nu scripts/generate_bindings.nu kotlin mqtt-broker
    ./packaging/kotlin/stage.sh 0.0.0-dev

# Cross-compile client+broker for every Android ABI (arm64-v8a/armeabi-v7a/
# x86_64/x86), stage generated Kotlin sources + .so files, then run Gradle
# under packaging/kotlin/android/ to build the AAR.
package-kotlin-android: _check-cargo-ndk
    nu scripts/generate_bindings.nu kotlin mqtt-client
    nu scripts/generate_bindings.nu kotlin mqtt-broker
    ./packaging/kotlin/stage.sh 0.0.0-dev
    ./packaging/kotlin/stage-android.sh

# Build a Docker image for the broker (multi-stage packaging/Dockerfile —
# builds mqtt-broker from source inside Docker, no host build needed)
package-docker:
    docker build -t stem-mqtt-broker:local -f packaging/Dockerfile .

# ── VHS Demo GIFs ─────────────────────────────────────────────────────────────

VHS_DIR := "examples/vhs"
VHS_GENERATED := "examples/vhs/generated"

# Generate all VHS demo GIFs (broker CLI + client examples)
vhs-all: _check-vhs
    #!/usr/bin/env sh
    set -e
    mkdir -p {{ VHS_GENERATED }}
    echo "╔════════════════════════════════════════════╗"
    echo "║   stem-mqtt Tapes (broker CLI + examples) ║"
    echo "╚════════════════════════════════════════════╝"
    for tape in {{ VHS_DIR }}/*.tape; do
        [ -f "$tape" ] || continue
        echo "▶  $tape"
        vhs "$tape" || echo "❌ Failed: $tape"
    done
    echo "✅ Demos done → {{ VHS_GENERATED }}/"

# Render a single tape by name (e.g. just vhs-tape pub-sub-demo)
vhs-tape name: _check-vhs
    #!/usr/bin/env sh
    if [ -f "{{ VHS_DIR }}/{{ name }}.tape" ]; then
        echo "▶  {{ VHS_DIR }}/{{ name }}.tape"
        vhs "{{ VHS_DIR }}/{{ name }}.tape" && echo "✅ Done."
    else
        echo "❌ Tape not found: {{ name }}.tape"
        echo ""
        just vhs-list
        exit 1
    fi

# List all available VHS tapes
vhs-list:
    #!/usr/bin/env sh
    echo "Tapes  →  {{ VHS_DIR }}/"
    ls {{ VHS_DIR }}/*.tape 2>/dev/null | sed 's|.*/||; s|\.tape||' | sed 's/^/  /' || echo "  (none)"

# Build the demo webpage's Docker image (nginx serving demo/web/ statically)
package-docker-demo-web:
    docker build -t stem-mqtt-demo-web:local -f demo/web/Dockerfile .

# ── Demo (browser client + Docker + Kubernetes) ──────────────────

# Run the broker with WebSocket support + a static file server for the
# demo webpage, using whatever's already installed (no Docker required)
demo-run:
    #!/usr/bin/env sh
    set -e
    echo "Broker (TCP :1883, WebSocket :8083) + demo webpage (:8090) — Ctrl-C to stop both"
    trap 'kill 0' EXIT
    cargo run -p stem-mqtt-broker --bin mqtt-broker -- --ws-port 8083 &
    (cd demo/web && python3 -m http.server 8090) &
    wait

# Run the broker (TCP :1883) + the Topcoat dashboard dev server (:3000)
# together. `topcoat dev` builds the dashboard, bundles its client-runtime
# assets, starts it, and live-reloads on source changes. The dashboard's
# "New connection" form defaults to 127.0.0.1:1883, matching this broker.
# Ctrl-C stops both.
demo-dashboard: _check-topcoat
    #!/usr/bin/env sh
    set -e
    echo "Broker (TCP :1883) + Topcoat dashboard (http://127.0.0.1:3000) — Ctrl-C to stop both"
    trap 'kill 0' EXIT
    cargo run -p stem-mqtt-broker --bin mqtt-broker &
    topcoat dev --package stem-mqtt-dashboard &
    wait

# Build and run the full demo (broker + demo webpage) via Docker Compose
demo-docker:
    docker compose up --build

# Tear down the Docker Compose demo
demo-docker-down:
    docker compose down

# ── Documentation ─────────────────────────────────────────────────────────────

# Generate and open docs for the client crate
doc-client:
    cargo doc --no-deps -p stem-mqtt-client --open

# Generate and open docs for the broker crate
doc-broker:
    cargo doc --no-deps -p stem-mqtt-broker --open

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
    cargo publish --dry-run -p stem-mqtt-client
    @echo "Dry-run: mqtt-broker"
    cargo publish --dry-run -p stem-mqtt-broker

# Publish both crates in dependency order: mqtt-client → mqtt-broker.
publish: check-all publish-client publish-broker
    @echo "✅ mqtt-client and mqtt-broker published to crates.io!"

# Publish mqtt-client (mqtt-broker depends on it — must go first)
publish-client:
    @echo "📦 Publishing mqtt-client…"
    cargo publish -p stem-mqtt-client
    @echo "⏳ Waiting 30 s for the index to propagate…"
    sleep 30

# Publish mqtt-broker
publish-broker:
    @echo "📦 Publishing mqtt-broker…"
    cargo publish -p stem-mqtt-broker

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

# Push the current branch to Gitea Microlab
push-gitea-microlab:
    git push gitea-microlab main

# Push the current branch to Gitea (nexus-lab instance)
push-gitea-nexus-lab:
    git push gitea-nexus-lab main

# Push the current branch to Gitea Starscream
push-gitea-starscream:
    git push gitea-starscream main

# Push the current branch to all remotes (continues on failure)
push-all:
    #!/usr/bin/env sh
    failed=""
    git push origin main             || failed="$failed origin"
    git push gitea-microlab main              || failed="$failed gitea-microlab"
    git push gitea-starscream main   || failed="$failed gitea-starscream"
    git push gitea-nexus-lab main    || failed="$failed gitea-nexus-lab"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to push to:$failed"
    else
        echo "✅ Pushed to GitHub, Gitea Microlab, Gitea Starscream, and Gitea (nexus-lab)!"
    fi

# Force-push the current branch to all remotes
push-all-force:
    #!/usr/bin/env sh
    failed=""
    git push --force origin main             || failed="$failed origin"
    git push --force gitea-microlab main              || failed="$failed gitea-microlab"
    git push --force gitea-starscream main   || failed="$failed gitea-starscream"
    git push --force gitea-nexus-lab main    || failed="$failed gitea-nexus-lab"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to force-push to:$failed"
    else
        echo "✅ Force-pushed to GitHub, Gitea Microlab, Gitea Starscream, and Gitea (nexus-lab)!"
    fi

# Pull the current branch from GitHub (origin)
pull:
    git pull origin main

# Pull the current branch from Gitea Microlab
pull-gitea-microlab:
    git pull gitea-microlab main

# Pull the current branch from Gitea (nexus-lab instance)
pull-gitea-nexus-lab:
    git pull gitea-nexus-lab main

# Pull the current branch from Gitea Starscream
pull-gitea-starscream:
    git pull gitea-starscream main

# Pull the current branch from all remotes (continues on failure)
pull-all:
    #!/usr/bin/env sh
    failed=""
    git pull origin main             || failed="$failed origin"
    git pull gitea-microlab main              || failed="$failed gitea-microlab"
    git pull gitea-starscream main   || failed="$failed gitea-starscream"
    git pull gitea-nexus-lab main    || failed="$failed gitea-nexus-lab"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to pull from:$failed"
    else
        echo "✅ Pulled from GitHub, Gitea Microlab, Gitea Starscream, and Gitea (nexus-lab)!"
    fi

# Push all tags to GitHub
push-tags:
    git push origin --tags

# Push all tags to all remotes (continues on failure)
push-tags-all:
    #!/usr/bin/env sh
    failed=""
    git push origin --tags             || failed="$failed origin"
    git push gitea-microlab --tags              || failed="$failed gitea-microlab"
    git push gitea-starscream --tags   || failed="$failed gitea-starscream"
    git push gitea-nexus-lab --tags    || failed="$failed gitea-nexus-lab"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to push tags to:$failed"
    else
        echo "✅ Tags pushed to all remotes!"
    fi

# ── Release workflows ─────────────────────────────────────────────────────────

# Verify remote CI + required registry credentials before creating a release tag.
release-preflight:
    #!/usr/bin/env sh
    set -eu
    [ "$(git branch --show-current)" = "main" ] || { echo "❌ Releases must run from main."; exit 1; }
    [ -z "$(git status --porcelain)" ] || { echo "❌ Working tree is dirty."; exit 1; }
    command -v gh >/dev/null 2>&1 || { echo "❌ gh CLI not found."; exit 1; }
    for secret in CRATES_IO_TOKEN NPM_TOKEN PYPI_API_TOKEN; do
        gh secret list --json name --jq '.[].name' | rg -qx "$secret" || {
            echo "❌ Required GitHub secret missing: $secret"; exit 1;
        }
    done
    conclusion=$(gh run list --workflow CI --commit "$(git rev-parse HEAD)" --limit 1 --json conclusion --jq '.[0].conclusion // "missing"')
    [ "$conclusion" = "success" ] || {
        echo "❌ Latest CI for HEAD is '$conclusion' (billing/runner failures also block release)."; exit 1;
    }
    echo "✅ Release preflight passed."

# Prepare an unpushed annotated tag. Supports either bumping from an older
# version or tagging an already-prepared version commit.
_prepare-release-tag version: (validate-tag version)
    #!/usr/bin/env sh
    set -eu
    tag="v{{ version }}"
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        echo "❌ Tag $tag already exists."
        exit 1
    fi
    current=$(just version)
    if [ "$current" != "{{ version }}" ]; then
        just bump "{{ version }}"
    else
        just check-release
        [ -z "$(git status --porcelain)" ] || {
            echo "❌ Quality gate changed tracked files; commit them before tagging."
            exit 1
        }
        git tag -a "$tag" -m "Release $tag"
    fi

# Prepare, tag, then push to GitHub — triggers the Release workflow.
release version:
    just release-preflight
    just _prepare-release-tag "{{ version }}"
    @echo "Pushing release v{{ version }} to GitHub…"
    git push --follow-tags origin main
    @echo "✅ Release v{{ version }} pushed — Release workflow will trigger automatically."
    @echo "   https://github.com/$(git remote get-url origin | sed 's/.*github.com[:/]//' | sed 's/\.git//')/actions"

# Prepare, tag, then push to Gitea Microlab only.
release-gitea-microlab version:
    just _prepare-release-tag "{{ version }}"
    @echo "Pushing release v{{ version }} to Gitea Microlab…"
    git push --follow-tags gitea-microlab main
    @echo "✅ Release v{{ version }} live on Gitea Microlab."

# Prepare, tag, then push to Gitea (nexus-lab instance) only.
release-gitea-nexus-lab version:
    just _prepare-release-tag "{{ version }}"
    @echo "Pushing release v{{ version }} to Gitea (nexus-lab)…"
    git push --follow-tags gitea-nexus-lab main
    @echo "✅ Release v{{ version }} live on Gitea (nexus-lab)."

# Prepare, tag, then push to Gitea Starscream only.
release-gitea-starscream version:
    just _prepare-release-tag "{{ version }}"
    @echo "Pushing release v{{ version }} to Gitea Starscream…"
    git push --follow-tags gitea-starscream main
    @echo "✅ Release v{{ version }} live on Gitea Starscream."

# Prepare, tag, then push to all remotes (continues on failure).
release-all version:
    #!/usr/bin/env sh
    set -eu
    just release-preflight
    just _prepare-release-tag "{{ version }}"
    echo "Pushing release v{{ version }} to all remotes…"
    failed=""
    git push --follow-tags origin main             || failed="$failed origin"
    git push --follow-tags gitea-microlab main              || failed="$failed gitea-microlab"
    git push --follow-tags gitea-starscream main   || failed="$failed gitea-starscream"
    git push --follow-tags gitea-nexus-lab main    || failed="$failed gitea-nexus-lab"
    if [ -n "$failed" ]; then
        echo "⚠️  Release v{{ version }} failed to push to:$failed"
    else
        echo "✅ Release v{{ version }} pushed to GitHub, Gitea Microlab, Gitea Starscream, and Gitea (nexus-lab)!"
    fi

# Push the latest commit and all tags to every remote (no bump, continues on failure).
push-release-all: check-all
    #!/usr/bin/env sh
    failed=""
    git push --follow-tags origin main             || failed="$failed origin"
    git push --follow-tags gitea-microlab main              || failed="$failed gitea-microlab"
    git push --follow-tags gitea-starscream main   || failed="$failed gitea-starscream"
    git push --follow-tags gitea-nexus-lab main    || failed="$failed gitea-nexus-lab"
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

# Force-sync Gitea Microlab with GitHub
sync-gitea-microlab:
    git push gitea-microlab main --force
    git push gitea-microlab --tags --force
    @echo "✅ Gitea Microlab force-synced with GitHub."

# Force-sync Gitea (nexus-lab instance) with GitHub
sync-gitea-nexus-lab:
    git push gitea-nexus-lab main --force
    git push gitea-nexus-lab --tags --force
    @echo "✅ Gitea (nexus-lab) force-synced with GitHub."

# Force-sync Gitea Starscream with GitHub
sync-gitea-starscream:
    git push gitea-starscream main --force
    git push gitea-starscream --tags --force
    @echo "✅ Gitea Starscream force-synced with GitHub."

# Force-sync all Gitea instances with GitHub (continues on failure)
sync-all-gitea:
    #!/usr/bin/env sh
    failed=""
    git push gitea-microlab main --force                  || failed="$failed gitea-microlab"
    git push gitea-microlab --tags --force                || failed="$failed gitea-microlab-tags"
    git push gitea-starscream main --force       || failed="$failed gitea-starscream"
    git push gitea-starscream --tags --force     || failed="$failed gitea-starscream-tags"
    git push gitea-nexus-lab main --force        || failed="$failed gitea-nexus-lab"
    git push gitea-nexus-lab --tags --force      || failed="$failed gitea-nexus-lab-tags"
    if [ -n "$failed" ]; then
        echo "⚠️  Failed to sync:$failed"
    else
        echo "✅ All Gitea instances force-synced with GitHub."
    fi

# Add a Gitea remote and optionally push — interactive (nu script)
setup-gitea url: _check-nu
    nu scripts/setup_gitea.nu {{ url }}

# Migrate this project to dual GitHub + Gitea hosting (interactive)
migrate-gitea: _check-nu
    nu scripts/migrate_to_gitea.nu
