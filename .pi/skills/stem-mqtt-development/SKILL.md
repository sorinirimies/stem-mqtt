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

## Architecture

- `crates/mqtt-client` hosts the async `tokio` client **and** the shared
  wire-protocol codec (`mqtt_client::protocol`) used by both the client and
  the broker — there is intentionally no separate "core" crate.
  `crates/mqtt-broker` reuses that codec instead of duplicating it.
  `crates/mqtt-client-node` is a hand-written napi-rs addon (not a UniFFI
  target, since JS isn't one).
- Client internals live in `crates/mqtt-client/src/client/{mod,types,inner,io}.rs`:
  - `types.rs` — pure data (`ConnectOptions`, `MqttMessage`, listener trait)
  - `inner.rs` — live-connection state (`Inner`) + pending-ack/QoS-2 bookkeeping
  - `io.rs` — wire I/O (read loop, keep-alive loop, encode/write)
  - `mod.rs` — public `MqttClient` API only

  Follow this split when adding client functionality — don't dump
  everything back into one file.
- Broker internals live in
  `crates/mqtt-broker/src/{retain,registry,events,broker,connection,session,topic,ws}.rs`:
  - `retain.rs` — `RetainStore` (retained-message store only)
  - `registry.rs` — `SessionRegistry` (session map, send/queue/QoS-2
    bookkeeping, fan-out)
  - `events.rs` — `EventHub` (listener notifications)
  - `broker.rs` — `BrokerState` composing the three subsystems, plus only
    genuinely cross-cutting orchestration (`detach_session`'s will
    handling, `send_matching_retained`)

  `connection.rs` calls `state.sessions.x()` / `state.retained.x()` /
  `state.events.x()` directly for single-subsystem ops — only add a
  `BrokerState` method when an operation genuinely spans more than one
  subsystem.

## Procedure

1. Always use `justfile` recipes instead of raw cargo/git commands — it
   already covers nearly every workflow: `just build`/`test`/`check-all`/
   `check-release`, `just bump <version>` / `release <version>`
   (bump+commit+tag+push, triggers the Release workflow),
   `just release-retrigger <version>`, `just push`/`push-all`/`pull-all`
   (GitHub `origin` + Gitea `gitea` remotes), `just push-tags`,
   `just publish` (crates.io), `just version`, `just vhs-all`/
   `vhs-tape <name>`/`vhs-list` (VHS demo GIFs under `examples/vhs/`),
   `just package-kotlin-jvm`/`package-kotlin-android`, `just bindings-kotlin`/
   `bindings-swift`/`bindings-python`/`bindings-ruby`. Run `just --list` if
   unsure a recipe exists before writing a manual command.
2. Before tagging/publishing a real release: confirm the target version
   matches what's already in `Cargo.toml`/`package.json` (bump if not),
   confirm `CRATES_IO_TOKEN`/`NPM_TOKEN` etc. secrets are actually
   configured (`gh secret list`) since publishing is irreversible
   (crates.io = yank only, npm unpublish restricted after 72h) — always get
   explicit user confirmation before pushing a version tag.
3. VHS demo tapes live in `examples/vhs/*.tape`, rendered GIFs in
   `examples/vhs/generated/` which **are** committed via git-lfs (`*.gif`
   tracked in `.gitattributes`) and embedded in `README.md`'s `## Preview`
   section — not gitignored. When a tape backgrounds a process (e.g. the
   broker) in the same terminal: redirect its stdout/stderr to `/dev/null`
   (its tracing logs otherwise interleave with the foreground command's
   output) and run `set +m` first to disable job-control notifications
   (otherwise `[1] <pid>` and `[1]+ Exit/Done ...` lines land in the
   recording). Use `Hide` / `Type "clear"` / `Enter` / `Show` to scrub any
   leftover typed-command text from the visible recording before the real
   demo starts.
4. This repo has **two** git remotes: `origin` (GitHub, primary) and
   `gitea` (self-hosted mirror). Default to `just push` (GitHub only)
   unless asked to sync both (`just push-all`) or Gitea specifically.
5. Packaging conventions — one crate/language-target's build lives under
   `packaging/<lang>/` with its own README documenting caveats:
   - `packaging/kotlin` — JVM jar, JNA resource-embedded native lib
   - `packaging/kotlin/android` — real AAR, jniLibs, cargo-ndk
     cross-compiled, its **own independent** Gradle build (do NOT make it a
     subproject of the JVM module's build, see Pitfalls)
   - `packaging/swift` — XCFramework: macOS + iOS device + iOS simulator
     slices via `cargo build --target aarch64-apple-ios` /
     `aarch64-apple-ios-sim` / `x86_64-apple-ios`, `lipo`'d where needed
   - `packaging/python` — maturin
   - `packaging/ruby` — uniffi-bindgen-ruby
   - `packaging/node` — napi-rs prebuilds
6. Both Kotlin packaging modules (JVM + Android) currently ship
   `mqtt-client` only, not `mqtt-broker` — see Pitfalls for why. When
   adding a new packaged language target, always try to actually build+run
   it locally (not just write plausible-looking config) before committing;
   this repo's existing Kotlin/Swift packaging had config bugs that were
   never build-tested until an actual local build caught them.

## Pitfalls

- **Broker CI flakiness**: never background a broker with
  `cargo run ... &` followed by a fixed `sleep N` before a client connects
  — first-compile time varies and a short sleep races it, causing
  "Connection refused". Pre-build the binary separately, then poll the
  port until it's actually listening.
- **mqtt-broker's Kotlin bindings hit a real upstream uniffi-rs bug**
  ([External Kotlin errors don't work, #2392](https://github.com/mozilla/uniffi-rs/issues/2392)):
  `MqttBroker::start`/`stop` return `mqtt_client::MqttError`, which is
  "external" to the `mqtt-broker` crate being bound — Kotlin bindgen
  silently drops those two methods from the generated class (emits a
  `// Sorry, the callable "start" isn't supported.` comment instead),
  leaving `MqttBroker` unable to compile. Workaround until fixed upstream
  (or until `mqtt-broker` gets its own local error type): only package
  `mqtt-client`'s Kotlin bindings, not `mqtt-broker`'s.
- **Missing kotlinx-coroutines-core**: any UniFFI Kotlin module exposing
  async-exported methods (`#[uniffi::export(async_runtime = "tokio")]`)
  needs `org.jetbrains.kotlinx:kotlinx-coroutines-core` as a dependency, or
  `compileKotlin` fails with "Unresolved reference kotlinx" — this was
  missing and silently broken in this repo's existing Kotlin packaging
  before anyone actually build-tested it.
- **Don't nest the Android AGP module under the JVM module's Gradle
  build**: sharing one Gradle build/classpath between a module applying
  `kotlin("jvm")` and a module using AGP 9's built-in Kotlin causes
  plugin-classloader collisions (`Could not create an instance of type
  KotlinAndroidTarget`). Give the Android module its own independent
  `settings.gradle.kts` instead.
- **AGP 9's built-in Kotlin** support is NOT compatible with also applying
  the `org.jetbrains.kotlin.android` plugin — don't apply both; just use
  `com.android.library` alone and let AGP compile the Kotlin sources
  itself.
- **Swift packaging was macOS-only** for a long time despite being called
  "Swift packaging" — always check `packaging/*/README.md`'s Caveats
  section before assuming a packaging target is actually complete; several
  were previously stubs/gaps that looked plausible but were never fully
  build-verified.
- **justfile template provenance**: this repo's `justfile` follows the same
  task-runner template/conventions as the author's other Rust workspace
  projects (same recipe names/structure for build/test/release/changelog/
  multi-remote push). If you maintain a sibling project with the same
  template, keep the shared recipes (release/bump/changelog/git-remote
  ones) in sync and only diverge on crate names and language-specific
  packaging recipes (this repo has UniFFI bindings + Node/Python/Ruby/
  Kotlin/Swift packaging that a GUI/TUI-style sibling project wouldn't
  need, and vice versa for VHS gui/tui-split or AUR packaging recipes).

## Verification

1. `just check-all` passes (fmt + clippy `-D warnings` + `test --workspace`
   + doc + nu script tests).
2. `cargo test --workspace --locked --all-features --all-targets` shows
   all suites passing, no ignored failures.
3. For any new/changed packaging target: actually run the real build
   command locally (`cargo build` for the target triple, `gradle build`/
   `assembleRelease`, `maturin build`, etc.) and inspect the produced
   artifact (e.g. `unzip -l` an AAR/XCFramework/jar) rather than trusting
   that the config merely parses.
4. For VHS tape changes: render with `just vhs-tape <name>` and inspect
   multiple extracted frames (`ffmpeg -i generated.gif frame_%03d.png`)
   rather than trusting the render succeeded silently — check for leaked
   background-process log lines or shell job-control messages.
5. Before pushing a version tag: `gh run list --branch main --limit 5`
   shows the latest CI run green.
