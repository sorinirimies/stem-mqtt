# Ruby

The gems `stem-mqtt-client` and `stem-mqtt-broker` (rubygems.org) wrap UniFFI's built-in Ruby
bindings (UniFFI 0.32 ships Ruby with callbacks, async and trait interfaces).

Each gem is built **per platform** and carries only that platform's native library
(`x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `arm64-darwin`, `x64-mingw-ucrt`); `gem install`
picks the one for the machine. The generated module's `ffi_lib 'mqtt_client'` is rewritten to load
the library bundled next to it.

- Built from the libraries of `build-native-libs` and pushed by `publish-ruby` in
  `.github/workflows/release.yml` (`RUBY_GEM_TOKEN`), after the release so they are not release assets.
- `scripts/ci/ruby_gem.nu` builds (`build`, `release`) and pushes (`publish`, skipping versions
  rubygems.org already has).
- `just test-bindings ruby` builds both gems for this machine, installs them into a scratch gem home
  and runs `tests/bindings/ruby/smoke.rb` against them.
