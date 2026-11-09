use std/assert
use ../ci/install_beam.nu [os-build arch-build otp-url elixir-url OTP ELIXIR]
use runner.nu run-tests

const JAMMY = 'PRETTY_NAME="Ubuntu 22.04.5 LTS"
NAME="Ubuntu"
VERSION_ID="22.04"
ID=ubuntu
ID_LIKE=debian'

# Regression: setup-beam on Gitea installed the ubuntu-20.04 OTP (ImageOS forced by the act
# runner), whose :crypto needs OpenSSL 1.1; the build now follows the runner's own OS.
def "test install beam: the otp build follows os-release, not ImageOS" [] {
    assert equal (os-build $JAMMY) "ubuntu-22.04"
    assert equal (os-build ($JAMMY | str replace '"22.04"' '"24.04"')) "ubuntu-24.04"
}

def "test install beam: a newer ubuntu gets the newest build, an older or other os is an error" [] {
    assert equal (os-build ($JAMMY | str replace '"22.04"' '"26.04"')) "ubuntu-24.04"
    assert error { os-build ($JAMMY | str replace '"22.04"' '"18.04"') }
    assert error { os-build ($JAMMY | str replace 'ID=ubuntu' 'ID=alpine') }
}

def "test install beam: machines map to the hex build arch" [] {
    assert equal (arch-build "x86_64") "amd64"
    assert equal (arch-build "aarch64") "arm64"
    assert error { arch-build "riscv64" }
}

def "test install beam: urls of the pinned builds" [] {
    assert equal (otp-url "amd64" "ubuntu-24.04") $"https://builds.hex.pm/builds/otp/amd64/ubuntu-24.04/OTP-($OTP).tar.gz"
    assert equal (elixir-url "1.18.6" "27.3.4.10") "https://builds.hex.pm/builds/elixir/v1.18.6-otp-27.zip"
    assert ((elixir-url) | str ends-with $"v($ELIXIR)-otp-27.zip")
}

def main [] { run-tests }
