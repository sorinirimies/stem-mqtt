# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — one npm package per platform
# ──────────────────────────────────────────────────────────────────────────────
# The main package carries the JavaScript only. Each platform's native libraries
# ship in their own package, `<main>-<target>` (target as the generated loader
# names it: linux-x64-gnu, darwin-arm64, win32-x64, …), restricted to that
# os/cpu and listed in the main package's optionalDependencies, so npm installs
# just the one the machine needs.
# ──────────────────────────────────────────────────────────────────────────────

# The package carrying one platform's libraries: `<main>-<target>`, and on Windows
# `<main>-<target>-msvc` (the ABI, as `-gnu` on Linux; npmjs.org's spam detection also
# rejects the name stem-mqtt-win32-x64). The loader (patch-node-loader) does the same.
export def platform-package-name [main: string, target: string]: nothing -> string {
    if ($target | str starts-with "win32-") { $"($main)-($target)-msvc" } else { $"($main)-($target)" }
}

# package.json of a platform package. `target` is `<os>-<cpu>[-<libc>]`.
export def platform-manifest [main: string, target: string, version: string, owner: string, publish_config: record]: nothing -> record {
    let parts = ($target | split row "-")
    let base = {
        name: (platform-package-name $main $target)
        version: $version
        description: $"Native libraries of ($main) for ($target)"
        os: [($parts | first)]
        cpu: [($parts | get 1)]
        files: ["client" "broker" "README.md" "LICENSE"]
        license: "MIT"
        author: "Sorin Irimies"
        homepage: $"https://github.com/($owner)/stem-mqtt#readme"
        repository: { type: "git", url: $"git+https://github.com/($owner)/stem-mqtt.git" }
        publishConfig: $publish_config
    }
    match ($parts | get --optional 2) {
        "gnu" => ($base | insert libc ["glibc"])
        "musl" => ($base | insert libc ["musl"])
        _ => $base
    }
}

# README.md of a platform package: what it is, and that it is not installed by hand.
# (npmjs.org's spam detection rejected a binaries-only package without one, v0.7.0.)
export def platform-readme [main: string, target: string, owner: string]: nothing -> string {
    [
        $"# (platform-package-name $main $target)"
        ""
        $"The native client and broker libraries of [($main)]\(https://www.npmjs.com/package/($main)\) for `($target)`."
        ""
        $"Do not install it directly: `npm install ($main)` picks the package for your platform"
        "from its optional dependencies."
        ""
        $"Source and documentation: https://github.com/($owner)/stem-mqtt"
        ""
    ] | str join "\n"
}

# The main package's manifest: the platform packages as optional dependencies, and only the
# platforms that have one, so `npm install` elsewhere fails up front (EBADPLATFORM).
export def main-manifest [manifest: record, main: string, targets: list<string>, version: string]: nothing -> record {
    let deps = ($targets | sort | reduce --fold {} { |t, acc| $acc | insert (platform-package-name $main $t) $version })
    let parts = ($targets | each { |t| $t | split row "-" })
    $manifest
    | upsert name $main
    | upsert version $version
    | upsert optionalDependencies $deps
    | upsert os ($parts | each { first } | uniq | sort)
    | upsert cpu ($parts | each { get 1 } | uniq | sort)
}

const LOADER_LINE = '    libraryPath: join(moduleDirectory, "prebuilds", target, filename),'

# Make a generated `*-ffi.js` load its library from the platform package (`<prefix><target>/<part>/`),
# falling back to the package's own prebuilds/ (how the runtime tests run it). Errors if the
# generator's loader changed shape, rather than shipping a package that cannot find its library.
export def patch-node-loader [source: string, prefix: string, part: string]: nothing -> string {
    if not ($source | str contains $LOADER_LINE) {
        error make --unspanned { msg: "the generated Node loader changed: prebuilds/<target> lookup not found" }
    }
    let helper = ([
        'import { createRequire as uniffiCreateRequire } from "node:module";'
        ''
        '// stem-mqtt: the native library ships in a per-platform package (an optional dependency),'
        '// named <prefix><target>, with -msvc on Windows.'
        'function uniffiPlatformLibrary(target, filename) {'
        '  const name = target.startsWith("win32-") ? `${target}-msvc` : target;'
        '  try {'
        ('    const manifest = uniffiCreateRequire(import.meta.url).resolve(`' + $prefix + '${name}/package.json`);')
        ('    return join(dirname(manifest), "' + $part + '", filename);')
        '  } catch {'
        '    return null;'
        '  }'
        '}'
        ''
    ] | str join "\n")
    let patched = ($source | str replace $LOADER_LINE '    libraryPath: uniffiPlatformLibrary(target, filename) ?? join(moduleDirectory, "prebuilds", target, filename),')
    $"($helper)\n($patched)"
}
