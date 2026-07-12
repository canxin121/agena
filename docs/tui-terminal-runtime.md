# TUI terminal runtime

The Agena TUI treats a terminal as a stateful, bidirectional protocol, not as
independent keyboard input and screen output streams. `TerminalRuntime` is the
single owner of terminal events, protocol output, mode changes, suspension,
and restoration.

## Invariants

1. Application and widget code must not create a `crossterm::EventStream`.
2. Application and integration code must not write OSC, CSI, or DCS directly
   to stdout. Complete protocol frames go through `TerminalRuntime`.
3. Only the runtime may enable or disable raw mode, alternate screen,
   bracketed paste, focus reporting, or keyboard enhancement flags.
4. Terminal queries run while the runtime has exclusive input ownership.
   Active background-color probing is disabled by default; it may be enabled
   with `AGENA_TUI_QUERY_BACKGROUND=1` for diagnostics.
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

## Module boundaries

```text
apps/agena-cli/src/terminal/
  mod.rs            runtime ownership and public operations
  lifecycle.rs      reversible tty state transitions
  capabilities.rs   topology and evidence-backed capabilities
  protocol.rs       bounded terminal query framing
  input.rs          normalized terminal input and legacy paste fallback

apps/agena-cli/src/clipboard/
  text.rs          native, tmux, and OSC 52 providers
  image.rs         native and WSL image acquisition
  path.rs          pasted path normalization

apps/agena-cli/src/attachment_source.rs
  clipboard-image and terminal file-transfer acquisition providers
```

`agena-tui-components::Editor` deliberately contains no terminal protocol or
paste timing state. It edits text supplied by the application.

## Adding an integration

New terminal functionality should be introduced in this order:

1. Add a narrow capability with `Supported`, `Unsupported`, or `Unknown`
   evidence.
2. Add a provider or typed protocol operation behind the runtime boundary.
3. Keep terminal brand and transport detection inside capabilities/providers.
4. Return a semantic result such as confirmed, best effort, denied, or
   unsupported.
5. Add fragmentation, timeout, lifecycle, and fallback tests before exposing
   the operation to a page.

Do not add `TERM_PROGRAM` branches to screens, editors, composer state, or
backend domain code.
