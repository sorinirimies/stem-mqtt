# VHS tape — stem-mqtt · Authentication Demo
# Showcases: the `auth_broker` example (a pluggable MqttAuthProvider + an event listener).
# Two clients with wrong logins are rejected, `demo`/`demo` is accepted and publishes, and the
# broker's own log then shows each authentication decision and connection event.
#
# Run: vhs examples/vhs/auth-demo.tape
# (or: just vhs-tape auth-demo)

Output examples/vhs/generated/auth-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1000
Set Height 520
Set FontSize 15
Set PlaybackSpeed 1.5

# ── Start the authenticating broker in the background (hidden) ──────────────
# Its output goes to a log file, shown after the client run: the broker's tracing logs
# would otherwise interleave with the client's output in this one terminal.
Hide
Type "set +m"
Enter
Type "demo() { cargo run -q -p stem-mqtt-client --example $1; }"
Enter
Type "brokerlog() { echo; echo '── broker log ──'; grep '^\[' /tmp/stem-mqtt-auth.log; }"
Enter
Type "cargo build -q -p stem-mqtt-client --examples && cargo build -q -p stem-mqtt-broker --bins --examples && echo RE''ADY"
Enter
Wait+Screen@300s /READY/
Type "cargo run -q -p stem-mqtt-broker --example auth_broker > /tmp/stem-mqtt-auth.log 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Two login attempts ───────────────────────────────────────────────────────
Type "demo auth_client"
Sleep 500ms
Enter
Sleep 4s

# ── What the broker saw (only its own [auth]/[event] lines, not tracing noise) ──
Type "brokerlog"
Sleep 500ms
Enter
Sleep 3s

Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
