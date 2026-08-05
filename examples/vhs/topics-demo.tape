# VHS tape — stem-mqtt · Topics & Wildcards Demo
# Showcases: three subscription filters of increasing scope — an exact
# topic, a `+` single-level wildcard, and a `#` multi-level wildcard — all
# watching the same topic tree, then four publishes of varying depth so
# you can see exactly which filter catches which message and why.
#
# Run: vhs examples/vhs/topics-demo.tape
# (or: just vhs-tape topics-demo)

Output examples/vhs/generated/topics-demo.gif

Set Theme "Catppuccin Mocha"
Set Width 1200
Set Height 800
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

# ── Run the topics example against it ────────────────────────────────────────
Type "cargo run -q -p stem-mqtt-client --example topics"
Sleep 500ms
Enter
Sleep 4s

# ── Stop the broker (hidden; job-control already off, no completion notice) ─
Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
