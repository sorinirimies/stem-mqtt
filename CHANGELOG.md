# Changelog

All notable changes to this project will be documented in this file.

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
