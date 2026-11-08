#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — Ruby gems (stem-mqtt-client, stem-mqtt-broker on rubygems.org)
# ──────────────────────────────────────────────────────────────────────────────
# One gem per crate per platform, each carrying only that platform's native
# library; `gem install` picks the gem matching the machine.
#
# Usage:
#   nu scripts/ci/ruby_gem.nu build <crate> <version> <rust-target> <native-lib> <generated.rb> <out-dir>
#   nu scripts/ci/ruby_gem.nu release <version> <libs-dir> <out-dir>
#                                                every gem: <libs-dir>/<rust-target>/ holds that
#                                                target's two libraries (the release build)
#   nu scripts/ci/ruby_gem.nu publish <dir>      gem push every *.gem in <dir> (GEM_HOST_API_KEY);
#                                                 versions rubygems.org already has are skipped
# ──────────────────────────────────────────────────────────────────────────────

# Rust target → RubyGems platform. Only targets the release builds.
export const PLATFORMS = {
    "x86_64-unknown-linux-gnu": "x86_64-linux"
    "aarch64-unknown-linux-gnu": "aarch64-linux"
    "x86_64-apple-darwin": "x86_64-darwin"
    "aarch64-apple-darwin": "arm64-darwin"
    "x86_64-pc-windows-msvc": "x64-mingw-ucrt"
}

# crate → gem name, Ruby entry file and generated module file.
export const GEMS = {
    "mqtt-client": { gem: "stem-mqtt-client", entry: "stem_mqtt_client", module: "mqtt_client", summary: "MQTT 3.1.1/5.0 client" }
    "mqtt-broker": { gem: "stem-mqtt-broker", entry: "stem_mqtt_broker", module: "mqtt_broker", summary: "Embeddable MQTT 3.1.1/5.0 broker" }
}

export def gem_platform [target: string]: nothing -> string {
    $PLATFORMS | get --optional $target | default {
        error make --unspanned { msg: $"no Ruby platform for Rust target '($target)'. Known: ($PLATFORMS | columns | str join ', ')" }
    }
}

# The generated module loads its library by bare name (from the system search
# path); make it load the copy shipped in the gem's native/ directory.
export def patch_loader [source: string, lib: string]: nothing -> string {
    let bare = $"ffi_lib '($lib)'"
    if not ($source | str contains $bare) {
        error make --unspanned { msg: $"generated Ruby has no `($bare)`; the UniFFI Ruby template changed" }
    }
    $source | str replace $bare $"ffi_lib File.expand_path\(\"native/#{FFI.map_library_name\('($lib)'\)}\", __dir__\)"
}

export def gemspec [crate: string, version: string, platform: string]: nothing -> string {
    let g = ($GEMS | get $crate)
    [
        "Gem::Specification.new do |s|"
        $"  s.name = \"($g.gem)\""
        $"  s.version = \"($version)\""
        $"  s.platform = \"($platform)\""
        $"  s.summary = \"($g.summary) for Ruby, implemented in Rust \(stem-mqtt\)\""
        "  s.authors = [\"Sorin Irimies\"]"
        "  s.license = \"MIT\""
        "  s.homepage = \"https://github.com/sorinirimies/stem-mqtt\""
        "  s.metadata = { \"source_code_uri\" => \"https://github.com/sorinirimies/stem-mqtt\", \"documentation_uri\" => \"https://github.com/sorinirimies/stem-mqtt/blob/main/docs/languages/ruby.md\" }"
        "  s.required_ruby_version = \">= 3.1\""
        "  s.files = Dir[\"lib/**/*\"]"
        "  s.require_paths = [\"lib\"]"
        "  s.add_dependency \"ffi\", \"~> 1.15\""
        "end"
        ""
    ] | str join "\n"
}

def "main build" [crate: string, version: string, target: string, native_lib: string, generated: string, out_dir: string] {
    let g = ($GEMS | get $crate)
    let platform = (gem_platform $target)
    let stage = ($out_dir | path join $"stage-($g.gem)-($platform)")
    let lib_dir = ($stage | path join lib $g.entry)
    rm -rf $stage
    mkdir ($lib_dir | path join native)
    cp $native_lib ($lib_dir | path join native)
    patch_loader (open --raw $generated) $g.module | save --force ($lib_dir | path join $"($g.module).rb")
    $"require_relative \"($g.entry)/($g.module)\"\n" | save --force ($stage | path join lib $"($g.entry).rb")
    gemspec $crate $version $platform | save --force ($stage | path join $"($g.gem).gemspec")
    let output = (gem_output $out_dir $g.gem $version $platform)
    do { cd $stage; ^gem build $"($g.gem).gemspec" --output $output }
    rm -rf $stage
}

# The file name of `crate`'s shared library on `target`.
export def lib_file [crate: string, target: string]: nothing -> string {
    let base = ($crate | str replace "-" "_")
    if ($target | str contains "windows") {
        $"($base).dll"
    } else if ($target | str contains "apple") {
        $"lib($base).dylib"
    } else {
        $"lib($base).so"
    }
}

def "main release" [version: string, libs_dir: string, out_dir: string] {
    let gen = ($out_dir | path join "gen")
    mkdir $out_dir
    for crate in ($GEMS | columns) {
        nu scripts/generate_bindings.nu ruby $crate ($gen | path join $crate)
    }
    let targets = (ls $libs_dir | where type == dir | get name | path basename)
    let missing = ($PLATFORMS | columns | where {|t| $t not-in $targets })
    if ($missing | is-not-empty) {
        error make --unspanned { msg: $"no libraries for ($missing | str join ', ')" }
    }
    for target in ($PLATFORMS | columns) {
        for crate in ($GEMS | columns) {
            let g = ($GEMS | get $crate)
            let lib = ($libs_dir | path join $target (lib_file $crate $target))
            main build $crate $version $target $lib ($gen | path join $crate $"($g.module).rb") $out_dir
        }
    }
    rm -rf $gen
    print (ls ($out_dir | path join "*.gem" | into glob) | select name size)
}

# The absolute path of a built gem. Resolved before `gem build` runs inside the
# staging directory, where a relative `out_dir` would point elsewhere.
export def gem_output [out_dir: string, gem: string, version: string, platform: string]: nothing -> string {
    $out_dir | path expand | path join $"($gem)-($version)-($platform).gem"
}

# Whether rubygems.org has `gem` at `version` for `platform`.
export def published [versions: list<record>, version: string, platform: string]: nothing -> bool {
    $versions | any {|v| $v.number == $version and $v.platform == $platform }
}

def "main publish" [dir: string] {
    for file in (glob ($dir | path join "*.gem")) {
        let parts = ($file | path basename | parse --regex '^(?<gem>stem-mqtt-[a-z]+)-(?<version>\d+\.\d+\.\d+)-(?<platform>.+)\.gem$' | first)
        let resp = (http get --full --allow-errors $"https://rubygems.org/api/v1/versions/($parts.gem).json")
        let versions = if $resp.status == 200 { $resp.body } else { [] }
        if (published $versions $parts.version $parts.platform) {
            print $"⏭️  ($file | path basename) is already on rubygems.org — skipping."
            continue
        }
        print $"▶ gem push ($file | path basename)"
        let result = (do { ^gem push $file } | complete)
        print $result.stdout
        if $result.exit_code != 0 {
            # Another release run may have pushed it in the meantime.
            if ($"($result.stdout)($result.stderr)" | str contains "Repushing of gem versions is not allowed") {
                print "⏭️  pushed meanwhile — skipping."
                continue
            }
            print -e $result.stderr
            error make --unspanned { msg: $"gem push failed for ($file | path basename)" }
        }
    }
}

def main [] {
    print "usage: nu scripts/ci/ruby_gem.nu build <crate> <version> <rust-target> <native-lib> <generated.rb> <out-dir> | publish <dir>"
}
