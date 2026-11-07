#!/usr/bin/env nu
# ── stem-mqtt · test_publish_crate.nu ───────────────────────────────────────
# Tests for the pure/exported helpers in scripts/ci/publish_crate.nu.
# `is_already_published` queries crates.io's real registry API, so these
# tests do hit the network (crates.io's public API, no auth needed).

use std/assert
use runner.nu *
use ../ci/publish_crate.nu [is_already_published is_duplicate_version]

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

# Regression: v0.5.0's GitHub release failed on "crate version `0.5.0` is already
# uploaded" after the Gitea release had published it first.
def "test publish_crate: an already uploaded version counts as published" [] {
    assert (is_duplicate_version "error: failed to publish stem-mqtt-client v0.5.0 to registry at https://crates.io\n\nCaused by:\n  the remote server responded with an error (status 400 Bad Request): crate version `0.5.0` is already uploaded")
    assert (is_duplicate_version "crate version `0.4.0` already exists")
    assert (not (is_duplicate_version "error: 401 Unauthorized"))
}
