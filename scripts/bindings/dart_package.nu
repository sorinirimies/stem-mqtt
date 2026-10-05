use rust_sources.nu *

# Build the pub.dev package `stem_mqtt` from generated Dart bindings.
#
# Like the Hackage package it is self-contained: a build hook compiles the bundled Rust
# sources (packaging/dart/pub/hook/build.dart), so no prebuilt library is shipped.

const TEMPLATE = "packaging/dart/pub"

# <gen>        generator output for the broker crate (includes the client bindings)
# <rust_root>  a workspace holding crates/ + Cargo.toml + Cargo.lock pinned to the generator's UniFFI
export def build-dart-package [gen: string, rust_root: string, out: string, version: string] {
    let repo = $env.PWD
    rm -rf $out
    mkdir ($out | path join "lib")
    mkdir ($out | path join "hook")

    # The generated code addresses its native asset as `package:uniffi/...` (the generator's
    # default package name). Rename it to this package so `stem_mqtt` is self-consistent and
    # doesn't squat the generic name `uniffi` on pub.dev.
    for file in (glob ($gen | path join "*.dart")) {
        open --raw $file
        | str replace --all "package:uniffi/uniffi:" "package:stem_mqtt/uniffi:"
        | save --force ($out | path join "lib" ($file | path basename))
    }

    # The Rust sources (no build output) compiled at install time.
    bundle-rust-sources $rust_root ($out | path join "rust")

    cp ($repo | path join $TEMPLATE "hook" "build.dart") ($out | path join "hook")
    cp ($repo | path join $TEMPLATE "README.md") $out
    $"## ($version)\n\nSee <https://github.com/sorinirimies/stem-mqtt/blob/main/CHANGELOG.md> for the release notes.\n"
    | save --force ($out | path join "CHANGELOG.md")
    cp ($repo | path join "LICENSE") $out
    open --raw ($repo | path join $TEMPLATE "pubspec.yaml.in")
    | str replace "@VERSION@" $version
    | save --force ($out | path join "pubspec.yaml")
}
