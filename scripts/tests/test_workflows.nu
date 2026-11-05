use std/assert
use runner.nu *

def root [] { $env.CURRENT_FILE | path dirname | path join ".." ".." }

def workflows [] {
    [".github" ".gitea"] | each {|h| glob (root | path join $h "workflows" "*.yml") } | flatten
}

def runs [wf: string] {
    open $wf | get jobs | values | each {|j| $j.steps? | default [] | each {|s| $s.run? | default "" } } | flatten
}

# A GitHub 504 on the Nushell download failed whole CI and release runs (v0.4.0).
def "test workflows: the nushell download retries and never pipes into tar" [] {
    for wf in (workflows) {
        for r in (runs $wf | where {|r| $r | str contains "nushell/nushell/releases/download" }) {
            assert not ($r =~ 'releases/download/\S+ \| tar') $"($wf): curl | tar has no retry"
            assert ($r | str contains "--retry-all-errors") $"($wf): the download does not retry"
            assert ($r | str contains "for attempt in") $"($wf): the download is tried once"
        }
    }
}

# dart pub honours the repo's .gitignore: a package staged under the ignored dist/
# publishes empty ("The pubspec is hidden").
def "test workflows: pub.dev stages the dart package outside the checkout" [] {
    let wf = root | path join .github workflows publish-pubdev.yml
    let steps = (open $wf).jobs.publish.steps
    let stage = $steps | where {|s| ($s.run? | default "") | str contains "publish_packages.nu stage dart" } | first
    assert ($stage.run | str contains "--out \"${{ runner.temp }}/dart\"")
    assert (runs $wf | all {|r| not ($r | str contains "dist/dart") })
    let validate = $steps | where name? == Validate | first
    assert equal $validate.working-directory "${{ runner.temp }}/dart/pub"
}

# A job that runs nu (directly or through a script that calls it) must install it:
# build-swift ran build_xcframework.sh without it ("nu: command not found", v0.4.0).
def "test workflows: every job that runs nu installs it" [] {
    for wf in (workflows) {
        for job in (open $wf | get jobs | transpose name job) {
            let steps = $job.job.steps? | default []
            let cmds = $steps | each {|s| $s.run? | default "" }
            let scripts = $cmds | each {|c| $c | parse --regex '(?P<p>\./[\w./-]+\.sh)' | get p } | flatten
            let via_script = $scripts | any {|p| (root | path join $p | path exists) and ((open --raw (root | path join $p)) =~ '(?m)^\s*nu\s') }
            let direct = $cmds | any {|c| $c =~ '(?m)(^|[;&|]\s*)nu\s' }
            if $direct or $via_script {
                let installs = $steps | any {|s| (($s.uses? | default "") | str contains "setup-nu") or (($s.name? | default "") | str starts-with "Install Nushell") }
                assert $installs $"($wf | path basename) job ($job.name) runs nu without installing it"
            }
        }
    }
}

# The dart runtime test needs the Dart SDK ("skipped: dart not found" failed v0.4.0).
def "test workflows: staging dart installs the dart sdk" [] {
    let steps = (open (root | path join .github workflows release.yml)).jobs.stage-packages.steps
    let dart = $steps | where {|s| ($s.uses? | default "") | str starts-with "dart-lang/setup-dart" }
    assert ($dart | is-not-empty)
    assert equal ($dart | first | get if) "matrix.language == 'dart'"
}

# A newer release run cancelled v0.4.1 mid-run; releases queue instead.
def "test workflows: releases are never cancelled" [] {
    for wf in (workflows | where {|p| ($p | path basename) == "release.yml" }) {
        assert equal ((open $wf).concurrency.cancel-in-progress) false $"($wf) cancels a release"
    }
}

def main [] { run-tests }
