# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — scratch workspaces pinned to another UniFFI release
# ──────────────────────────────────────────────────────────────────────────────
# Generators that need a different UniFFI release than the workspace are fed a
# library built in target/uniffi-<version>/: the two library crates with
# `uniffi` pinned to exactly that release. The workspace is never modified.
# ──────────────────────────────────────────────────────────────────────────────

use spec.nu [pin-workspace-toml]

# Create (or refresh) target/uniffi-<version>/ under `repo` and return its path.
export def prepare-scratch-workspace [version: string, repo: string = ""]: nothing -> string {
    let repo = if $repo == "" { $env.PWD } else { $repo }
    let root = ($repo | path join "target" $"uniffi-($version)")
    mkdir ($root | path join "crates")
    for crate in (ls ($repo | path join "crates") | where name =~ 'mqtt-(client|broker)$' | get name) {
        let dest = ($root | path join "crates" ($crate | path basename))
        rm -rf $dest
        cp -r $crate $dest
    }
    pin-workspace-toml (open --raw ($repo | path join "Cargo.toml")) $version | save --force ($root | path join "Cargo.toml")
    # Reuse the scratch lockfile between runs so re-resolution stays offline.
    if not ($root | path join "Cargo.lock" | path exists) {
        cp ($repo | path join "Cargo.lock") ($root | path join "Cargo.lock")
    }
    do { cd $root; ^cargo update -p uniffi --precise $version }
    $root
}

# The workspace a language's library is built in: the repo itself, or its scratch workspace.
export def library-root [uniffi: string, repo: string = ""]: nothing -> string {
    let repo = if $repo == "" { $env.PWD } else { $repo }
    if $uniffi == "workspace" { $repo } else { prepare-scratch-workspace $uniffi $repo }
}
