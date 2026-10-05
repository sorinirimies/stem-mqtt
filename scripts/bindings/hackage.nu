use rust_sources.nu *

# Build the Hackage source package `stem-mqtt` from generated Haskell bindings.
#
# Hackage packages are source distributions, so the package carries what it needs to build
# itself: the generated modules + C shims, a vendored `UniFFI.Runtime`, and the Rust sources
# that a custom Setup.hs compiles with cargo (see packaging/haskell/hackage/Setup.hs).

const TEMPLATE = "packaging/haskell/hackage"

# <gen>          generator output for the broker crate (includes the client modules)
# <rust_root>    a workspace holding crates/ + Cargo.toml + Cargo.lock pinned to the generator's UniFFI
# <upstream>     checkout of uniffi-bindgen-haskell (for the runtime module and its license)
export def build-hackage-package [
    gen: string
    rust_root: string
    upstream: string
    out: string
    version: string
] {
    let repo = $env.PWD
    rm -rf $out
    mkdir $out
    let manifest = (open ($gen | path join "manifest.json"))

    # Generated Haskell + C, plus the runtime the generated code imports.
    cp -r ($gen | path join "haskell") ($out | path join "haskell")
    cp -r ($gen | path join "cbits") ($out | path join "cbits")
    let runtime_dir = ($out | path join "haskell" "UniFFI")
    mkdir $runtime_dir
    cp ($upstream | path join "haskell" "uniffi-runtime" "src" "UniFFI" "Runtime.hs") $runtime_dir
    cp ($upstream | path join "LICENSE") ($out | path join "LICENSE.uniffi-runtime")

    # The Rust sources (no build output) compiled at install time.
    bundle-rust-sources $rust_root ($out | path join "rust")

    cp ($repo | path join $TEMPLATE "Setup.hs") $out
    cp ($repo | path join $TEMPLATE "README.md") $out
    cp ($repo | path join "LICENSE") $out

    let modules = ($manifest.publicHaskellModules | append "UniFFI.Runtime")
    let indent = {|items| $items | each { |m| $"    ($m)" } | str join "\n" }
    open --raw ($repo | path join $TEMPLATE "stem-mqtt.cabal.in")
    | str replace "@VERSION@" $version
    | str replace "@EXPOSED@" (do $indent $modules)
    | str replace "@OTHER@" (do $indent $manifest.internalHaskellModules)
    | str replace "@CSOURCES@" (do $indent $manifest.cSources)
    | save --force ($out | path join "stem-mqtt.cabal")
}
