#!/usr/bin/env nu
# ── stem-mqtt · test_publish_crate.nu ───────────────────────────────────────
# Tests for the pure/exported helpers in scripts/ci/publish_crate.nu.
# Does NOT hit the network — `is_already_published` for a version that could
# never exist is used purely to exercise the "not found" code path.

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

def main [] { run-tests }
