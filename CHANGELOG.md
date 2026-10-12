# Changelog

All notable changes to this project will be documented in this file.

## [0.3.5] - 2026-10-12
### ✨ Features
- accept SONATYPE_TOKEN (Portal settings.xml snippet, user:pass or bearer) for Maven Central
- skip the Maven Central upload when the versions are already published (GitHub + Gitea can both release)
### 🔄 CI
- publish Kotlin/JVM to Maven Central from GitHub too, via the shared Nushell script and SONATYPE_TOKEN
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.3.4...v0.3.5
## [0.3.4] - 2026-10-12
### ⚡ Performance
- Arc'd index entries, borrow-based fan_out, RwLock retain store with exact-topic fast path; add fan_out tests
- single-buffer packet encode, shared QoS0 frames, strict SUBACK/NUL handling found by fuzzing, new fuzz targets and CI, inner/ws tests
### ✨ Features
- pull-style auth queue for Dart/Haskell (next_auth_request/resolve_auth_request, fails closed); Criterion benches for codec, topic matching, fan-out, index
### 🐛 Bug Fixes
- bound topic depth (stack overflow), inbound QoS2, subscriptions; validate will topic
- fail-closed auth without a provider, outbound byte budget, pre-CONNECT socket cap, reserved $ topics, stop race; client QoS2 cap and keep-alive during callbacks; CI secrets via env, pin Go in Gitea CI **[BREAKING]**
- drop [[bench]] tables from bundled Rust sources (Dart/Haskell builds); fix private intra-doc link
- use str downcase again — str lowercase doesn't exist before nu 0.114 and broke test-nu on older nu
### 📚 Documentation
- move the publishing section out of the README into docs/publishing.md (maintainers)
- move Development, CI/CD and Release process out of the README into docs/development.md
### 🔄 CI
- add cargo-audit workflow and just audit recipe
- pin Go to 1.24.x for runtime-go job
- run nightly
- verify Go after setup with go.dev fallback; fix Dart/Haskell smoke tests and docs for fail-closed auth
### 🔧 Chores
- use str lowercase (nu 0.114+) and require it in _check-nu
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.3.3...v0.3.4
## [0.3.3] - 2026-10-09
### ✨ Features
- publish stem-mqtt (client + broker) to npmjs.org with multi-platform prebuilds
### 🔄 CI
- dispatchable npm publish workflow; public registries use the GitHub owner, not the Gitea user
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.3.2...v0.3.3
## [0.3.2] - 2026-10-06
### ♻️  Refactor
- own QoS enum so every binding is self-contained (no cross-crate external types) **[BREAKING]**
### ✨ Features
- pull-style delivery, subscription index, shared subscriptions, session expiry, MQTT 5 enhanced auth; fix varint overflow
- Swift runtime test, generator update checker, Gitea package target, Gitea/GitHub registry endpoints
- one binding solution per language, client and broker everywhere **[BREAKING]**
- public registries — Maven Central, Hackage, pub.dev, npmjs, NuGet.org, PyPI, Go modules
### 🐛 Bug Fixes
- bound enhanced-auth exchanges, skip event allocation when unpolled; drop rustls-pemfile and update rustls (RUSTSEC-2026-0285); broad unit/CLI tests
- link the Rust archive with -l so GNU ld resolves it (undefined ffi_*_rust_future_* on Linux)
### 💄 Style
- cargo fmt
### 📚 Documentation
- Haskell upstream PR, manual GitHub Packages publish workflow, CI image Dockerfile, docs for new features
- README, packaging and crate docs for new features, Gitea publishing, CI image, fuzzing
- rewrite README (Rust first, all languages, public registries, crates.io badges); new tapes + examples for shared subscriptions, retained/will and auth
- preview GIFs one per section instead of a grid
- detailed per-language guides (docs/languages) with tested quick starts; fix Java jar native loading
### 🔄 CI
- Swift runtime job, generator cache, weekly update/audit jobs, Gitea package publishing on release
- publish Kotlin (JVM) and Python wheels to Gitea registries; document publish coverage
- retry toolchain setup steps once (runner network to CDNs is flaky: EHOSTUNREACH on setup-java)
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.3.1...v0.3.2
## [0.3.1] - 2026-10-02
### 🐛 Bug Fixes
- retry rustup and cap parallelism for binding jobs; install aarch64 libc headers for the cross build
- install Gradle directly on Gitea (setup-gradle action cannot be resolved there)
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.3.0...v0.3.1
## [0.3.0] - 2026-10-02
### ✨ Features
- harden client and broker — session takeover race, packet limits, bounded queues, half-open detection; uniffi 0.31 **[BREAKING]**
- Go, C#, Java, Dart, Node and Haskell bindings with nushell tooling and per-language runtime tests
### 🐛 Bug Fixes
- validate packaged broker executables
- gate deps-update on Cargo.lock only, add permissions, drop auto-release
### 📚 Documentation
- document bindings, limits, GitHub Packages and the new CI
### 🔄 CI
- run binding runtime tests and package builds on Gitea and GitHub, publish to GitHub Packages on release
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.2.3...v0.3.0
## [0.2.3] - 2026-09-07
### ✨ Features
- real QoS 1/2 redelivery instead of send-once-and-hope
- real QoS 1/2 redelivery + fix spurious immediate keep-alive ping
- opt-in auto-reconnect with backoff + subscription replay
- TLS support (client + broker), pure-Rust rustls, incl. mutual TLS
- prepare multi-language MQTT 0.2.3 release
### 🐛 Bug Fixes
- preserve UniFFI metadata across Linux packaging
- define ARM architecture for cross-compiled ring
### 🔧 Chores
- remove project-local skill copy
- update changelog for v0.2.3
### 🧪 Testing
- cover session resumption, retained-clear, max_clients
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.2.2...v0.2.3
## [0.2.2] - 2026-08-05
### 🐛 Bug Fixes
- rename published packages to stem-mqtt-client / stem-mqtt-broker
### 📚 Documentation
- add per-language installation instructions, fix stale layout docs
### 🔧 Chores
- update Package.swift for v0.2.1
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.2.1...v0.2.2
## [0.2.1] - 2026-08-04
### 🐛 Bug Fixes
- npm publish was crashing, crates.io publish was silently a no-op
### 🔧 Chores
- update Package.swift for v0.2.0
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.2.0...v0.2.1
## [0.2.0] - 2026-08-04
### ♻️  Refactor
- break down broker god-object and monolithic client into cohesive modules
### ✨ Features
- real iOS device/simulator support + Android AAR
### 🐛 Bug Fixes
- silence broker tracing + job-control noise in pub-sub-demo.gif
- unblock the release quality gate on current Nushell + rustdoc
### 📚 Documentation
- add VHS demo GIF preview to README, track *.gif via git-lfs
- add stem-mqtt-development pi skill (.pi/skills/)
- restructure stem-mqtt-development skill with references/ subfolder
- add VHS demos for MQTT versions, topic wildcards, long-lived connections
### 🔧 Chores
- add VHS demo-tape recipes (sync from gitkraft justfile)
**Full Changelog**: https://github.com/sorinirimies/stem-mqtt/compare/v0.1.0...v0.2.0
## [0.1.0] - 2026-08-03
### ✨ Features
- initial stem-mqtt workspace — MQTT client/broker, docs, tests, CI/CD
- Node.js/TypeScript bindings (napi-rs) + multi-platform mqtt-client artifacts
- MQTT-over-WebSocket support + browser demo + Docker/Kubernetes
### 🔄 CI
- fix flaky test-node broker startup (pre-build + port poll)
