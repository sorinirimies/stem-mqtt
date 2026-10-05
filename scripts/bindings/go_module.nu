# Assemble the public Go module (`github.com/<owner>/stem-mqtt-go`).
#
# pkg.go.dev serves Go modules straight from a git repository + tag, so the generated Go
# is committed to a dedicated module repository (see scripts/publish_go_module.nu). The
# generated cgo preamble only says `#include <mqtt_client.h>`; here it gains the `#cgo`
# flags that make `go build` find the headers (shipped next to the sources) and link the
# native library (which users install separately — release assets / `cargo build`).

# <gen>     generated Go (dir holding mqtt_client/ and mqtt_broker/)
# <module>  import path, e.g. github.com/sorinirimies/stem-mqtt-go
export def build-go-module [gen: string, out: string, module: string] {
    let repo = $env.PWD
    rm -rf $out
    mkdir $out
    for pair in [{ pkg: "mqtt_client", lib: "mqtt_client" } { pkg: "mqtt_broker", lib: "mqtt_broker" }] {
        let dest = ($out | path join $pair.pkg)
        cp -r ($gen | path join $pair.pkg) $dest
        let go_file = ($dest | path join $"($pair.pkg).go")
        open --raw $go_file
        | str replace $"// #include <($pair.pkg).h>" $"// #cgo CFLAGS: -I${SRCDIR}\n// #cgo LDFLAGS: -l($pair.lib)\n// #include <($pair.pkg).h>"
        | save --force $go_file
    }
    $"module ($module)\n\ngo 1.21\n" | save --force ($out | path join "go.mod")
    cp ($repo | path join "LICENSE") $out
    cp ($repo | path join "packaging" "go" "README.md") $out
}
