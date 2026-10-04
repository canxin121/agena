# Side conversations and temporary questions

Web and TUI expose `/side` and `/btw` as separate commands and as controls at
the left of the composer status row. Both inherit the current conversation's
history and model selection. They submit work to a separate execution; the
parent's message queue, input draft and pending interactions are preserved.

| Command | Behavior |
| --- | --- |
| `/side [question]` | Create and open a persistent child conversation. An optional question starts it immediately. The child remains in session navigation, can perform newly requested work under its permissions, and has a return-to-parent control after reopening. `/aside` is an alias. |
| `/btw [question]` | Open a temporary Markdown question window. An optional question starts it immediately. Read-only tools can inspect context; file changes, shell execution, delegation and other mutation tools are unavailable. |

The model receives an explicit system instruction identifying the mode and
parent session, plus an AI-only boundary before the first new user message.
Inherited tasks, approvals, plans and pending tool calls are context rather
than instructions to continue. Answers are displayed separately and are not
automatically sent into the parent's prompt. A side shares the workspace,
so explicitly requested edits there still affect files the parent may use.

In the BTW window, Enter sends, Stop cancels the question while preserving
the partial answer, and Close/Escape closes it. TUI also supports Ctrl+C to
stop the question and arrows, Page Up/Down or the wheel to scroll the answer.
The main composer stays independent. Each new BTW send creates a fresh
question against the parent's history, rather than extending a durable chat.
Use a side for ongoing discussion.

## Execution and lifecycle

`POST /api/v1/sessions/{id}/btw` owns one temporary execution and returns SSE
`btw` events containing `{ text, done, error }` snapshots. Clients do not
automatically replay the POST. Closing/disconnecting aborts the request;
late snapshots cannot update a closed window or a different request.

The server admits at most four concurrent questions, accepts up to 16 KiB
of UTF-8 question text, and limits answer text to 256 KiB. Answer updates are
coalesced at 100 ms intervals through a bounded channel. Execution is limited
to ten minutes after submission. Requests that need additional approval or
interactive input stop with an explanation to continue in a side or main chat.

Temporary identity and permissions are stored atomically when forking, before
observers can see the child. Temporary conversations are excluded from normal
session lists, navigation counts and the shared UI live feed. The latter caches
classification per connection, avoiding a database lookup for every streamed
part, and handles clients joining after question creation.

The owner cancels and removes the temporary conversation on completion,
failure or disconnect. Cleanup waits for execution to unwind before deleting
storage. If shutdown interrupts cleanup, startup recovery removes the remaining
temporary conversation; persistent side conversations survive. The public fork
API accepts side mode but rejects attempts to create an unowned BTW session.
Temporary questions cannot themselves be forked or rewound. Further forks of a side
retain side mode and record their immediate parent, keeping the model boundary
and Web/TUI return controls consistent.

Read-only enforcement is applied to tool discovery and invocation, before
tool preparation hooks, and cannot be bypassed by granting a tool approval.
The path permission ceiling also denies writes and survives policy refresh.
This uses plugin read-only declarations and Agena's permission checks; it is
not an operating-system sandbox for untrusted plugin code.

## Regression coverage

HTTP tests use a local fake model to exercise inherited context, read tools,
rejected writes, a parent paused for user input, stream disconnect cleanup,
normal and midstream live subscriptions, hidden statistics, and persistent
side identity. Runtime tests cover policy refresh, permission persistence and
startup cleanup. TUI and Web tests cover cancellation ownership, duplicate
sends, stale updates, window closure and stream parsing, including split UTF-8.

Verification on 2026-10-04: the eight affected Rust library suites passed 993
tests, the CLI/server binary passed 101 tests, and Web passed 583 tests. The
final nested-branch correction passed the 211-test runtime and 41-test API
server suites again. The Web production build and strict Clippy
checks across the affected crates and their targets passed. Browser visual
verification was unavailable in the development environment.

The boundary semantics were compared with `codex-rs/tui/src/app/side.rs` in
the neighboring Codex checkout. The neighboring OpenCode fork implementation
was also reviewed. Agena retains its own storage memberships and the persistent
side/temporary BTW behavior described above.
