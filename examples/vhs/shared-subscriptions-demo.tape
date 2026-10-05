# VHS tape — stem-mqtt · Shared Subscriptions Demo
# Showcases: three workers joining the group `workers` via $share/workers/jobs/#, nine jobs
# published, every job delivered to exactly one worker round-robin (3 each) — while an
# ordinary subscriber on the same topics still sees all nine.
#
# Run: vhs examples/vhs/shared-subscriptions-demo.tape
# (or: just vhs-tape shared-subscriptions-demo)

Output examples/vhs/generated/shared-subscriptions-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 640
Set FontSize 15
Set PlaybackSpeed 1.5

# ── Start the broker in the background (hidden from recording) ──────────────
Hide
Type "set +m"
Enter
Type "cargo run -q -p stem-mqtt-broker --bin mqtt-broker -- --port 1883 > /dev/null 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Run the example against it ───────────────────────────────────────────────
Type "cargo run -q -p stem-mqtt-client --example shared_subscriptions"
Sleep 500ms
Enter
Sleep 6s

Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
