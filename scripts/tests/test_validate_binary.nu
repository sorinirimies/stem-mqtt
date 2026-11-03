#!/usr/bin/env nu
use std/assert
use runner.nu *
use ../ci/validate_binary.nu [format_for has_magic problem]

def pad [head: binary]: nothing -> binary { $head | bytes add --end (1..2000 | each { 0x[01] } | bytes collect) }

def "test validate_binary: each target needs its format" [] {
    assert equal (format_for x86_64-pc-windows-msvc) "PE/COFF (MZ)"
    assert equal (format_for aarch64-apple-darwin) "Mach-O"
    assert equal (format_for x86_64-unknown-linux-musl) "ELF"
    assert equal (format_for wasm32-unknown-unknown) null
}

def "test validate_binary: real headers pass" [] {
    assert equal (problem (pad 0x[4D 5A 90 00]) x86_64-pc-windows-msvc) null
    assert equal (problem (pad 0x[CF FA ED FE]) aarch64-apple-darwin) null
    assert equal (problem (pad 0x[CA FE BA BE]) aarch64-apple-darwin) null
    assert equal (problem (pad 0x[7F 45 4C 46]) x86_64-unknown-linux-gnu) null
}

def "test validate_binary: a wrong format fails" [] {
    assert ((problem (pad 0x[7F 45 4C 46]) x86_64-pc-windows-msvc) | str starts-with "not PE/COFF")
}

def "test validate_binary: empty, tiny and zero-filled binaries fail" [] {
    assert equal (problem 0x[] x86_64-unknown-linux-gnu) "missing or implausibly small binary"
    assert equal (problem (1..2000 | each { 0x[00] } | bytes collect) x86_64-unknown-linux-gnu) "binary contains only zero bytes"
}

def "test validate_binary: an unknown target is an error" [] {
    assert ((problem (pad 0x[7F 45 4C 46]) wasm32-unknown-unknown) | str starts-with "unsupported")
}

def main [] { run-tests }
