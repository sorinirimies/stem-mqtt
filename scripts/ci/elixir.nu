#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — The Elixir/Erlang package (packaging/elixir, Hex: stem_mqtt)
# ──────────────────────────────────────────────────────────────────────────────
# Usage:
#   nu scripts/ci/elixir.nu test               build the NIF from source; format, compile, ExUnit
#   nu scripts/ci/elixir.nu publish <version>  after the GitHub release: checksum the release's
#                                              NIFs, test against them, mix hex.publish
#                                              (HEX_API_KEY). Skips a version Hex already has.
# ──────────────────────────────────────────────────────────────────────────────

const PACKAGE = "stem_mqtt"
const DIR = "packaging/elixir"

# Whether Hex already has `package` at `version`.
export def hex_has [package: string, version: string]: nothing -> bool {
    let resp = (http get --full --allow-errors $"https://hex.pm/api/packages/($package)/releases/($version)")
    $resp.status == 200
}

# Whether `mix hex.publish` failed only because another run published this version first.
export def is_duplicate_version [output: string]: nothing -> bool {
    ($output | str contains "--replace") or ($output | str contains "has already been published")
}

def --wrapped mix [...args: string] {
    print $"▶ mix ($args | str join ' ')"
    ^mix ...$args
}

def setup [] {
    cd $DIR
    mix local.hex --force
    mix local.rebar --force
    mix deps.get
}

def "main test" [] {
    setup
    cd $DIR
    with-env { STEM_MQTT_BUILD: "1" } {
        mix format --check-formatted
        mix compile --warnings-as-errors
        mix test
    }
}

def "main publish" [version: string] {
    if (hex_has $PACKAGE $version) {
        print $"⏭️  ($PACKAGE) ($version) is already on Hex — skipping."
        return
    }
    setup
    cd $DIR
    # The download task compiles the app, which needs the NIF: build it from source once.
    with-env { STEM_MQTT_BUILD: "1" } {
        mix rustler_precompiled.download StemMqtt.Native --all --print
    }
    # What a Hex user gets: the release's NIF, verified against the new checksums.
    rm -rf _build priv/native
    mix compile --warnings-as-errors
    mix test
    let result = (do { ^mix hex.publish --yes } | complete)
    print $result.stdout
    if $result.exit_code != 0 {
        let output = $"($result.stdout)\n($result.stderr)"
        if (is_duplicate_version $output) {
            print $"⏭️  ($PACKAGE) ($version) was published meanwhile — skipping."
            return
        }
        print -e $result.stderr
        error make --unspanned { msg: "mix hex.publish failed" }
    }
}

def main [] {
    print "usage: nu scripts/ci/elixir.nu test | publish <version>"
}
