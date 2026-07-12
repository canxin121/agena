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
4. The runtime permits one bounded graphics/cell-size/background negotiation after
   alternate-screen entry and before the sole `EventStream` is created. No
   screen or application code may issue another response-bearing query.
   OSC 11 background evidence is authoritative when returned; environment
   color hints remain the fallback.
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

## Module boundaries

```text
apps/agena-cli/src/terminal/
  mod.rs            runtime ownership and public operations
  broker.rs         complete-frame protocol serialization
  lifecycle.rs      reversible tty state transitions
  capabilities.rs   capability decisions and provider readiness
  identity.rs       multi-source terminal identity evidence
  transport.rs      SSH/Mosh/multiplexer/WSL environment evidence
  overrides.rs      strict override parsing and diagnostics
  profiles.rs       declarative terminal-family capability profiles
  version.rs        normalized dotted terminal versions
  protocol.rs       non-interactive environment color evidence
  input.rs          normalized terminal input and legacy paste fallback

apps/agena-cli/src/clipboard/
  text.rs          native, tmux, and OSC 52 providers
  image.rs         native and WSL image acquisition
  path.rs          pasted path normalization

apps/agena-cli/src/attachment_source.rs
  native/Kitty clipboard-image and terminal file-transfer acquisition providers

apps/agena-cli/src/helper_runner.rs
  executable checks, bounded probes, timeouts, cancellation and child reaping

apps/agena-cli/src/provider_error.rs
  typed provider failures and fallback policy

apps/agena-cli/src/math_render.rs
  RaTeX image typesetting, bounded artifact/protocol caches, and Unicode fallback
```

`agena-tui-components::Editor` deliberately contains no terminal protocol or
paste timing state. It edits text supplied by the application.

## Adding an integration

New terminal functionality should be introduced in this order:

1. Add a narrow capability with confirmed, user-forced, profiled,
   policy-dependent, unsupported or unknown evidence and separate integration
   readiness.
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
