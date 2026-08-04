# VHS tape — stem-mqtt · MQTT Versions Demo
# Showcases: MQTT 3.1.1 and MQTT 5.0 clients connected to the same broker
# at the same time, each publishing and both receiving both messages —
# proof the protocol version is negotiated per-connection, not per-broker.
#
# Run: vhs examples/vhs/mqtt-versions-demo.tape
# (or: just vhs-tape mqtt-versions-demo)

Output examples/vhs/generated/mqtt-versions-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 700
Set FontSize 16
Set PlaybackSpeed 1.5

# ── Start the broker in the background (hidden from recording) ──────────────
# See pub-sub-demo.tape for why: `set +m` + /dev/null redirect keep the
# broker's own job-control/tracing noise out of the recording.
Hide
Type "set +m"
Enter
Type "cargo run -q -p mqtt-broker --bin mqtt-broker -- --port 1883 > /dev/null 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Run the mqtt_versions example against it ─────────────────────────────────
Type "cargo run -q -p mqtt-client --example mqtt_versions"
Sleep 500ms
Enter
Sleep 4s

# ── Stop the broker (hidden; job-control already off, no completion notice) ─
Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
