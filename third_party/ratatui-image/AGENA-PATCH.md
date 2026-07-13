# Agena patch for ratatui-image 11.0.6

This is the source published as `ratatui-image` 11.0.6, with two narrow
terminal-ownership fixes:

1. The stdio capability query uses an absolute deadline and a cancellable
   descriptor/console wait on the calling thread. Upstream's detached blocking
   reader survives a timeout and can consume the application's next key event.
2. Picker construction no longer runs `tmux set -p allow-passthrough on` as an
   environment-detection side effect. Agena owns transport policy and never
   changes a user's multiplexer configuration implicitly.

`Picker::from_parts` is added so Agena can pass centrally negotiated properties
without triggering a second probe. The package manifest omits upstream bins,
examples and benches because this vendored copy is a library dependency. All
other source remains upstream code.
