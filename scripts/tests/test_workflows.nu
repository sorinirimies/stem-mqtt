use std/assert
use runner.nu *

def root [] { $env.CURRENT_FILE | path dirname | path join ".." ".." }

def workflows [] {
    [".github" ".gitea"] | each {|h| glob (root | path join $h "workflows" "*.yml") } | flatten
}

def runs [wf: string] {
    open $wf | get jobs | values | each {|j| $j.steps? | default [] | each {|s| $s.run? | default "" } } | flatten
}

# Regression: setup-beam on Gitea installed the ubuntu-20.04 OTP, whatever ImageOS the
# workflow set (job env, then step env: the act runner forces ubuntu20), and :crypto
# failed on the runner's OpenSSL 3. Gitea installs the BEAM with install_beam.nu.
def "test workflows: gitea installs the beam without setup-beam" [] {
    let steps = (open (root | path join .gitea workflows ci.yml)).jobs.test-elixir.steps
    assert not ($steps | any {|s| ($s.uses? | default "") | str starts-with "erlef/setup-beam" })
    let install = ($steps | where {|s| ($s.run? | default "") == "nu scripts/ci/install_beam.nu" })
    assert equal ($install | length) 1
    let names = ($steps | each {|s| $s.run? | default "" })
    assert (($names | enumerate | where item == "nu scripts/ci/install_beam.nu" | first).index < ($names | enumerate | where item == "nu scripts/ci/elixir.nu test" | first).index)
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

# Either host publishes to the public registries (npmjs, NuGet, Maven Central, Hackage,
# Go): only Gitea's release did, and GitHub-only tags never reached npm (v0.4.1).
# Both hosts publish the public registries, except npm and Maven: their packages are one
# per platform, and only GitHub builds every platform's libraries (Gitea's v0.7.0 release
# would have published Linux-only packages had it been first).
def "test workflows: both hosts publish the public registries, npm and maven from github only" [] {
    let matrix = {|h| (open (root | path join $h workflows release.yml)).jobs.publish-public.strategy.matrix.language }
    let github = (do $matrix .github)
    assert equal (do $matrix .gitea) ($github | where $it not-in [node java])
    assert ("node" in $github and "java" in $github)
    assert not (root | path join .gitea workflows publish-npm.yml | path exists)
    let npm = (open (root | path join .github workflows publish-npm.yml))
    assert ($npm.jobs.build-native-libs.uses | str ends-with "native-libs.yml")
    assert ($npm.jobs.publish.steps | any {|s| ($s.run? | default "") | str contains "--natives native-libs" })
}

# Regression: on Linux, `cargo test --workspace --all-targets` failed to link a
# test executable for the NIF (undefined enif_* symbols).
def "test workflows: workspace tests exclude the nif" [] {
    let files = [.github/workflows/ci.yml .gitea/workflows/ci.yml .gitea/workflows/deps-update.yml justfile scripts/ci/quality_gate.nu scripts/check_publish.nu]
    for f in $files {
        let lines = open --raw (root | path join $f) | lines | where {|l| $l =~ 'cargo test --workspace' and $l !~ 'name:|print' }
        for l in $lines {
            assert ($l | str contains "--exclude stem_mqtt_nif") $"($f): ($l | str trim)"
        }
    }
}

# The per-platform native libraries are built after the release (the release attaches every
# artifact that exists when it runs), for every target ruby_gem.nu and native_libs.nu map,
# and for every UniFFI release a per-platform package (Ruby, npm, Maven) is built against.
def "test workflows: native libraries follow the release and cover every platform" [] {
    use ../ci/ruby_gem.nu [PLATFORMS]
    use ../ci/native_libs.nu [TARGETS]
    use ../bindings/spec.nu [spec-for]
    let gh = (open (root | path join .github workflows release.yml)).jobs
    assert ("release" in $gh.build-native-libs.needs) "native library artifacts would be attached to the release"
    assert equal $gh.build-native-libs.uses "./.github/workflows/native-libs.yml"
    let job = (open (root | path join .github workflows native-libs.yml)).jobs.build
    assert equal ($job.strategy.matrix.target | sort) ($PLATFORMS | columns | sort)
    assert equal ($job.strategy.matrix.target | sort) ($TARGETS | columns | sort)
    assert equal ($job.strategy.matrix.include | get target | sort) ($TARGETS | columns | sort)
    let needed = ([ruby java node] | each { |l| (spec-for $l).uniffi } | uniq | sort)
    assert equal ($job.strategy.matrix.uniffi | sort) $needed
    for consumer in [publish-ruby publish-github-packages publish-public] {
        assert ("build-native-libs" in ($gh | get $consumer).needs) $"($consumer) runs before the libraries exist"
    }
}

# Ruby gems are built from the workspace UniFFI libraries.
def "test workflows: ruby gems use the workspace libraries" [] {
    let steps = (open (root | path join .github workflows release.yml)).jobs.publish-ruby.steps
    assert ($steps | any { |s| ($s.with?.pattern? | default "") == "native-workspace-*" })
    assert ($steps | any { |s| ($s.run? | default "") | str contains "native-libs/workspace" })
}

# The stem_mqtt Hex package downloads its NIFs from the GitHub release, so only
# GitHub publishes it, after the release that carries them; Gitea never does.
def "test workflows: hex publishes from github after the release with every nif" [] {
    let gh = (open (root | path join .github workflows release.yml)).jobs
    assert ("build-nif" in $gh.release.needs) "the release does not wait for the NIFs"
    assert ("release" in $gh.publish-hex.needs) "hex publishes before the release exists"
    let native = open --raw (root | path join packaging elixir lib stem_mqtt native.ex)
    let built = $gh.build-nif.strategy.matrix.include | get target | sort
    let listed = $native | parse --regex '(?s)targets: ~w\((?<t>[^)]*)\)' | get 0.t | split row --regex '\s+' | where $it != "" | sort
    assert equal $listed $built "native.ex targets and the build-nif matrix differ"
    let gitea = open --raw (root | path join .gitea workflows release.yml)
    assert (not ($gitea | str contains "elixir.nu publish")) "gitea must not publish to hex"
}

# v0.4.1: `gradle publish` for GitHub Packages also staged unsigned artifacts, and
# Maven Central refused the bundle ("missing: stem-mqtt-kotlin-0.4.1.jar.asc").
def "test workflows: maven central stages only signed artifacts" [] {
    for h in [.github .gitea] {
        let steps = (open (root | path join $h workflows release.yml)).jobs.publish-kotlin.steps
        let runs = $steps | where {|s| ($s.working-directory? | default "") == "packaging/kotlin" } | each {|s| $s.run? | default "" }
        assert ($runs | all {|r| not ($r =~ 'gradle publish(\s|$)') }) $"($h): a plain gradle publish stages unsigned artifacts"
        let central = $runs | where {|r| $r | str contains publishAllPublicationsToStagingRepository } | first
        assert ($central | str starts-with "rm -rf build/staging-repo") $"($h): staging is not cleared first"
    }
}

# v0.4.1: a re-run hit 409 Conflict at GitHub Packages and never reached Maven Central.
def "test workflows: package registry publishes tolerate an existing version" [] {
    for h in [.github .gitea] {
        let steps = (open (root | path join $h workflows release.yml)).jobs | values | each {|j| $j.steps? | default [] } | flatten
        let runs = $steps | each {|s| $s.run? | default "" } | where {|r| $r =~ '(gradle publishAllPublicationsToPackagesRepository|gradle publish |twine upload --repository-url)' }
        assert ($runs | is-not-empty) $"($h): no package registry publish found"
        assert ($runs | all {|r| ($r | str contains publish_once.nu) or ($r | str contains "--skip-existing") }) $"($h): a registry publish fails on an existing version"
    }
}

# A release re-run builds the tag, whose CI scripts may be the broken ones (v0.4.1 could
# not reach Maven Central). `source` lets a manual run build from a fixed ref.
def "test workflows: release jobs check out the resolved ref" [] {
    for h in [.github .gitea] {
        let wf = open (root | path join $h workflows release.yml)
        assert ($wf.on.workflow_dispatch.inputs | columns | any {|c| $c == source }) $"($h): no source input"
        let refs = $wf.jobs | values | each {|j| $j.steps? | default [] } | flatten | where {|s| ($s.uses? | default "") starts-with "actions/checkout" } | each {|s| $s.with?.ref? } | compact
        assert ($refs | is-not-empty)
        assert ($refs | all {|r| $r == '${{ needs.validate.outputs.ref }}' }) $"($h): a job checks out something else"
    }
}

# GitHub publishes to PyPI with Trusted Publishing; the PyPI projects trust exactly
# this workflow file and the `pypi` environment.
def "test workflows: pypi uses trusted publishing" [] {
    let wf = root | path join .github workflows release.yml
    let job = open $wf | get jobs.publish-python
    assert equal $job.environment pypi
    assert equal ($job.permissions | get id-token) write
    let pubs = $job.steps | where {|s| ($s.uses? | default "") starts-with "pypa/gh-action-pypi-publish" }
    assert equal ($pubs | each {|s| $s.with | get packages-dir }) [dist/client dist/broker]
    assert ($pubs | all {|s| $s.with | get skip-existing })
    assert not (open --raw $wf | str contains "PYPI_API_TOKEN")
}

# Only the newest release keeps its binaries, on both hosts, right after the release
# is created (notes and tags stay).
def "test workflows: releases prune older binaries" [] {
    for h in [.github .gitea] {
        let steps = open (root | path join $h workflows release.yml) | get jobs.release.steps
        let names = $steps | get -o name
        let create = $names | enumerate | where {|s| ($s.item | default "") =~ "^Create (GitHub|Gitea) Release$" } | get index | first
        let prune = $names | enumerate | where item == "Keep binaries only on the newest release" | get index
        assert equal ($prune | length) 1 $"($h): no prune step"
        assert ($prune.0 > $create) $"($h): prune runs before the release exists"
        assert str contains ($steps | get $prune.0 | get run) "scripts/ci/prune_releases.nu ${{ needs.validate.outputs.tag }}"
    }
}

def "test workflows: the msrv job pins the toolchain from Cargo.toml without a version ref" [] {
    let cargo = open (root | path join Cargo.toml)
    let msrv = $cargo | get -o workspace.package.rust-version | default ($cargo | get -o package.rust-version)
    let step = open (root | path join .github workflows ci.yml) | get jobs.msrv.steps
        | where {|s| ($s | get -o uses | default "") | str starts-with "dtolnay/rust-toolchain" } | first
    # A version ref (@1.90) is something Dependabot bumps; @master + toolchain is not.
    assert equal $step.uses "dtolnay/rust-toolchain@master"
    assert equal ($step.with.toolchain | into string) $msrv
}

# Gitea checks the MSRV too, with the same toolchain and commands as GitHub.
def "test workflows: gitea checks the msrv like github" [] {
    let gh = open (root | path join .github workflows ci.yml) | get jobs.msrv.steps
    let gt = open (root | path join .gitea workflows ci.yml) | get jobs.msrv.steps
    let msrv = $gh | where {|s| ($s.with?.toolchain? | default null) != null } | first | get with.toolchain | into string
    let install = $gt | where {|s| ($s.name? | default "") starts-with "Install Rust" } | first
    assert ($install.run | str contains $"--default-toolchain ($msrv) ") "gitea installs the msrv toolchain"
    let checks = {|steps| $steps | where {|s| ($s.run? | default "") starts-with "cargo " } | get run }
    assert equal (do $checks $gt) (do $checks $gh)
}

# Every language test_bindings.nu smoke-tests runs in CI, on both hosts.
def "test workflows: every smoke-tested language runs in ci on both hosts" [] {
    use ../test_bindings.nu [TESTED]
    for host in [.github .gitea] {
        let matrix = (open (root | path join $host workflows ci.yml)).jobs.test-bindings-runtime.strategy.matrix.language
        assert equal ($matrix | sort) ($TESTED | sort) $"($host) runtime matrix"
    }
}

def main [] { run-tests }
