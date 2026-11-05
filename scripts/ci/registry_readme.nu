# Registry READMEs from the language guides (docs/languages/<language>.md).
#
# A registry page should show the guide for the language it serves, not the Rust crate README.
# The guides link relative to the repository, which breaks on pypi.org / npmjs.com / nuget.org,
# so those links are pointed at the repository on GitHub.
#
#   nu scripts/ci/registry_readme.nu pypi          # regenerate the committed PyPI READMEs
#   nu scripts/ci/registry_readme.nu pypi --check  # fail when they are out of date

const ROOT = path self ../..

# The PyPI READMEs, named by `readme` in crates/<crate>/pyproject.toml.
export const PYPI_READMES = ["crates/mqtt-client/README.pypi.md" "crates/mqtt-broker/README.pypi.md"]

# The repository owner the committed READMEs link to.
export const OWNER = "sorinirimies"

# `docs/languages/<language>.md` as a registry README, links made absolute.
export def registry-readme [language: string, owner: string, --root: string = $ROOT]: nothing -> string {
    let base = $"https://github.com/($owner)/stem-mqtt/blob/main"
    open --raw ($root | path join "docs" "languages" $"($language).md")
    | str replace --all "](../../" $"]\(($base)/"
    | str replace --all --regex '\]\(([A-Za-z_-]+\.md)\)' $"]\(($base)/docs/languages/${1}\)"
}

# Committed PyPI READMEs that differ from the Python guide.
export def stale-pypi [--root: string = $ROOT]: nothing -> list<string> {
    let want = registry-readme python $OWNER --root $root
    $PYPI_READMES | where {|f|
        let p = $root | path join $f
        not ($p | path exists) or (open --raw $p) != $want
    }
}

def main [] { print -e "usage: nu scripts/ci/registry_readme.nu pypi [--check]" }

# Write (or with --check, verify) the PyPI READMEs.
def "main pypi" [--check] {
    if $check {
        let stale = stale-pypi
        if ($stale | is-not-empty) {
            print -e $"❌ out of date: ($stale | str join ', '). Run: just registry-readmes"
            exit 1
        }
        print "pypi_readmes=current"
        return
    }
    let text = registry-readme python $OWNER
    for f in $PYPI_READMES { $text | save --force --raw ($ROOT | path join $f) }
    print $"pypi_readmes=($PYPI_READMES | length)"
}
