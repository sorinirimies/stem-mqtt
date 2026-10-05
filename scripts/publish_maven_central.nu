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
#   2. generate a user token  → MAVEN_CENTRAL_USERNAME / MAVEN_CENTRAL_PASSWORD
#   3. create a GPG key and publish the public half to keys.openpgp.org
#      → SIGNING_KEY (ASCII-armoured private key) / SIGNING_PASSWORD
#
# Usage:
#   nu scripts/publish_maven_central.nu <staging-dir> <name> [--manual] [--dry-run]
#
#   staging-dir   e.g. packaging/kotlin/build/staging-repo (from
#                 `gradle publishAllPublicationsToStagingRepository -PgroupId=io.github.…`,
#                 run with SIGNING_KEY set)
#   --manual      upload but leave the deployment for you to release in the Portal UI
#                 (default: publish automatically once validation passes)
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

def main [staging: string, name: string, --manual, --dry-run] {
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
    let user = ($env.MAVEN_CENTRAL_USERNAME? | default "")
    let pass = ($env.MAVEN_CENTRAL_PASSWORD? | default "")
    if $user == "" or $pass == "" {
        error make { msg: "MAVEN_CENTRAL_USERNAME / MAVEN_CENTRAL_PASSWORD are not set (Central Portal user token)" }
    }
    let token = ($"($user):($pass)" | encode base64)
    let result = (^curl "--header" $"Authorization: Bearer ($token)" ...$args | complete)
    if $result.exit_code != 0 {
        error make { msg: $"Maven Central upload failed: ($result.stdout) ($result.stderr)" }
    }
    print $"==> uploaded; deployment id: ($result.stdout | str trim)"
    print "    track it at https://central.sonatype.com/publishing/deployments"
}
