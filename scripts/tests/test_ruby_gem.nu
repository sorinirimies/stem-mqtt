#!/usr/bin/env nu
# ── stem-mqtt · test_ruby_gem.nu ────────────────────────────────────────────
# Tests for scripts/ci/ruby_gem.nu (per-platform Ruby gems).

use std/assert
use ../ci/ruby_gem.nu [PLATFORMS gem_platform patch_loader gemspec lib_file published]
use runner.nu run-tests

def "test ruby_gem: rust targets map to rubygems platforms" [] {
    assert equal (gem_platform "aarch64-apple-darwin") "arm64-darwin"
    assert equal (gem_platform "x86_64-pc-windows-msvc") "x64-mingw-ucrt"
    assert error { gem_platform "riscv64gc-unknown-linux-gnu" }
}

def "test ruby_gem: the loader points at the bundled library" [] {
    let src = "module UniFFILib\n  extend FFI::Library\n  ffi_lib 'mqtt_client'\nend\n"
    let out = (patch_loader $src "mqtt_client")
    assert ($out | str contains "ffi_lib File.expand_path(\"native/#{FFI.map_library_name('mqtt_client')}\", __dir__)")
    assert (not ($out | str contains "ffi_lib 'mqtt_client'"))
}

# Guards against a UniFFI template change silently shipping a gem that looks
# for the library on the system path.
def "test ruby_gem: an unexpected loader is an error" [] {
    assert error { patch_loader "ffi_lib File.join(x)" "mqtt_client" }
}

def "test ruby_gem: one library per platform, named per OS" [] {
    assert equal (lib_file "mqtt-client" "x86_64-unknown-linux-gnu") "libmqtt_client.so"
    assert equal (lib_file "mqtt-broker" "aarch64-apple-darwin") "libmqtt_broker.dylib"
    assert equal (lib_file "mqtt-broker" "x86_64-pc-windows-msvc") "mqtt_broker.dll"
}

def "test ruby_gem: the gemspec pins name, version and platform" [] {
    let spec = (gemspec "mqtt-broker" "1.2.3" "x86_64-linux")
    assert ($spec | str contains 's.name = "stem-mqtt-broker"')
    assert ($spec | str contains 's.version = "1.2.3"')
    assert ($spec | str contains 's.platform = "x86_64-linux"')
    assert ($spec | str contains 's.add_dependency "ffi"')
}

def "test ruby_gem: a version is published per platform" [] {
    let versions = [{ number: "1.0.0", platform: "x86_64-linux" } { number: "1.0.0", platform: "arm64-darwin" }]
    assert (published $versions "1.0.0" "x86_64-linux")
    assert (not (published $versions "1.0.0" "x64-mingw-ucrt"))
    assert (not (published $versions "1.0.1" "x86_64-linux"))
}

def main [] { run-tests }
