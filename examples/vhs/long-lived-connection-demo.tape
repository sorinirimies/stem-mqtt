# VHS tape — stem-mqtt · Long-Lived Connection Demo
# Showcases: a single connection held open for ~20 seconds — well past
# multiple keep-alive intervals (3s each here) — publishing a heartbeat
# every 2s and checking is_connected() throughout, instead of the
# connect-publish-disconnect shape the other demos use. Sped up via
# PlaybackSpeed so the ~20s real-time run stays a snappy preview.
#
# Run: vhs examples/vhs/long-lived-connection-demo.tape
# (or: just vhs-tape long-lived-connection-demo)

Output examples/vhs/generated/long-lived-connection-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 800
Set FontSize 15
Set PlaybackSpeed 2.5

# ── Start the broker in the background (hidden from recording) ──────────────
Hide
Type "set +m"
Enter
Type "cargo run -q -p mqtt-broker --bin mqtt-broker -- --port 1883 > /dev/null 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Run the long_lived_connection example against it (~20s real time) ───────
Type "cargo run -q -p mqtt-client --example long_lived_connection"
Sleep 500ms
Enter
Sleep 23s

# ── Stop the broker (hidden; job-control already off, no completion notice) ─
Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
