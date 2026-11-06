use std/assert
use ../ci/prune_releases.nu [from_gh from_gitea doomed newest]
use runner.nu run-tests

const GITEA_RELEASES = [
    { id: 3, tag_name: "v1.2.0", assets: [{ id: 30, name: "app-x86_64.tar.gz" }] }
    { id: 2, tag_name: "v1.1.0", assets: [{ id: 20, name: "app-x86_64.tar.gz" } { id: 21, name: "app.deb" }] }
    { id: 1, tag_name: "v1.0.0", assets: [] }
]

def "test prune releases: gitea deletes every asset but the kept release" [] {
    let d = doomed (from_gitea $GITEA_RELEASES) "v1.2.0"
    assert equal ($d | select release id) [{ release: 2, id: 20 } { release: 2, id: 21 }]
}

def "test prune releases: github assets are addressed by tag and name" [] {
    let gh = [{ tagName: "v2", assets: [{ name: "a.zip" }] } { tagName: "v1", assets: [{ name: "a.zip" }] }]
    assert equal (doomed (from_gh $gh) "v2" | select release name) [{ release: "v1", name: "a.zip" }]
}

def "test prune releases: refuses an unknown kept tag" [] {
    assert error { doomed (from_gitea $GITEA_RELEASES) "v9.9.9" }
}

def "test prune releases: refuses when the kept release has no assets" [] {
    assert error { doomed (from_gitea $GITEA_RELEASES) "v1.0.0" }
}

# A manual re-run of v0.4.1 after v0.4.2 must not delete the v0.4.2 binaries.
def "test prune releases: newest compares versions not text" [] {
    let rels = [{ tag: "v0.4.1" } { tag: "v0.10.0" } { tag: "v0.10.0-rc1" } { tag: "nightly" }]
    assert equal (newest $rels) "v0.10.0"
    assert equal (newest ($rels | where tag != "v0.10.0")) "v0.10.0-rc1"
    assert equal (newest [{ tag: "nightly" }]) null
}

def main [] { run-tests }
