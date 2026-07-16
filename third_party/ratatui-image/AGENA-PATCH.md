# Agena patch for ratatui-image 11.0.6

This is the source published as `ratatui-image` 11.0.6, with six narrow
Agena-specific fixes:

1. The stdio capability query uses an absolute deadline and a cancellable
   descriptor/console wait on the calling thread. Upstream's detached blocking
   reader survives a timeout and can consume the application's next key event.
2. Picker construction no longer runs `tmux set -p allow-passthrough on` as an
   environment-detection side effect. Agena owns transport policy and never
   changes a user's multiplexer configuration implicitly.
3. iTerm2 images use terminal-cell dimensions and cell-aligned row slices.
   Pixel-sized row images can render smaller than their cursor rows under
   Retina scaling or custom line spacing, leaving large gaps between slices.
4. Kitty virtual placements declare their target cell rectangle, so scaled
   formulas and images match the Unicode placeholder grid instead of exposing
   only a natural-size subset.
5. Sixel passthrough doubles every nested escape for tmux, and viewport slicing
   removes bands hidden at both the top and bottom instead of overdrawing the
   rest of the TUI.
6. Fixed and sliced widgets now agree with each protocol's actual clipping
   support and use overflow-safe terminal geometry at viewport boundaries.

`Picker::from_parts` is added so Agena can pass centrally negotiated properties
without triggering a second probe. The package manifest omits upstream bins,
examples, benches and image assets because this vendored copy is a library
dependency; retained tests therefore use generated fixtures. All other source
remains upstream code.
