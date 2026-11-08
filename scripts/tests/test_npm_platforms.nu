#!/usr/bin/env nu
# ── stem-mqtt · test_npm_platforms.nu ───────────────────────────────────────
# Tests for scripts/bindings/npm_platforms.nu (one npm package per platform).

use std/assert
use ../bindings/npm_platforms.nu [platform-package-name platform-manifest main-manifest patch-node-loader platform-readme]
use runner.nu run-tests

const LOADER = 'function defaultBundledLibrary() {
  return Object.freeze({
    target,
    libraryPath: join(moduleDirectory, "prebuilds", target, filename),
  });
}'

def "test npm platforms: a platform package is restricted to its os, cpu and libc" [] {
    let m = (platform-manifest "stem-mqtt" "linux-arm64-gnu" "1.2.3" "acme" { registry: "r", access: "public" })
    assert equal $m.name "stem-mqtt-linux-arm64-gnu"
    assert equal $m.version "1.2.3"
    assert equal $m.os ["linux"]
    assert equal $m.cpu ["arm64"]
    assert equal $m.libc ["glibc"]
    assert equal $m.publishConfig.access "public"
    assert equal $m.files ["client" "broker" "README.md" "LICENSE"]
    let mac = (platform-manifest "stem-mqtt" "darwin-arm64" "1.2.3" "acme" { registry: "r" })
    assert ("libc" not-in ($mac | columns))
}

def "test npm platforms: the main package depends on every platform package, optionally" [] {
    let m = (main-manifest { name: "x", dependencies: { koffi: "^2" } } "@acme/pkg" ["win32-x64" "darwin-arm64" "linux-x64-gnu"] "2.0.0")
    assert equal $m.name "@acme/pkg"
    assert equal $m.optionalDependencies { "@acme/pkg-darwin-arm64": "2.0.0", "@acme/pkg-linux-x64-gnu": "2.0.0", "@acme/pkg-win32-x64-msvc": "2.0.0" }
    assert equal $m.os ["darwin" "linux" "win32"]
    assert equal $m.cpu ["arm64" "x64"]
    assert equal $m.dependencies.koffi "^2"
}

def "test npm platforms: the loader looks in the platform package first" [] {
    let out = (patch-node-loader $LOADER "stem-mqtt-" "broker")
    assert ($out starts-with 'import { createRequire as uniffiCreateRequire } from "node:module";')
    # The same names as platform-package-name: -msvc on Windows.
    assert ($out | str contains 'const name = target.startsWith("win32-") ? `${target}-msvc` : target;')
    assert ($out | str contains 'resolve(`stem-mqtt-${name}/package.json`)')
    assert ($out | str contains 'join(dirname(manifest), "broker", filename)')
    assert ($out | str contains 'libraryPath: uniffiPlatformLibrary(target, filename) ?? join(moduleDirectory, "prebuilds", target, filename),')
}

def "test npm platforms: a changed loader is an error, not a package without a library" [] {
    assert error { patch-node-loader "function other() {}" "stem-mqtt-" "client" }
}

def "test npm platforms: platform package names keep the scope" [] {
    # Windows packages name their ABI (npmjs.org rejected stem-mqtt-win32-x64 as spam, v0.7.0).
    assert equal (platform-package-name "@acme/stem-mqtt-node" "win32-x64") "@acme/stem-mqtt-node-win32-x64-msvc"
    assert equal (platform-package-name "stem-mqtt" "linux-x64-gnu") "stem-mqtt-linux-x64-gnu"
}

def "test npm platforms: a platform package has a readme that points to the main package" [] {
    let readme = (platform-readme "stem-mqtt" "win32-x64" "o")
    assert ($readme | str starts-with "# stem-mqtt-win32-x64-msvc\n")
    assert ($readme | str contains "`npm install stem-mqtt`")
    assert ($readme | str contains "https://github.com/o/stem-mqtt")
    assert ("README.md" in (platform-manifest "stem-mqtt" "win32-x64" "1.0.0" "o" {}).files)
}

def main [] { run-tests }
