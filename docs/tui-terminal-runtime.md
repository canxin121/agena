# TUI terminal runtime

The Agena TUI treats a terminal as a stateful, bidirectional protocol, not as
independent keyboard input and screen output streams. `TerminalRuntime` is the
single owner of terminal events, protocol output, mode changes, suspension,
and restoration.

“Single owner” does not mean that only one terminal-related program exists.
For example, with iTerm2 → SSH → tmux → Agena, Agena opens the tmux pane's PTY
on the Ubuntu host; tmux transports display commands through SSH to iTerm2,
which is the endpoint emulator that finally interprets them. Agena owns one
local PTY input/output pair while retaining separate evidence about the
endpoint and the intervening transport layers.

## Invariants

1. Application and widget code must not create a terminal input reader.
2. Application and integration code must not write OSC, CSI, or DCS directly
   to stdout. Complete protocol frames go through `TerminalRuntime`.
3. Only the runtime may enable or disable raw mode, alternate screen,
   bracketed paste, focus reporting, mouse capture, or keyboard enhancement
   flags. Mouse capture is active only on the unobscured main chat surface;
   routes and modal surfaces release it so the terminal can provide native
   text selection and scrollback behavior.
4. The runtime permits one bounded graphics/cell-size/background negotiation
   after alternate-screen entry and before runtime input starts.
   The query waits synchronously with an absolute deadline; it never leaves a
   detached stdin reader that can steal a later key. No screen or application
   code may issue another response-bearing query. OSC 11 background evidence
   is authoritative when returned; environment color hints remain the
   fallback.
5. External editors, pagers, and transfer utilities run through
   `TerminalRuntime::with_suspended`, which restores the terminal after
   success, error, or panic.
6. Keyboard event-type reporting stays disabled until an application feature
   genuinely needs release events.
7. Terminal timing heuristics belong to the input normalization boundary, not
   reusable editors or widgets.
8. Clipboard operations choose a provider from terminal capabilities. OSC 52
   is reported as best effort, not as confirmed clipboard success.
9. Attachment sources acquire files; attachment staging validates and freezes
   message content. A staged in-memory attachment is not reread from a mutable
   path at submission time.
10. Application-owned OSC/CSI/DCS writes pass through the protocol broker as
    complete validated frames.
11. Suspending the TUI preserves normalized user input; shutdown never drains
    arbitrary terminal events by elapsed time.
12. TUI startup requires stdin and stdout to be interactive handles for the
    same terminal device and rejects `TERM=dumb` before changing terminal
    state. A process-wide ownership guard prevents a second `TerminalRuntime`.
13. Startup, suspend/resume and shutdown are transactions. Every completed
    mode transition is tracked and unwound on ordinary errors, construction
    failures, panics and `Drop`; a suspended-operation panic is never replaced
    by a secondary clear/resume error.

## Endpoint, path and provider decisions

Agena does not reduce every feature to the smallest capability shared by every
named layer. That would unnecessarily disable protocols that SSH transports
transparently. Each feature decision keeps three concerns separate:

1. endpoint evidence: confirmed, user-forced, profiled, unsupported or unknown;
2. path state: clear, explicitly forced, unverified or blocked;
3. provider readiness: no helper needed, ready, or missing.

A feature is operational only when all three permit it. Environment variables
can establish that SSH, Mosh, tmux, screen, Zellij or WSL are present, but they
cannot generally prove their nesting order or the number of repeated layers.
Diagnostics therefore report an unordered evidence set instead of inventing a
chain.

Graphics uses a more specific path policy. Plain SSH remains eligible because
it is byte-transparent and the endpoint query is authoritative. A tmux pane is
eligible only when a read-only `show-options` probe observes
`allow-passthrough` as `on` or `all`; Agena never changes the user's tmux
configuration. The query and later image protocol are then tmux-wrapped, even
when `TERM=screen-*`. Agena also reads tmux's client terminal name; a detected
tmux-inside-tmux/screen path remains unverified because the outer pane's option
cannot be inspected safely from the inner server. Mosh, screen, Zellij, nested
multiplexers and other unverifiable tmux paths use the deterministic
two-dimensional Unicode renderer in automatic mode. An expert can force a
known path with `ui.tui.graphics=native` (or `AGENA_TUI_GRAPHICS=native`), or
disable all probing with `ui.tui.graphics=unicode`.

## Module boundaries

```text
apps/agena/src/terminal/
  mod.rs            runtime ownership and public operations
  broker.rs         complete-frame protocol serialization
  lifecycle.rs      reversible tty state transitions
  capabilities.rs   capability decisions and provider readiness
  graphics.rs       native-image transport policy and read-only tmux probing
  identity.rs       multi-source terminal identity evidence
  transport.rs      SSH/Mosh/multiplexer/WSL environment evidence
  overrides.rs      strict override parsing and diagnostics
  profiles.rs       declarative terminal-family capability profiles
  version.rs        normalized dotted terminal versions
  protocol.rs       non-interactive environment color evidence
  input.rs          sole cancellable readiness reader, event normalization,
                    and legacy paste fallback

apps/agena/src/clipboard/
  text.rs          native, tmux, and OSC 52 providers
  image.rs         native and WSL image acquisition
  path.rs          pasted path normalization

apps/agena/src/attachment_source.rs
  native/Kitty clipboard-image and terminal file-transfer acquisition providers

apps/agena/src/helper_runner.rs
  executable checks, bounded probes, timeouts, cancellation and child reaping

apps/agena/src/provider_error.rs
  typed provider failures and fallback policy

apps/agena/src/math_render.rs
  scoped per-App render configuration, RaTeX typesetting, bounded caches, and
  Unicode fallback

third_party/ratatui-image/
  minimal 11.0.6 source patch for cancellable stdio queries, side-effect-free
  tmux detection, construction from centrally negotiated properties, exact
  iTerm2/Kitty cell geometry, and tmux-safe/cropped Sixel output
```

`agena-tui-components::Editor` deliberately contains no terminal protocol or
paste timing state. It edits text supplied by the application.

## Adding an integration

New terminal functionality should be introduced in this order:

1. Add narrow endpoint evidence, transport-path state and provider readiness;
   do not encode all three as a terminal brand check.
2. Add a provider or typed protocol operation behind the runtime boundary.
3. Keep terminal brand and transport detection inside capabilities/providers.
4. Return a typed semantic result such as cancelled, denied, unsupported,
   dependency missing, timed out, protocol error or I/O error.
5. Add fragmentation, timeout, lifecycle, and fallback tests before exposing
   the operation to a page.

Do not add `TERM_PROGRAM` branches to screens, editors, composer state, or
backend domain code.

The recognized profiles, overrides and compatibility matrix are documented in
[`tui-terminal-compatibility.md`](tui-terminal-compatibility.md).
