# Bundle the Rust sources a source package (Hackage, pub.dev) compiles at install time.

# Copy the two library crates + a workspace manifest/lockfile trimmed to exactly those crates
# (no demo/dashboard or fuzz members, no benches) into <dest>. The lockfile is pruned with
# `cargo update --workspace` so `cargo build --locked` works on the trimmed workspace.
export def bundle-rust-sources [root: string, dest: string] {
    mkdir ($dest | path join "crates")
    open --raw ($root | path join "Cargo.toml")
    | str replace --regex '(?s)members\s*=\s*\[.*?\]' 'members = ["crates/mqtt-client", "crates/mqtt-broker"]'
    | save --force ($dest | path join "Cargo.toml")
    cp ($root | path join "Cargo.lock") $dest
    for crate in [mqtt-client mqtt-broker] {
        let out = ($dest | path join "crates" $crate)
        mkdir $out
        # Benches are dev-only and aren't shipped, so drop their `[[bench]]` tables —
        # Cargo refuses a manifest that names a bench file which isn't there.
        open --raw ($root | path join "crates" $crate "Cargo.toml")
        | str replace --all --regex '\[\[bench\]\]\n(?:[^\[\n][^\n]*\n|\n)*' ''
        | save --force ($out | path join "Cargo.toml")
        cp -r ($root | path join "crates" $crate "src") $out
        let readme = ($root | path join "crates" $crate "README.md")
        if ($readme | path exists) { cp $readme $out }
    }
    # Drop lockfile entries of the removed members, without touching any pinned version.
    let pruned = (do { cd $dest; ^cargo update --workspace --offline } | complete)
    if $pruned.exit_code != 0 {
        let online = (do { cd $dest; ^cargo update --workspace } | complete)
        if $online.exit_code != 0 { error make { msg: $"cargo update --workspace failed: ($online.stderr)" } }
    }
    rm -rf ($dest | path join "target")
}
