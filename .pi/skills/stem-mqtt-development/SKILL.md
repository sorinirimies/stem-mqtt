---
name: stem-mqtt-development
description: Work on the stem-mqtt Rust workspace (MQTT 3.1.1/5.0 client + broker with UniFFI bindings for Kotlin/Swift/Python/Ruby/Node). Use when building features, fixing CI, cutting releases, adding packaging targets, or writing examples/docs in this repo.
---

# stem-mqtt development

## When to Use

Use whenever working in the stem-mqtt repository: implementing/refactoring
MQTT client or broker logic, adding examples, fixing CI workflows, cutting a
release, adding or debugging a language-binding packaging target
(Kotlin/JVM, Kotlin/Android, Swift/iOS, Python, Ruby, Node), working with the
VHS demo GIFs, or navigating the dual GitHub+Gitea git remote setup.

## Quick orientation

- No separate "core" crate: `crates/mqtt-client` hosts both the async
  client and the shared wire-protocol codec (`mqtt_client::protocol`);
  `crates/mqtt-broker` reuses it.
- Client code is split by responsibility under
  `crates/mqtt-client/src/client/{mod,types,inner,io}.rs`. Broker code is
  split into cohesive subsystems under
  `crates/mqtt-broker/src/{retain,registry,events,broker}.rs`. Follow these
  splits when adding functionality — see
  [references/architecture.md](references/architecture.md) for the full
  breakdown before making structural changes.
- Use `justfile` recipes instead of raw cargo/git commands for everything
  (build, test, release, bindings, packaging, multi-remote git). See
  [references/workflow.md](references/workflow.md) for the recipe
  reference, release-safety checklist, dual-remote setup, and VHS demo-GIF
  conventions.
- Adding or touching a packaging target (Kotlin/JVM, Kotlin/Android,
  Swift/iOS, Python, Ruby, Node)? Read
  [references/packaging.md](references/packaging.md) first — it documents
  per-target layout, several already-discovered gotchas (a Gradle
  classloader collision, a missing dependency, an upstream uniffi-rs bug
  blocking `mqtt-broker`'s Kotlin bindings), and the "always build it for
  real before committing" habit that caught them.

## Top pitfalls (see references/ for full detail + context)

- Never background a broker with `cargo run ... &` + a fixed `sleep N`
  before a client connects — poll the port instead
  ([workflow.md](references/workflow.md)).
- `mqtt-broker`'s Kotlin bindings don't compile (upstream uniffi-rs #2392)
  — both Kotlin packaging modules ship `mqtt-client` only
  ([packaging.md](references/packaging.md)).
- Don't nest the Kotlin/Android AGP module inside the JVM module's Gradle
  build — give it its own `settings.gradle.kts`
  ([packaging.md](references/packaging.md)).
- This repo has two git remotes (`origin` = GitHub, `gitea` = mirror) —
  default to GitHub-only unless told otherwise
  ([workflow.md](references/workflow.md)).

## Verification

1. `just check-all` passes (fmt + clippy `-D warnings` + `test --workspace`
   + doc + nu script tests).
2. `cargo test --workspace --locked --all-features --all-targets` shows
   all suites passing, no ignored failures.
3. For any new/changed packaging target: actually run the real build
   command locally and inspect the produced artifact — see
   [references/packaging.md](references/packaging.md).
4. For VHS tape changes: render and inspect multiple extracted frames —
   see [references/workflow.md](references/workflow.md).
5. Before pushing a version tag: `gh run list --branch main --limit 5`
   shows the latest CI run green.
