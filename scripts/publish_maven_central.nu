#!/usr/bin/env nu
# ──────────────────────────────────────────────────────────────────────────────
# stem-mqtt — publish a Gradle staging repository to Maven Central
# ──────────────────────────────────────────────────────────────────────────────
# Maven Central (via the Sonatype Central Portal) takes a zip of a Maven
# repository layout: every artifact next to its checksums and PGP signature.
# Gradle's built-in `maven-publish` + `signing` produce exactly that into a local
# "staging" repository; this script zips it and uploads it with the Portal's
# publisher API. No third-party Gradle plugin involved.
#
# One-time setup (by a human, on https://central.sonatype.com):
#   1. verify the namespace `io.github.<user>` (create the verification repo it asks for)
#   2. generate a user token  → SONATYPE_TOKEN (paste the Portal's `settings.xml` snippet as is;
#      `user:password` or the ready-made base64 bearer also work), or the two halves
#      separately as MAVEN_CENTRAL_USERNAME / MAVEN_CENTRAL_PASSWORD
#   3. create a GPG key and publish the public half to keys.openpgp.org
#      → SIGNING_KEY (ASCII-armoured private key) / SIGNING_PASSWORD
#
# Usage:
#   nu scripts/publish_maven_central.nu <staging-dir> <name> [--manual] [--dry-run] [--force]
#
#   staging-dir   e.g. packaging/kotlin/build/staging-repo (from
#                 `gradle publishAllPublicationsToStagingRepository -PgroupId=io.github.…`,
#                 run with SIGNING_KEY set)
#   --manual      upload but leave the deployment for you to release in the Portal UI
#                 (default: publish automatically once validation passes)
#   --force       upload even if Central already serves these versions. Without it the upload
#                 is skipped when they are already published, so GitHub and Gitea releases
#                 can both run this without the second one failing as a duplicate.
# ──────────────────────────────────────────────────────────────────────────────

const PORTAL = "https://central.sonatype.com/api/v1/publisher/upload"

# The `curl` arguments (after the bearer header, which carries a secret) for one upload.
export def upload-args [bundle: string, name: string, manual: bool]: nothing -> list<string> {
    let publishing = if $manual { "USER_MANAGED" } else { "AUTOMATIC" }
    [
        "--fail-with-body" "--silent" "--show-error"
        "--form" $"bundle=@($bundle)"
        $"($PORTAL)?name=($name | url encode)&publishingType=($publishing)"
    ]
}

# The Portal's `Authorization: Bearer` value, from whichever credentials were supplied.
# Accepted, in order of preference:
#   - separate username + password (MAVEN_CENTRAL_USERNAME / MAVEN_CENTRAL_PASSWORD)
#   - `token` holding the Portal's `settings.xml` snippet (<username>…</username><password>…</password>)
#   - `token` as `username:password`
#   - `token` already base64-encoded (used verbatim)
export def central-bearer [user: string, pass: string, token: string]: nothing -> string {
    if $user != "" and $pass != "" {
        return ($"($user):($pass)" | encode base64)
    }
    let t = ($token | str trim)
    if $t == "" {
        error make { msg: "no Maven Central credentials: set SONATYPE_TOKEN (or MAVEN_CENTRAL_USERNAME + MAVEN_CENTRAL_PASSWORD)" }
    }
    if ($t | str contains "<username>") {
        let u = ($t | parse --regex '<username>\s*([^<]*?)\s*</username>' | get capture0)
        let p = ($t | parse --regex '<password>\s*([^<]*?)\s*</password>' | get capture0)
        if ($u | is-empty) or ($p | is-empty) or ($u.0 == "") or ($p.0 == "") {
            error make { msg: "SONATYPE_TOKEN looks like a settings.xml snippet but has no <username>/<password>" }
        }
        return ($"($u.0):($p.0)" | encode base64)
    }
    if ($t | str contains ":") {
        return ($t | encode base64)
    }
    $t
}

# The Maven Central URL of every `.pom` in the staging repository — one per published
# artifact version, and the file Central serves once that version is live.
export def central-pom-urls [staging: string]: nothing -> list<string> {
    glob ($staging | path join "**" "*.pom")
    | each { |f| $f | path relative-to $staging | str replace --all '\\' '/' }
    | each { |rel| $"https://repo1.maven.org/maven2/($rel)" }
    | sort
}

# True when Central already serves every artifact version in `staging`.
def already-published [staging: string]: nothing -> bool {
    let urls = (central-pom-urls $staging)
    if ($urls | is-empty) { return false }
    $urls | all { |url|
        let code = (^curl --silent --head --output /dev/null --write-out "%{http_code}" --max-time 20 $url | complete | get stdout | str trim)
        $code == "200"
    }
}

# Every artifact must come with a signature and checksums, or Central rejects the
# whole bundle with an opaque error — so check up front and say what is missing.
export def missing-files [staging: string]: nothing -> list<string> {
    let artifacts = (glob ($staging | path join "**" "*") --no-dir
        | where { |f| ($f | path parse | get extension) in ["jar" "pom" "module"] })
    $artifacts
    | each { |f|
        [".asc" ".md5" ".sha1"]
        | where { |suffix| not ($"($f)($suffix)" | path exists) }
        | each { |suffix| $"($f | path basename)($suffix)" }
    }
    | flatten
}

def main [staging: string, name: string, --manual, --dry-run, --force] {
    let staging = ($staging | path expand)
    if not ($staging | path exists) { error make { msg: $"staging repository not found: ($staging)" } }
    let missing = (missing-files $staging)
    if ($missing | is-not-empty) {
        error make { msg: $"Maven Central needs signatures + checksums; missing: ($missing | first 5 | str join ', ')(if ($missing | length) > 5 { ' …' } else { '' }). Was SIGNING_KEY set when publishing to the staging repository?" }
    }

    let bundle = ($env.PWD | path join "target" $"($name).zip")
    mkdir ($bundle | path dirname)
    rm -f $bundle
    do { cd $staging; ^zip -qr $bundle . }
    print $"==> bundle ($bundle | path basename): ((ls $bundle | get size.0))"

    let args = (upload-args $bundle $name $manual)
    if $dry_run {
        print $"==> would POST to Maven Central: curl ($args | str join ' ')"
        return
    }
    if (not $force) and (already-published $staging) {
        print "==> every artifact version is already on Maven Central — nothing to upload (use --force to override)"
        return
    }
    let token = (central-bearer ($env.MAVEN_CENTRAL_USERNAME? | default "") ($env.MAVEN_CENTRAL_PASSWORD? | default "") ($env.SONATYPE_TOKEN? | default ""))
    let result = (^curl "--header" $"Authorization: Bearer ($token)" ...$args | complete)
    if $result.exit_code != 0 {
        error make { msg: $"Maven Central upload failed: ($result.stdout) ($result.stderr)" }
    }
    print $"==> uploaded; deployment id: ($result.stdout | str trim)"
    print "    track it at https://central.sonatype.com/publishing/deployments"
}
