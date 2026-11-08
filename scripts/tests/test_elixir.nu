#!/usr/bin/env nu
# ── stem-mqtt · test_elixir.nu ──────────────────────────────────────────────
# Tests for the helpers in scripts/ci/elixir.nu (the stem_mqtt Hex package).

use std/assert
use ../ci/elixir.nu [hex_has is_duplicate_version]
use runner.nu run-tests

def "test elixir: a version Hex never had is not published" [] {
    assert (not (hex_has "stem_mqtt" "0.0.0-never"))
}

def "test elixir: a published Hex release is found" [] {
    assert (hex_has "jason" "1.4.4")
}

# The GitHub and Gitea releases of one tag may race to publish.
def "test elixir: a version published meanwhile counts as published" [] {
    assert (is_duplicate_version "** (Mix) must include the --replace flag to update an existing package version")
    assert (not (is_duplicate_version "** (Mix) Invalid API key"))
}

def main [] { run-tests }
