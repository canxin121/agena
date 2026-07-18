# Agena Crossterm patch

This directory vendors Crossterm 0.29.0 (MIT) with one input-boundary
extension.

Crossterm's Unix parser does not classify OSC replies. A reply such as
`ESC ] 4 ; -2 ; rgb:... ST` is consequently emitted as an Alt+] key followed
by ordinary character keys. That is unsafe for applications which issue
response-bearing terminal queries because a delayed reply can become user
text after the query caller times out.

The Agena patch adds typed `Event::TerminalResponse` values for the two
background-color protocols Agena emits, Kitty's graphics capability reply,
and the DSR/CPR completion markers. The classification happens while the
original byte frame is still intact, before keyboard decoding. Application
code can therefore correlate or discard late responses without reconstructing
protocol payloads from key-event timing.

One redundant-parentheses warning in the upstream Unix tty helper is also
removed so the vendored crate builds warning-free on Agena's Rust toolchain.
The rest of the source is the unmodified 0.29.0 crate published on crates.io.
