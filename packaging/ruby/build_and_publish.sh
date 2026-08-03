#!/usr/bin/env bash
# Builds and publishes the `stem_mqtt` Ruby gem: generates Ruby bindings via
# the external `uniffi-bindgen-ruby` generator (Ruby isn't one of the
# languages built into the `uniffi` crate's own CLI, unlike Kotlin/Swift/
# Python), bundles them with the release cdylib, and `gem push`es the
# result.
#
# Usage: packaging/ruby/build_and_publish.sh <version>
#
# Requires:
#   - `uniffi-bindgen-ruby` on PATH (cargo install uniffi-bindgen-ruby)
#   - GEM_HOST_API_KEY set for `gem push` to authenticate
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
work="packaging/ruby/.build"
rm -rf "$work"
mkdir -p "$work/lib/stem_mqtt"

case "$(uname -s)" in
    Darwin) ext="dylib" ;;
    Linux)  ext="so" ;;
    *) echo "unsupported host OS: $(uname -s)" >&2; exit 1 ;;
esac

for crate in mqtt-client mqtt-broker; do
    lib_name="${crate//-/_}"
    echo "==> building ${crate} (release cdylib)"
    cargo build --release -p "${crate}"

    echo "==> generating Ruby bindings for ${crate}"
    uniffi-bindgen-ruby \
        --library "target/release/lib${lib_name}.${ext}" \
        --out-dir "$work/lib/stem_mqtt"

    cp "target/release/lib${lib_name}.${ext}" "$work/lib/stem_mqtt/"
done

sed "s/VERSION_PLACEHOLDER/${version}/" packaging/ruby/stem_mqtt.gemspec.tmpl \
    > "$work/stem_mqtt.gemspec"

(
    cd "$work"
    gem build stem_mqtt.gemspec
    gem push ./*.gem
)

echo "published stem_mqtt ${version}"
