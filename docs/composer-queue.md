# Composer Queue, Cancel & Steer

This document describes how Agena handles user input that arrives while
the AI is mid-turn. The design is a deliberate hybrid of Claude Code's
local pending-message queue and OpenAI Codex's in-flight steer
mechanism: both are exposed, and the user picks which one fires by the
key they press.

## Goals

1. **No dropped input** — typing during a turn is never a no-op.
2. **Two distinct intents** — "queue this" vs. "act on this now".
3. **Real cancel** — Esc terminates the running turn at the core, not
   only on the client.
4. **Configurable keys** — every binding lives in TOML; the defaults
   are picked, not hardcoded.

## Surface

| Action | Default key | When idle | When AI is working |
|---|---|---|---|
| Queue (secondary) | `Enter` | Submit immediately | Append to pending queue, drain after turn |
| Submit-now (primary) | `Ctrl+Enter` | Submit immediately | **Steer** the active turn — model sees the message on its next step |
| Newline | `Shift+Enter`, `Alt+Enter`, `Ctrl+J` | Insert literal newline | Same |
| Edit queue | `Up` (cursor on first line) | Pull queued messages back into editor | Same |
| Cancel turn | `Esc` (composer or transcript) | n/a | Cancel the in-flight turn at the core |
| Clear input | `Esc Esc` (within 600 ms) | Wipe editor | Wipe editor |
| Side-question | `/btw <question>` | Spawn a child session, ask there, parent untouched | Same — main turn keeps running |
| Queue control | `/queue [list\|clear\|pop]` | Inspect / drop / pop into editor | Same |

The defaults follow the user's stated preference: `Enter` queues,
`Ctrl+Enter` sends now. To swap them (Codex-style: Enter = send-now,
Tab = queue), drop a `~/.agena/tui.toml`:

```toml
[keybindings.composer]
submit     = ["enter"]
queue      = ["tab"]
newline    = ["shift+enter", "ctrl+j"]
edit_queue = ["up"]

# Optional — double-tap window for "Esc Esc clears input".
double_esc_window_ms = 500
```

Every `parse_chord` token is case-insensitive and supports the
modifiers `ctrl`, `alt` (alias `meta`/`option`), `shift`, plus named
keys (`enter`, `up`, `f3`, `pageup`, …) and single character keys.

## Architecture

```
┌──────────────────────────── agena-tui ────────────────────────────┐
│                                                                   │
│   handle_composer_key  ── ComposerKeyBindings::match_action       │
│           │                                                       │
│           ├─ Submit      → submit_or_steer ──► Backend::steer_input
│           ├─ Queue       → queue_or_submit ──► ComposerQueue.enqueue
│           ├─ Newline     → editor newline                          │
│           └─ EditQueue   → ComposerQueue.pop_all_editable          │
│                                                                   │
│   On TurnSubmitted(Ok)                                            │
│       └─ try_drain_queue_one ──► restore + submit                  │
│                                                                   │
│   ComposerQueue ─ FIFO with three bands: Now / Next / Later       │
│                                                                   │
└────────────────── REST/IPC ──┬──────────────────────────────────────┘
                                ▼
┌─────────────────────── agena-api-server ──────────────────────────┐
│   Op::CancelTurn  ──► SessionManager::cancel_active_turn          │
│   Op::SubmitTurn  ──► SessionManager::submit_user_turn            │
│   (steer_input    ──► SessionManager::steer_input — direct call)  │
└────────────────────────┬───────────────────────────────────────────┘
                          ▼
┌────────────────────── agena (core) ───────────────────────────────┐
│                                                                   │
│   SessionManager                                                   │
│     turn_registry : TurnRegistry  (Mutex<HashMap<i64, Arc<…>>>)   │
│                                                                   │
│   submit_user_turn(req)                                            │
│     │                                                              │
│     ├── (control, steer_rx) = registry.register(session_id)        │
│     │                                                              │
│     └── run_until_stable                                           │
│           ┌── loop                                                 │
│           │      if control.cancel.is_cancelled() → bail           │
│           │      drain_steer_input(steer_rx) → append User msgs    │
│           │      run_model_turn                                    │
│           │         tokio::select! {                               │
│           │             res = processor.run_turn(req+cancel) =>…   │
│           │             _   = control.cancel.cancelled() =>…       │
│           │         }                                              │
│           │      append turn boundary events                       │
│           │      continue / return                                 │
│           └──                                                      │
│                                                                   │
│   processor.run_turn (stream loop)                                 │
│     ┌── loop                                                       │
│     │      tokio::select! {                                        │
│     │          biased;                                             │
│     │          _   = cancel.cancelled() => break                   │
│     │          ev  = stream.next()      => handle event            │
│     │      }                                                       │
│     │      if cancel triggered → terminal_error = "cancelled"      │
│     └──                                                            │
└───────────────────────────────────────────────────────────────────┘
```

## TurnRegistry

A `Mutex<HashMap<session_id, Arc<TurnControl>>>`. Each `TurnControl`
owns:

* `cancel: CancellationToken` — fired by `cancel_active_turn`.
* `steer_tx: mpsc::UnboundedSender<Vec<PartContent>>` — populated by
  `steer_input`, drained between turn iterations inside
  `drain_steer_input`.

`register` will cancel and replace any prior entry for the same
session, so a re-entered turn never inherits stale steer messages.
`unregister_if_matches` uses pointer equality to avoid clobbering a
newly registered control if the older task races.

## ComposerQueue

Three priority bands with strict ordering:

* `Now` — pushed to the front. Used for failed-steer fallback.
* `Next` — normal user submissions while busy. FIFO.
* `Later` — system notifications, never starves real input.

`pop_all_editable` is the inverse of `enqueue` — used by `Up` and
`/queue pop` to pull every editable entry back into the editor.

## Cancel semantics

Two layers cooperate:

1. **TUI** — Esc immediately clears `transcript.submitting` so the
   user regains control and the cancel RPC fires asynchronously.
2. **Core** — `cancel_active_turn` signals the `CancellationToken`.
   Both `run_until_stable` (between turns) and `processor.run_turn`
   (inside the stream loop, biased select) observe it. The
   provider-side stream future is dropped, which on reqwest-based
   providers also tears down the underlying HTTP connection.

If the user hits Esc but no turn is active, the call returns
`AppError::Internal("no in-flight turn …")` and the dispatcher
silently turns it into an `Ack` — Esc never errors visibly.

## Steer semantics

`SessionManager::steer_input(session_id, parts)` sends the parts down
the steer channel. They become a `User` message on the next iteration
of `run_until_stable`, before the next call to `run_model_turn`. The
model sees them as fresh user input — Codex's `push_pending_input`
behavior, but exposed as an explicit `Op` rather than an implicit
side-effect of "Enter while busy".

If steer fails (no in-flight turn / closed channel), the TUI catches
the error in `handle_steer_submitted` and pushes the draft onto the
queue with `QueuePriority::Now` so it sends out at the next turn
boundary. Nothing is dropped.

## /btw side-question

`/btw <question>` spawns a fresh child session (via
`Backend::create_session(parent_id=current)`) and submits the question
there from a detached `tokio::spawn`. The parent transcript and
`submitting` flag are untouched — the user can keep typing into the
parent while the child session runs. Switch panes to read the answer
when convenient.

This trades exact equivalence with Claude Code's `/btw` (which forks
agent state) for implementation simplicity: the shared parent context
is the workspace, not the conversation.

## Tests

* `apps/agena-tui/src/composer_queue.rs` — 4 unit tests (FIFO,
  priority ordering, pop_all_editable, preview truncation).
* `apps/agena-tui/src/keybindings.rs` — 3 unit tests (defaults
  distinguish submit/queue, parse_chord parser, override semantics).
* `crates/agena/src/session/control.rs` — 5 unit tests (cancel,
  no-active-turn error, steer push, re-register cancels prior,
  unregister respects ptr-eq).
* `crates/agena/src/session/manager.rs` —
  `cancel_active_turn_aborts_a_running_turn` (real provider stream
  with a 60s sleep, cancelled within ~80ms), plus negative-path
  `cancel_with_no_active_turn_is_a_clean_error` and
  `steer_with_no_active_turn_is_a_clean_error`.

## Future work

* Provider-side cancel hint: Anthropic / OpenAI Responses both
  understand explicit cancel; we currently rely on `Drop` of the
  reqwest stream. Adding an explicit RPC will let us tell the
  upstream we're done, freeing their compute too.
* `/queue` could grow a paginated overlay rather than a one-line
  flash for large queues.
* Multi-user / shared-session: the TurnRegistry is keyed on
  `session_id`, so per-user steers in a shared session would need
  identity propagation.
