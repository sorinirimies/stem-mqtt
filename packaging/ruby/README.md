# Ruby packaging (RubyGems)

Ruby isn't one of the languages built into the `uniffi` crate's own CLI
(only Kotlin/Swift/Python ship there) — Ruby bindings come from the
separate, community-maintained
[`uniffi-bindgen-ruby`](https://github.com/mozilla/uniffi-rs/tree/main/uniffi_bindgen)
generator, invoked directly rather than through
`scripts/generate-bindings.sh`.

## What `build_and_publish.sh` does

1. `cargo build --release` for both crates.
2. `uniffi-bindgen-ruby --library <cdylib> --out-dir lib/stem_mqtt` for
   each crate.
3. Copies the release native library alongside the generated `.rb` files.
4. Renders `stem_mqtt.gemspec.tmpl` with the release version, `gem build`s
   it, and `gem push`es the result — gated on the `RUBYGEMS_API_KEY`
   repository secret in `.github/workflows/release.yml`'s `publish-ruby`
   job (skips gracefully if unset, same pattern as crates.io/PyPI).

## Building locally

```sh
cargo install uniffi-bindgen-ruby --locked
export GEM_HOST_API_KEY=...   # only needed to actually push
./packaging/ruby/build_and_publish.sh 0.0.0-dev
```

## Caveats

- Bundles **one native library for the host OS/arch** that built it — no
  per-platform gem variants yet (RubyGems supports platform-specific gems
  via the `platform:` gemspec attribute; extending this to multi-platform
  is a follow-up, not implemented here).
- `uniffi-bindgen-ruby`'s CLI surface can change between versions; if
  `build_and_publish.sh` fails on the `uniffi-bindgen-ruby` step, check
  `uniffi-bindgen-ruby --help` against what this script assumes.
