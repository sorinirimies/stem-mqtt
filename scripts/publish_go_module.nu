#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — publish the Go bindings as a Go module
# ──────────────────────────────────────────────────────────────────────────────
# Go has no upload step: a module is "published" by tagging a git repository, after
# which proxy.golang.org / pkg.go.dev pick it up. The module lives in the `go/`
# subdirectory of this repository (github.com/<owner>/stem-mqtt/go), so no repository of its
# own: the generated Go is committed under `go/` on a commit that holds nothing else, and
# only the tag `go/v<version>` is pushed. main never receives generated code.
#
# Needs a token that can push tags to the repository: in GitHub Actions, GITHUB_TOKEN with
# `contents: write` (never passed as an argument).
#
# Usage: nu scripts/publish_go_module.nu <stage> <version> [--owner O] [--dry-run]
#   <stage>   the directory produced by `publish_packages.nu stage go`
# ──────────────────────────────────────────────────────────────────────────────

use bindings/go_module.nu *

def main [stage: string, version: string, --owner: string = "sorinirimies", --dry-run] {
    let module = (go-module-path $owner)
    let tag = (go-module-tag $version)
    let work = ($env.PWD | path join "target" "go-module")
    rm -rf $work
    build-go-module ($stage | path join "sources") ($work | path join "repo" "go") $module
    print $"==> assembled ($module) ($tag) at ($work | path join 'repo' 'go')"
    if $dry_run { return }

    let token = ($env.GITHUB_TOKEN? | default "")
    if $token == "" { error make { msg: "GITHUB_TOKEN is not set (a token that can push tags to the repository)" } }
    let remote = $"https://x-access-token:($token)@github.com/($owner)/stem-mqtt.git"
    # A re-run finds the tag already there.
    if (^git ls-remote --tags $remote $"refs/tags/($tag)" | str trim | is-not-empty) {
        print $"==> ($tag) is already on the repository, nothing to publish."
        return
    }
    cd ($work | path join "repo")
    ^git init --quiet
    ^git add -A
    ^git -c user.name="stem-mqtt release" -c user.email="release@stem-mqtt.invalid" commit --quiet -m $"stem-mqtt Go module v($version)"
    ^git tag $tag
    ^git push --quiet $remote $"refs/tags/($tag)"
    print $"==> pushed ($tag); it appears on pkg.go.dev after the first `go get ($module)@v($version)`"
}
