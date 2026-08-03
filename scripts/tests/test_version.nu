#!/usr/bin/env nu
# ── stem-mqtt · test_version.nu ─────────────────────────────────────────────
# Tests for scripts/version.nu — reads workspace.package.version from
# Cargo.toml and confirms it's a well-formed semver string that matches what
# every crate's `version.workspace = true` will resolve to.

use std/assert
use runner.nu *

def "test version: matches semver shape" [] {
    let version = (open Cargo.toml | get workspace.package.version)
    assert ($version | find --regex '^\d+\.\d+\.\d+$' | is-not-empty)
}

def "test version: script output matches Cargo.toml directly" [] {
    let from_toml = (open Cargo.toml | get workspace.package.version)
    let from_script = (nu scripts/version.nu | str trim)
    assert equal $from_toml $from_script
}

def "test version: mqtt-client crate inherits workspace version" [] {
    let crate_toml = (open crates/mqtt-client/Cargo.toml)
    assert equal ($crate_toml | get package.version.workspace) true
}

def "test version: internal mqtt-client dependency pin matches workspace version" [] {
    let workspace_version = (open Cargo.toml | get workspace.package.version)
    let pinned = (open Cargo.toml | get workspace.dependencies.mqtt-client.version)
    assert equal $pinned $workspace_version
}

def main [] { run-tests }
