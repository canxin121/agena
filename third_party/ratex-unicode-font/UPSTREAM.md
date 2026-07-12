# `ratex-unicode-font` source patch

This directory contains the source of `ratex-unicode-font` 0.1.13, published
under the MIT license by the RaTeX contributors:

- Upstream repository: <https://github.com/erweixin/RaTeX>
- crates.io package checksum:
  `8144de0e20108ed19827203b38df77037779a5a68aa293fba858d25b240e170d`

Agena carries one behavioral patch: font-discovery messages are silent by
default instead of being written unconditionally with `eprintln!`. A direct
stderr write while the TUI owns the terminal corrupts the alternate-screen
layout and can scroll image placements over the composer.

Set `RATEX_UNICODE_FONT_DIAGNOSTICS=1` to restore the upstream diagnostic
messages when troubleshooting outside the TUI.
