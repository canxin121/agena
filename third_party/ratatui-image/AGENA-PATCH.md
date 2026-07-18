# Agena patch for ratatui-image 11.0.6

This is the source published as `ratatui-image` 11.0.6, with seven narrow
Agena-specific fixes:

1. The startup stdio capability query uses an absolute liveness deadline and a
   cancellable descriptor/console wait on the calling thread. Upstream's
   detached blocking reader survives a timeout and can consume the
   application's next key event. Background detection is a separate bounded
   startup query, supports both OSC 11 and iTerm2's documented OSC 4;-2 form,
   and correctly normalizes the two- or four-digit RGB components iTerm2 may
   return instead of turning two-digit white into black. Every query appends a
   DSR after its request on the ordered output stream and treats that response
   as the protocol boundary; no post-response settle timer is used. Agena adds
   a final typed CPR barrier before handing the byte stream to normal input, so
   a liveness timeout never doubles as proof that a reply cannot still arrive.
2. Picker construction no longer runs `tmux set -p allow-passthrough on` as an
   environment-detection side effect. Agena owns transport policy and never
   changes a user's multiplexer configuration implicitly.
3. iTerm2 images use terminal-cell dimensions and explicit aspect-ratio
   preservation. Every displayed image is transmitted as one contiguous PNG,
   including a lazily cached crop at viewport edges, so Retina scaling or
   custom line spacing can never expose gaps between terminal-row images. PNG
   alpha is retained end to end so Agena's formula glyphs can use a transparent
   canvas over terminal colors, transparency, or background images.
4. Kitty virtual placements declare their target cell rectangle, so scaled
   formulas and images match the Unicode placeholder grid instead of exposing
   only a natural-size subset. Agena requests proportional scaling for cached
   placements; Kitty's virtual-placement rules preserve the raster ratio.
5. Sixel passthrough doubles every nested escape for tmux, viewport slicing
   removes bands hidden at both the top and bottom instead of overdrawing the
   rest of the TUI, and the encoder explicitly requests square pixels.
6. Primitive half-block rendering letterboxes into its two-subpixel cell grid
   rather than stretching the input independently on both axes.
7. Fixed and sliced widgets now agree with each protocol's actual clipping
   support and use overflow-safe terminal geometry at viewport boundaries.

`Picker::from_parts` is added so Agena can pass centrally negotiated properties
without triggering a second probe. The package manifest omits upstream bins,
examples, benches and image assets because this vendored copy is a library
dependency; retained tests therefore use generated fixtures. All other source
remains upstream code.
