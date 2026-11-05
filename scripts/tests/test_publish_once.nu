use std/assert
use ../ci/publish_once.nu [already-there publish-once]
use runner.nu run-tests

def "test publish once: registry conflicts count as published" [] {
    assert (already-there "> Received status code 409 from server: Conflict")
    assert (already-there "npm error 409 Conflict - PUT https://npm.pkg.github.com/x - Cannot publish over existing version")
    assert (already-there "npm error code EPUBLISHCONFLICT")
    assert (already-there "HTTPError: 400 Bad Request: File already exists.")
}

def "test publish once: other failures are not hidden" [] {
    assert not (already-there "> Received status code 401 from server: Unauthorized")
    assert not (already-there "npm error code E403")
}

def "test publish once: a conflicting publish succeeds" [] {
    assert equal (publish-once [nu -c "print -e 'status code 409'; exit 1"]) "already"
    assert equal (publish-once [nu -c "exit 0"]) "yes"
}

def "test publish once: a real failure still fails" [] {
    let failed = try { publish-once [nu -c "print -e 'status code 401'; exit 1"]; false } catch { true }
    assert $failed
}

def main [] { run-tests }
