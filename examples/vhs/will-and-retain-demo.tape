# VHS tape — stem-mqtt · Retained Messages & Last Will Demo
# Showcases: a retained message delivered to a subscriber that arrives *after* it was
# published, then a Last Will and Testament — the broker publishes a vanished client's
# will to a watcher.
#
# Run: vhs examples/vhs/will-and-retain-demo.tape
# (or: just vhs-tape will-and-retain-demo)

Output examples/vhs/generated/will-and-retain-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 460
Set FontSize 16
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
Type "cargo run -q -p stem-mqtt-client --example will_and_retain"
Sleep 500ms
Enter
Sleep 5s

Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
