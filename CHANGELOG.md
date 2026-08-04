# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2026-08-04
### ♻️  Refactor
- break down broker god-object and monolithic client into cohesive modules
### ✨ Features
- real iOS device/simulator support + Android AAR
### 🐛 Bug Fixes
- silence broker tracing + job-control noise in pub-sub-demo.gif
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
