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
# `set +m` disables job-control notifications entirely, so backgrounding
# the broker never prints "[1] <pid>" or "[1]+ Exit/Done ..." into the
# recording. stdout/stderr also redirected to /dev/null: the broker's own
# tracing logs ("client connected" / "client disconnected") would
# otherwise interleave with the pub_sub example's output below, since both
# share this terminal.
Hide
Type "set +m"
Enter
Type "cargo run -q -p stem-mqtt-broker --bin mqtt-broker -- --port 1883 > /dev/null 2>&1 &"
Enter
Sleep 3s
Type "clear"
Enter
Show

# ── Run the pub/sub example against it ───────────────────────────────────────
Type "cargo run -q -p stem-mqtt-client --example pub_sub"
Sleep 500ms
Enter
Sleep 4s

# ── Stop the broker; hidden, and job-control is already off so no
# completion notice can appear even if a Show followed (it doesn't: the
# tape ends here, so the last captured frame is the pub_sub example's
# completed output above).
Hide
Type "kill %1 2>/dev/null; wait 2>/dev/null"
Enter
Sleep 500ms
