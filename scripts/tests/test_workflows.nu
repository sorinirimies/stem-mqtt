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

def main [] { run-tests }
