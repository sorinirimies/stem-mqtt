# VHS tape — stem-mqtt · Authentication Demo
# Showcases: the `auth_broker` example (a pluggable MqttAuthProvider + an event listener).
# A client with the wrong login is rejected, `demo`/`demo` is accepted and publishes, and the
# broker's own log then shows each authentication decision and connection event.
#
# Run: vhs examples/vhs/auth-demo.tape
# (or: just vhs-tape auth-demo)

Output examples/vhs/generated/auth-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 500
Set FontSize 16
Set PlaybackSpeed 1.5

# ── Start the authenticating broker in the background (hidden) ──────────────
# Its output goes to a log file, shown after the client run: the broker's tracing logs
# would otherwise interleave with the client's output in this one terminal.
Hide
Type "set +m"
Enter
Type "cargo run -q -p stem-mqtt-broker --example auth_broker > /tmp/stem-mqtt-auth.log 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Two login attempts ───────────────────────────────────────────────────────
Type "cargo run -q -p stem-mqtt-client --example auth_client"
Sleep 500ms
Enter
Sleep 4s

# ── What the broker saw (only its own [auth]/[event] lines, not tracing noise) ──
Type "echo; echo '── broker log ──'; grep '^\[' /tmp/stem-mqtt-auth.log"
Sleep 500ms
Enter
Sleep 3s

Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
