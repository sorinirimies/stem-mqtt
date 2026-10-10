# Development (maintainers)

Build, test, CI and release workflow for working on this repository. Users don't need any of this — see the
[README](../README.md) and the [language guides](languages/README.md). Publishing is in [publishing.md](publishing.md).

Every task below is also a [`just`](https://github.com/casey/just) recipe —
run `just --list` for the full set (bindings, packaging, changelog, version
bumps, releases, multi-remote git push/pull, ...):

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Or run everything CI runs in one shot:

```sh
just check-all      # fmt + clippy + test + doc + nu script tests
./scripts/check.sh  # equivalent, no `just`/`nu` required
```

## Bindings, packages and CI

```sh
just install-bindgens          # the pinned third-party generators (scripts/bindings/spec.nu)
just test-bindings             # runtime-test every language (or: just test-bindings go java)
just package-verify-all        # compile + pack every publishable package, no upload
just ci-bindings               # all of the above, strict — what the Gitea CI runs
just clean-bindings            # remove generated artifacts and scratch dirs
just check-bindgen-updates     # are the pinned generators behind their upstream? (weekly in CI)
just ci-image                  # CI image with every toolchain + generator pre-installed
```

CI runs on Gitea (`.gitea/workflows/ci.yml`, Linux runner; also mirrored in
`.github/workflows/ci.yml`): fmt, clippy, test, doc, the nu tests, then one
`test-bindings-runtime` job per language, the Kotlin Gradle test, and one `test-packaging` job per
package. A weekly scheduled run catches upstream generator drift.

## Nushell scripts

Deterministic release/CI plumbing (tag validation, version bump, changelog,
crates.io publish, quality gate) lives in `scripts/*.nu` as tested Nushell
scripts rather than ad-hoc shell in CI YAML — see
[`scripts/tests/`](../scripts/tests) (`nu scripts/tests/run_all.nu`, also
reachable as `just test-nu`).

## CI/CD

- **Gitea** (`.gitea/workflows/`) — the Linux runner that does the real work: `ci.yml` (fmt, clippy, tests,
  docs, nu tests, a runtime test and a package build per language, toolchain setup retried once on network
  flakes), `release.yml` (on a `vX.Y.Z` tag: broker binaries for Linux and Windows, Gitea release, then crates.io
  and every registry above), `deps-update.yml`.
- **GitHub** (`.github/workflows/`) — mirrors CI and adds what needs GitHub-hosted runners: macOS/iOS/Android
  builds, the Swift package, GitHub Packages, PyPI wheels for several platforms, and `publish-pubdev.yml`.

## Release process

```sh
just release <version>   # preflight + bump + quality gate + tag + push
```

See the [`justfile`](../justfile) for the multi-remote (GitHub + Gitea) workflow: `just bump`, `just release-all`,
`just sync-gitea`, `just migrate-gitea`, …
