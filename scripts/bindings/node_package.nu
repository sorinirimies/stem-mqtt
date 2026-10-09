# Lay out the Node package: the two generated components side by side, sharing ONE runtime.
#
# `uniffi-bindgen-node-js` emits every component as a self-contained package with its own copy of
# `runtime/`. The copies are identical, and each registers named types (`RustBuffer`,
# `CallbackInterfaceFree`, …) in koffi's process-wide registry — so loading the client and the
# broker from one install (one koffi) fails with "Duplicate type name 'RustBuffer'". Sharing a
# single runtime makes its name-deduplicating caches work across both components.
#
#   <dest>/client/…  <dest>/broker/…  <dest>/runtime/…
export def assemble-node-package [client_gen: string, broker_gen: string, dest: string] {
    rm -rf $dest
    mkdir $dest
    cp -r $client_gen ($dest | path join "client")
    cp -r $broker_gen ($dest | path join "broker")

    let client_runtime = ($dest | path join "client" "runtime")
    let broker_runtime = ($dest | path join "broker" "runtime")
    # The copies must be byte-identical, or sharing one would silently change a component's behaviour.
    for file in (ls $client_runtime | get name | path basename) {
        if (open --raw ($client_runtime | path join $file)) != (open --raw ($broker_runtime | path join $file)) {
            error make { msg: $"the generated runtimes differ in ($file): cannot share one runtime" }
        }
    }
    mv $client_runtime ($dest | path join "runtime")
    rm -rf $broker_runtime

    for part in [client broker] {
        for file in (glob ($dest | path join $part "*.{js,ts}")) {
            open --raw $file | str replace --all '"./runtime/' '"../runtime/' | save --force $file
        }
    }
}
