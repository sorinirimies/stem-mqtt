use std/assert
use ../ci/registry_readme.nu [registry-readme stale-pypi PYPI_READMES OWNER]
use runner.nu run-tests

const ROOT = path self ../..

# A throwaway repository holding one guide.
def fixture [guide: string]: nothing -> string {
    let root = mktemp -d
    mkdir ($root | path join docs languages)
    $guide | save ($root | path join docs languages python.md)
    for f in $PYPI_READMES { mkdir ($root | path join $f | path dirname) }
    $root
}

def "test registry readme: repo links point at github" [] {
    let root = fixture "See [concepts](README.md) and [the example](../../examples/python/main.py)."
    let out = registry-readme python acme --root $root
    assert str contains $out "(https://github.com/acme/stem-mqtt/blob/main/docs/languages/README.md)"
    assert str contains $out "(https://github.com/acme/stem-mqtt/blob/main/examples/python/main.py)"
    assert not ($out | str contains "](../../")
    rm -rf $root
}

def "test registry readme: missing or edited pypi readmes are stale" [] {
    let root = fixture "# Python"
    assert equal (stale-pypi --root $root) $PYPI_READMES
    for f in $PYPI_READMES { registry-readme python $OWNER --root $root | save ($root | path join $f) }
    assert equal (stale-pypi --root $root) []
    "# Rust" | save --force ($root | path join ($PYPI_READMES | first))
    assert equal (stale-pypi --root $root) [($PYPI_READMES | first)]
    rm -rf $root
}

# PyPI shows the README pyproject.toml names. It showed the Rust crate README before.
def "test registry readme: pypi shows the python guide" [] {
    for f in $PYPI_READMES {
        let crate = $ROOT | path join ($f | path dirname)
        assert equal (open ($crate | path join pyproject.toml) | get project.readme) ($f | path basename)
    }
    assert equal (stale-pypi) [] "run: just registry-readmes"
}

def main [] { run-tests }
