# VHS tape — stem-mqtt · TLS Demo
# Showcases: the blocking client over TLS (`mqtts` example, feature `rustls`) against a broker with
# a TLS listener (`tls_broker` example, throwaway self-signed certificate). Without the broker's CA
# the certificate is refused; with it, a subscriber receives what a publisher sends.
#
# Run: vhs examples/vhs/tls-demo.tape
# (or: just vhs-tape tls-demo)

Output examples/vhs/generated/tls-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 960
Set Height 420
Set FontSize 16
Set PlaybackSpeed 1.5

# ── Build, start the TLS broker in the background, define shorthands (hidden) ─
# The commands are real `cargo run` calls; they are wrapped in short functions so the recording
# shows what matters (the arguments), not a screenful of cargo flags being typed.
Hide
Type "set +m"
Enter
Type "mqtts() { cargo run -q -p stem-mqtt-client-blocking --features rustls --example mqtts -- $@; }"
Enter
Type "cargo build -q -p stem-mqtt-broker --example tls_broker && cargo build -q -p stem-mqtt-client-blocking --features rustls --example mqtts && echo RE''ADY"
Enter
Wait+Screen@300s /READY/
Type "cargo run -q -p stem-mqtt-broker --example tls_broker > /dev/null 2>&1 &"
Enter
Sleep 3s
Type "CA=${TMPDIR:-/tmp}/stem-mqtt-tls/ca.pem; rm -f /tmp/stem-mqtt-tls-sub.log"
Enter
Type "clear"
Enter
Show

# ── 1. No trusted CA: the broker's self-signed certificate is refused ────────
Type "mqtts pub localhost 8883 home/light on"
Sleep 500ms
Enter
Sleep 2s

# ── 2. Trust the broker's CA: subscribe in the background, then publish ──────
Type "mqtts sub localhost 8883 'home/#' $CA > /tmp/stem-mqtt-tls-sub.log 2>&1 &"
Sleep 500ms
Enter
Sleep 2s
Type "mqtts pub localhost 8883 home/light on $CA && echo published over TLS"
Sleep 500ms
Enter
Sleep 2s
Type "cat /tmp/stem-mqtt-tls-sub.log"
Sleep 500ms
Enter
Sleep 3s

Hide
Type "kill %1 %2 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
