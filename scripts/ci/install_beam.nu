#!/usr/bin/env nu
# Install Erlang/OTP and Elixir from builds.hex.pm (the builds erlef/setup-beam installs) and put
# them on GITHUB_PATH. Used on Gitea instead of setup-beam: that picks the OTP build from ImageOS,
# and Gitea's act runner forces ImageOS=ubuntu20 for ubuntu-latest whatever the workflow sets
# (job or step env). The ubuntu-20.04 OTP links OpenSSL 1.1, so :crypto, and with it
# `mix local.rebar`, fails on the runner's OpenSSL 3. Here the build follows /etc/os-release.
#
#   nu scripts/ci/install_beam.nu            # into ~/.local/beam, then GITHUB_PATH

export const OTP = "27.3.4.10"
export const ELIXIR = "1.18.6"

# The Ubuntu releases builds.hex.pm has OTP builds for.
const UBUNTU = ["20.04" "22.04" "24.04"]

# builds.hex.pm's name of the OS in /etc/os-release, e.g. `ubuntu-24.04`. A newer Ubuntu
# than the builds gets the newest build (the same OpenSSL 3 ABI).
export def os-build [os_release: string]: nothing -> string {
    let fields = ($os_release | lines | where $it =~ '=' | parse "{key}={value}"
        | update value { str trim --char '"' } | transpose --header-row --as-record)
    let id = ($fields.ID? | default "")
    let version = ($fields.VERSION_ID? | default "")
    if $id != "ubuntu" {
        error make --unspanned { msg: $"no builds.hex.pm OTP build for ($id) ($version); Ubuntu only" }
    }
    if $version in $UBUNTU {
        $"ubuntu-($version)"
    } else if ($version | into float) > ($UBUNTU | last | into float) {
        $"ubuntu-($UBUNTU | last)"
    } else {
        error make --unspanned { msg: $"no builds.hex.pm OTP build for Ubuntu ($version)" }
    }
}

# builds.hex.pm's name of a machine (`uname -m`).
export def arch-build [machine: string]: nothing -> string {
    match $machine {
        "x86_64" | "amd64" => "amd64"
        "aarch64" | "arm64" => "arm64"
        $other => { error make --unspanned { msg: $"no builds.hex.pm OTP build for ($other)" } }
    }
}

export def otp-url [arch: string, os: string, version: string = $OTP]: nothing -> string {
    $"https://builds.hex.pm/builds/otp/($arch)/($os)/OTP-($version).tar.gz"
}

export def elixir-url [version: string = $ELIXIR, otp: string = $OTP]: nothing -> string {
    $"https://builds.hex.pm/builds/elixir/v($version)-otp-($otp | split row '.' | first).zip"
}

# Download with retries: the runner's network to the CDNs is flaky.
def fetch [url: string, file: string] {
    for attempt in 1..5 {
        let r = (^curl -fsSL --retry 3 --retry-all-errors -o $file $url | complete)
        if $r.exit_code == 0 { return }
        print $"download of ($url) failed \(attempt ($attempt)\); retrying in 15s"
        sleep 15sec
    }
    error make --unspanned { msg: $"could not download ($url)" }
}

def main [--root: string = ""] {
    let root = if $root != "" { $root } else { $env.HOME | path join .local beam }
    let os = (os-build (open --raw /etc/os-release))
    let arch = (arch-build (^uname -m | str trim))
    let tmp = (mktemp -d)

    print $"▶ Erlang/OTP ($OTP) for ($arch)/($os)"
    let otp_dir = ($root | path join otp)
    fetch (otp-url $arch $os) ($tmp | path join otp.tar.gz)
    ^tar -xzf ($tmp | path join otp.tar.gz) -C $tmp
    rm -rf $otp_dir
    mv ($tmp | path join $"OTP-($OTP)") $otp_dir
    do { cd $otp_dir; ^./Install -minimal $otp_dir }

    print $"▶ Elixir ($ELIXIR)"
    let elixir_dir = ($root | path join elixir)
    fetch (elixir-url) ($tmp | path join elixir.zip)
    rm -rf $elixir_dir
    ^unzip -q ($tmp | path join elixir.zip) -d $elixir_dir
    rm -rf $tmp

    let bins = [($otp_dir | path join bin) ($elixir_dir | path join bin)]
    with-env { PATH: ($bins | append $env.PATH) } {
        # :crypto is what the wrong build broke; fail here, not in mix.
        ^erl -noshell -eval 'ok = crypto:start(), io:format("OTP ~s, crypto ok~n", [erlang:system_info(otp_release)]), halt().'
        ^elixir --version
    }
    if ($env.GITHUB_PATH? | default "") != "" {
        ($bins | str join "\n") + "\n" | save --append $env.GITHUB_PATH
    }
}
