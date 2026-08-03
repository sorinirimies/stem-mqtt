#!/usr/bin/env bash
# Stages generated Kotlin bindings + the release cdylib into
# packaging/kotlin/staged/ so `gradle publish` (run from packaging/kotlin/)
# can package them into one JVM jar.
#
# Usage: packaging/kotlin/stage.sh <version>
#
# Expects `scripts/generate-bindings.sh kotlin <crate>` to have already been
# run for both crates (the release workflow does this before calling this
# script) — see .github/workflows/release.yml's `publish-kotlin` job.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

version="${1:?usage: $0 <version>}"
out="packaging/kotlin/staged"
rm -rf "$out"
mkdir -p "$out/kotlin" "$out/resources"

for crate in mqtt-client mqtt-broker; do
    lib_name="${crate//-/_}"
    src="bindings/kotlin/${crate}"
    if [ -d "$src" ]; then
        echo "==> staging Kotlin sources for ${crate}"
        cp -r "$src"/. "$out/kotlin/"
    fi

    # Native library, named the way UniFFI's Kotlin JVM loader expects
    # (platform-qualified resource path; adjust per target when building a
    # true multi-platform jar with one native lib per OS/arch).
    case "$(uname -s)" in
        Darwin) ext="dylib"; os_dir="darwin" ;;
        Linux)  ext="so"; os_dir="linux-x86-64" ;;
        *) echo "unsupported host OS for staging: $(uname -s)" >&2; exit 1 ;;
    esac
    lib_path="target/release/lib${lib_name}.${ext}"
    if [ -f "$lib_path" ]; then
        mkdir -p "$out/resources/${os_dir}"
        cp "$lib_path" "$out/resources/${os_dir}/"
    else
        echo "==> building ${crate} release cdylib (not found at ${lib_path})"
        cargo build --release -p "${crate}"
        mkdir -p "$out/resources/${os_dir}"
        cp "$lib_path" "$out/resources/${os_dir}/"
    fi
done

echo "staged version ${version} -> ${out}"
