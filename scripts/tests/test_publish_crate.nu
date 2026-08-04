#!/usr/bin/env nu
# ── stem-mqtt · test_publish_crate.nu ───────────────────────────────────────
# Tests for the pure/exported helpers in scripts/ci/publish_crate.nu.
# `is_already_published` queries crates.io's real registry API, so these
# tests do hit the network (crates.io's public API, no auth needed).

use std/assert
use runner.nu *
use ../ci/publish_crate.nu [is_already_published]

def "test publish_crate: nonsense version is reported as not published" [] {
    let result = (is_already_published "mqtt-client" "0.0.0-definitely-not-a-real-release")
    assert equal $result false
}

def "test publish_crate: nonsense crate name is reported as not published" [] {
    let result = (is_already_published "this-crate-name-does-not-exist-anywhere-xyz" "0.1.0")
    assert equal $result false
}

# Regression test for a real bug: the previous implementation shelled out to
# `cargo info <crate>@<version>`, which — when run inside this workspace —
# resolves against the *local path crate* of the same name+version instead
# of querying the actual crates.io registry. That meant it always reported
# "already published" for whatever version currently sits in Cargo.toml,
# even if that exact version had never actually been published, silently
# no-op'ing every real crates.io publish attempt in CI. A bogus version
# under the *real* crate name (which does exist on crates.io, just owned by
# an unrelated project) exercises exactly that gap.
def "test publish_crate: real crate name with a version that was never published is reported as not published" [] {
    let result = (is_already_published "mqtt-client" "0.0.0-local-workspace-version-probe")
    assert equal $result false
}

def main [] { run-tests }
