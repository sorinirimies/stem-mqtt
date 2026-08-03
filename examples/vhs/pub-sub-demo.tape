# VHS tape — stem-mqtt · Pub/Sub Demo
# Showcases: starting the mqtt-broker CLI, then running the pub_sub
# client example against it for a full publish/subscribe round trip.
#
# Run: vhs examples/vhs/pub-sub-demo.tape
# (or: just vhs-tape pub-sub-demo)

Output examples/vhs/generated/pub-sub-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 700
Set FontSize 16
Set PlaybackSpeed 1.5

# ── Start the broker in the background (hidden from recording) ──────────────
Hide
Type "cargo run -q -p mqtt-broker --bin mqtt-broker -- --port 1883 &"
Enter
Sleep 3s
Show

# ── Run the pub/sub example against it ───────────────────────────────────────
Type "cargo run -q -p mqtt-client --example pub_sub"
Sleep 500ms
Enter
Sleep 4s

# ── Stop the broker (hidden) ──────────────────────────────────────────────────
Hide
Type "kill %1 2>/dev/null"
Enter
Show
