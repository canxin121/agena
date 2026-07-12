# TUI terminal compatibility

Agena uses capability profiles rather than treating `$TERM` as a complete
description of the terminal. Detection combines terminal identity evidence,
version, platform and detected environment layers (SSH, Mosh, tmux, screen,
Zellij and WSL). Environment variables do not prove layer order or nesting
depth, so Agena never presents them as a verified transport topology.

## Recognized terminal families

| Family | Identity evidence | Agena behavior |
|---|---|---|
| iTerm2 | `TERM_PROGRAM`, `LC_TERMINAL`, `ITERM_SESSION_ID` | Native `it2ul`/`it2dl` attachment transfer when the shell-integration utilities are installed |
| Kitty | `KITTY_WINDOW_ID`, `KITTY_PID`, `xterm-kitty` | Kitty keyboard mode, OSC 52, remote image clipboard reads, graphics capability evidence, and `kitten transfer` uploads/downloads |
| WezTerm | `TERM_PROGRAM`, `WEZTERM_PANE` | OSC 52; Kitty keyboard mode remains off unless explicitly enabled because WezTerm makes it configurable |
| Ghostty | `TERM_PROGRAM`, `GHOSTTY_RESOURCES_DIR`, `xterm-ghostty` | Kitty keyboard mode, OSC 52 read/write evidence, graphics capability evidence and terminfo diagnostics |
| Windows Terminal | `WT_SESSION` | ConPTY-safe VT defaults, native clipboard, OSC 52 profile and WSL topology detection |
| VS Code | `TERM_PROGRAM=vscode` | xterm.js-oriented profile, theme environment support and Remote SSH diagnostics |
| Apple Terminal | `TERM_PROGRAM=Apple_Terminal` | Conservative legacy keyboard behavior and native clipboard |
| Alacritty | `TERM_PROGRAM`, `ALACRITTY_SOCKET` | Standard VT, bracketed paste, focus and OSC 52 profile |
| VTE family | `VTE_VERSION` | GNOME Terminal/Ptyxis/Tilix-compatible VT and OSC 52 profile |
| Konsole | `KONSOLE_VERSION`, `KONSOLE_DBUS_SERVICE` | KDE terminal VT and OSC 52 profile |
| foot | `TERM=foot` or `foot-extra` | Kitty keyboard mode, synchronized-output evidence and OSC 52 |
| Warp | `TERM_PROGRAM=WarpTerminal` | Standard VT and OSC 52 profile |
| JetBrains | `TERMINAL_EMULATOR` | Conservative embedded-terminal profile |
| Rio / Contour | `TERM_PROGRAM` | Standard VT and OSC 52 profiles |
| xterm-compatible / Linux console / dumb | `$TERM` | `xterm-*` is compatibility evidence rather than product identity; unsupported enhanced modes are not enabled |

The terminal identity may be visible through a multiplexer while a protocol is
not. Agena therefore marks keyboard enhancement, rich clipboard, graphics and
terminal file transfer as `policy-dependent` behind tmux, screen, Zellij or
Mosh unless an explicit override provides stronger evidence. Capability state
distinguishes confirmed platform support, a user-forced override, a terminal
profile, transport or permission policy, unknown support, and whether Agena
has a working provider.

## Markdown math rendering

Transcript Markdown supports inline `$...$` and `\(...\)`, display `$$...$$`
and `\[...\]`, plus fenced `math`, `tex`, `latex`, and `katex` blocks. Escaped
dollar signs and inline-code spans are not interpreted as formulas.

On terminals that negotiate Kitty graphics, Sixel, or the iTerm2 inline-image
protocol, Agena typesets formulas with embedded KaTeX fonts through the pure
Rust RaTeX pipeline and places the resulting transparent image in the
transcript's scrollable line layout. Wide display formulas are scaled to the
viewport, and inline formulas share a bottom-aligned line box with surrounding
text. Image protocols are regenerated after terminal suspend/resume.

When no native graphics protocol is available, Agena renders formulas as 2-D
Unicode cell layouts, retaining stacked fractions, roots, scripts, and matrix
structure where supported. Unsupported input remains visible as source text.
Formula length, output dimensions, decoded pixels, artifact count, and encoded
protocol count are bounded so model-produced Markdown cannot grow the render
caches without limit.

## Kitty attachment transfer

When Agena runs directly in Kitty and an executable standalone `kitten` helper
passes version and subcommand probes, local files can be transferred into a
remote Agena process:

```text
/attach ~/Pictures/example.png
/attach "~/Documents/file with spaces.pdf" ~/Downloads/data.csv
/image ~/Pictures/example.png
```

The paths are interpreted on the computer running Kitty, not on the host
running Agena. Kitty presents its own permission and path confirmation UI. The
received tree is isolated in an exclusively created `0700` temporary directory.
Symbolic links and special files are rejected. Transfer is cancelled while it
is running if it exceeds 32 files, 64 directories, 16 levels of nesting,
50 MiB per file, or 200 MiB in total. Clipboard images also have dimension and
decoded-pixel safety limits.

`/download <workspace-path>` uses `kitten transfer` in Kitty and `it2dl` in
iTerm2. Kitty downloads default to `Downloads/` on the local computer. Set
`AGENA_TUI_DOWNLOAD_DIR` to change the local destination.

Kitty transfer through a multiplexer is disabled by default because support
depends on the multiplexer version and passthrough configuration. It can be
enabled after verification with `AGENA_TUI_KITTY_FILE_TRANSFER=1`.

When native clipboard access is unavailable, the normal image-attach action
uses `kitten clipboard --get-clipboard` to request a raster image from the
computer running Kitty. Kitty owns the permission prompt and the complete
bidirectional clipboard transaction. The TUI is suspended until the helper has
finished and drained its responses, then the resulting PNG is validated before
it is staged.

## Overrides

Overrides are intended for diagnostics and environments whose evidence is
hidden by containers, SSH configuration or multiplexers:

| Variable | Values | Effect |
|---|---|---|
| `AGENA_TUI_TERMINAL` | `kitty`, `wezterm`, `ghostty`, `windows-terminal`, `vscode`, and the other family names above | Override terminal identity |
| `AGENA_TUI_TERMINAL_VERSION` | version string | Attach a version to an identity override |
| `AGENA_TUI_KEYBOARD_PROTOCOL` | `kitty`, `legacy`, `auto` | Force Kitty/CSI-u enhancement on, force legacy input, or use profile evidence |
| `AGENA_TUI_OSC52` | boolean | Enable or disable OSC 52 text-copy requests |
| `AGENA_TUI_NATIVE_CLIPBOARD` | boolean | Enable or disable the operating-system clipboard provider |
| `AGENA_TUI_KITTY_FILE_TRANSFER` | boolean | Enable or disable Kitty TTY file transfer |
| `AGENA_TUI_QUERY_BACKGROUND` | boolean | Reserved; currently reported and ignored because active stdin probes are disabled |
| `AGENA_TUI_DOWNLOAD_DIR` | local path | Kitty download destination, interpreted on the local computer |
| `AGENA_TUI_KITTEN` | executable path | Explicit standalone `kitten` helper path |
| `AGENA_TUI_HELPER_TIMEOUT_SECS` | `15..3600` | Timeout for interactive terminal helpers; defaults to 300 seconds |

Do not force a protocol merely because the outer terminal supports it. Every
hop between Agena and that terminal must preserve the protocol and its replies.

## Diagnostics

`/diagnostics` opens a scrollable report with identity evidence, confidence,
conflicts, environment-layer evidence, protocol state, provider readiness,
helper versions and compatibility warnings. Press `c` or `y` to copy the
sanitized report. Invalid overrides are reported instead of being silently
ignored. Detailed warnings are also written to the configured TUI log.

## Compatibility test matrix

Changes to terminal I/O should cover at least these cases:

1. Direct local terminal with legacy keyboard input.
2. Direct Kitty/Ghostty/foot with enhanced keyboard input.
3. WezTerm with Kitty keyboard mode both disabled and enabled.
4. SSH with and without propagated terminal identity.
5. tmux, screen and Zellij with passthrough disabled.
6. Windows Terminal through native PowerShell, WSL and SSH.
7. VS Code desktop, Remote SSH and Dev Containers.
8. Fragmented, incomplete and delayed OSC/CSI/DCS responses.
9. Suspend/resume around editor, pager and file-transfer helpers.
10. Double Ctrl+C and panic restoration with no protocol tail left in the shell.
