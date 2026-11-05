# Runs one publish command; a version the registry already has counts as published.
#
# Releases run on both GitHub and Gitea and can be re-run, so the same version is often
# pushed twice. Registries answer that with a conflict instead of a no-op; without this a
# re-run stopped at GitHub Packages and never reached Maven Central (v0.4.1).
#
# Usage: nu scripts/ci/publish_once.nu <command> [args...]
# Prints `published=yes` or `published=already` on stdout.

# True when a failed publish only means "this version is already there".
export def already-there [output: string]: nothing -> bool {
    $output =~ '(?i)409 Conflict|status code:? 409|Cannot publish over existing version|EPUBLISHCONFLICT|already exists'
}

# Runs `cmd` and maps an "already published" failure to success.
export def publish-once [cmd: list<string>]: nothing -> any {
    let r = do { run-external ($cmd | first) ...($cmd | skip 1) } | complete
    print -e $r.stdout
    print -e $r.stderr
    if $r.exit_code == 0 { return "yes" }
    if (already-there $"($r.stdout)\n($r.stderr)") {
        print -e $"($cmd | first): this version is already published, nothing to do."
        return "already"
    }
    error make { msg: $"($cmd | first) failed with exit code ($r.exit_code)" }
}

def main [...cmd: string] {
    if ($cmd | is-empty) { error make { msg: "usage: nu scripts/ci/publish_once.nu <command> [args...]" } }
    print $"published=(publish-once $cmd)"
}
