#!/usr/bin/env nu
# ── stem-mqtt · test_version.nu ─────────────────────────────────────────────
# Tests for scripts/version.nu — reads workspace.package.version from
# Cargo.toml and confirms it's a well-formed semver string that matches what
# every crate's `version.workspace = true` will resolve to.

use std/assert
use runner.nu *
use ../bump_version.nu [set_mix_version set_internal_dep_versions]

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

def "test version: mqtt-proto crate inherits workspace version" [] {
    let crate_toml = (open crates/mqtt-proto/Cargo.toml)
    assert equal ($crate_toml | get package.version.workspace) true
}

def "test version: internal mqtt-proto dependency pin matches workspace version" [] {
    let workspace_version = (open Cargo.toml | get workspace.package.version)
    let pinned = (open Cargo.toml | get workspace.dependencies.mqtt-proto.version)
    assert equal $pinned $workspace_version
}

def "test version: mqtt-client takes mqtt-proto from the workspace so a bump reaches it" [] {
    assert equal (open crates/mqtt-client/Cargo.toml | get dependencies.mqtt-proto.workspace) true
}

def "test version: a bump rewrites both internal pins and nothing else" [] {
    let before = ([
        'tokio = { version = "1" }'
        'mqtt-proto  = { package = "stem-mqtt-proto", path = "crates/mqtt-proto", version = "0.8.0" }'
        'mqtt-client = { package = "stem-mqtt-client", path = "crates/mqtt-client", version = "0.8.0" }'
        'mqtt-broker = { path = "crates/mqtt-broker", version = "0.8.0" }'
    ] | str join "\n")
    let after = (set_internal_dep_versions $before "0.9.0" | lines)
    assert equal ($after | get 0) 'tokio = { version = "1" }'
    assert ($after | get 1 | str contains 'version = "0.9.0"')
    assert ($after | get 2 | str contains 'version = "0.9.0"')
    assert ($after | get 3 | str contains 'version = "0.8.0"') "only the pins the script owns change"
}

def "test version: internal mqtt-client dependency pin matches workspace version" [] {
    let workspace_version = (open Cargo.toml | get workspace.package.version)
    let pinned = (open Cargo.toml | get workspace.dependencies.mqtt-client.version)
    assert equal $pinned $workspace_version
}

# crates.io's "mqtt-client" name is taken by an unrelated project (since
# 2019) that this repo doesn't own, so the *published* Cargo package names
# are stem-mqtt-client / stem-mqtt-broker — only the [lib] name (Rust import
# path, `use mqtt_client::...`) and the directory stay mqtt-client/
# mqtt_client. This test guards against silently reverting that rename.
def "test version: crates.io package names are stem-prefixed since mqtt-client and mqtt-broker are taken by unrelated crates" [] {
    let client_pkg = (open crates/mqtt-client/Cargo.toml | get package.name)
    let broker_pkg = (open crates/mqtt-broker/Cargo.toml | get package.name)
    assert equal $client_pkg "stem-mqtt-client"
    assert equal $broker_pkg "stem-mqtt-broker"

    # The workspace.dependencies alias key stays "mqtt-client" for internal
    # ergonomics (every crate says `mqtt-client = { workspace = true }`),
    # but must resolve to the real, renamed package.
    let aliased_package = (open Cargo.toml | get workspace.dependencies.mqtt-client.package)
    assert equal $aliased_package "stem-mqtt-client"
}

# Hex users get the version from mix.exs, so it must follow the workspace.
def "test version: the Hex package has the workspace version" [] {
    let version = (open Cargo.toml | get workspace.package.version)
    let mix = (open --raw packaging/elixir/mix.exs)
    assert ($mix | str contains $'@version "($version)"') "packaging/elixir/mix.exs @version drifted from Cargo.toml"
}

def "test version: set_mix_version rewrites only @version" [] {
    let mix = "  @version \"0.1.0\"\n  @source_url \"x\"\n  version: @version,\n"
    assert equal (set_mix_version $mix "1.2.3") "  @version \"1.2.3\"\n  @source_url \"x\"\n  version: @version,\n"
}

def main [] { run-tests }
