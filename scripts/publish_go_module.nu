#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — publish the Go bindings as a Go module
# ──────────────────────────────────────────────────────────────────────────────
# Go has no upload step: a module is "published" by tagging a git repository, after
# which proxy.golang.org / pkg.go.dev pick it up. This pushes the generated Go to a
# dedicated module repository and tags it `v<version>`.
#
# One-time setup: create an empty repository (default
# github.com/<owner>/stem-mqtt-go) and a token that can push to it:
#   GO_MODULE_TOKEN   token with push access (never passed as an argument)
#   GO_MODULE_REPO    optional override, e.g. github.com/acme/stem-mqtt-go
#
# Usage: nu scripts/publish_go_module.nu <stage> <version> [--owner O] [--dry-run]
#   <stage>   the directory produced by `publish_packages.nu stage go`
# ──────────────────────────────────────────────────────────────────────────────

use bindings/go_module.nu *

def main [stage: string, version: string, --owner: string = "sorinirimies", --dry-run] {
    let module = ($env.GO_MODULE_REPO? | default $"github.com/($owner)/stem-mqtt-go")
    let work = ($env.PWD | path join "target" "go-module")
    build-go-module ($stage | path join "sources") ($work | path join "module") $module
    print $"==> assembled ($module) v($version) at ($work | path join 'module')"
    if $dry_run { return }

    let token = ($env.GO_MODULE_TOKEN? | default "")
    if $token == "" { error make { msg: "GO_MODULE_TOKEN is not set (token with push access to the module repository)" } }
    let remote = $"https://x-access-token:($token)@($module)"
    let clone = ($work | path join "repo")
    rm -rf $clone
    ^git clone --quiet $remote $clone
    # Replace the repository content with the freshly generated module (keep .git).
    for entry in (ls --all $clone | where { |e| ($e.name | path basename) != ".git" }) { rm -rf $entry.name }
    cp -r ...(glob ($work | path join "module" "*")) $clone
    cd $clone
    ^git add -A
    let changed = (^git status --porcelain | str trim | is-not-empty)
    if $changed {
        ^git -c user.name="stem-mqtt release" -c user.email="release@stem-mqtt.invalid" commit --quiet -m $"stem-mqtt v($version)"
    }
    ^git tag $"v($version)"
    ^git push --quiet origin HEAD $"v($version)"
    print $"==> pushed v($version); it appears on pkg.go.dev after the first `go get ($module)@v($version)`"
}
