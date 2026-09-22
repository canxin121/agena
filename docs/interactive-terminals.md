# Interactive shell terminals

Agena's shell tools support persistent pseudo-terminals (PTYs) for REPLs,
interactive installers, command shells, and text-mode full-screen applications.
This is a native process backend; it does not depend on tmux or a desktop terminal.

## Tool contract

Start a terminal with `shell.run`:

```json
{
  "command": "python3 -q",
  "tty": true,
  "workdir": ".",
  "rows": 24,
  "cols": 80,
  "yield_time_ms": 1000,
  "reads": [],
  "writes": [],
  "network": []
}
```

Use the returned `process_id` for subsequent calls. `tty: true` retains the
process even when `run_in_background` is omitted. `yield_time_ms` is a bounded
wait for initial output, **not** a process deadline. `timeout_ms`, if supplied,
is the lifetime limit. A quiet prompt does not mean that the program finished.
Do not combine a PTY with `monitor`: regex/quiet-period completion belongs to
noninteractive monitored commands.

`shell.write` sends exact UTF-8 input and returns incremental output and the
current virtual screen. It never trims text or adds an implicit newline:

```json
{
  "process_id": "<returned process_id>",
  "chars": "print('hello')\r",
  "wait_ms": 250,
  "reads": [],
  "writes": [],
  "network": []
}
```

Typing and submission can be separate calls. `\r` is the usual Enter key,
`\t` is Tab, and `\u0003`/`\u0004` are the Ctrl-C/Ctrl-D bytes. Their effect
depends on terminal mode and the application: in raw mode Ctrl-C may be ordinary
input, and Ctrl-D is not a universal process-exit command. Arrow/function keys
use terminal escape sequences; consult `terminal.application_cursor` when
choosing cursor-key sequences. For multiline paste, a caller can send bracketed
paste delimiters when `terminal.bracketed_paste` is enabled.

Empty `chars` reads without typing. With `since_seq` omitted, `shell.write`
consumes the terminal's unread-output cursor. An explicit `since_seq` replays
output after that cursor without moving the automatic cursor. Continue from the
returned `last_seq` while `has_more` is true. `shell.logs` also accepts explicit
cursors; `shell.list` includes owned terminals. A cursor beyond the latest event
is rejected **before** any input is delivered.

`shell.resize` takes `process_id`, `rows`, and `cols`. It updates the operating
system PTY size and the virtual screen. On Unix the kernel notifies the
foreground process group of the changed size.

`shell.signal` takes `process_id` and `signal`:

- `interrupt`: Unix foreground-group SIGINT; ConPTY terminal Ctrl-C on Windows.
- `terminate`: end the PTY process session, with a short graceful interval and
  subsequent forced cleanup of remaining managed jobs.
- `kill`: skip the graceful interval.

`shell.stop` is equivalent to terminal `terminate`. Interrupt is distinct from
closing the entire session and from typing a control byte. Stop and signal
requests are not queued behind the ordinary shell output-worker limit.

## Output, screen, and bounds

The usual shell payload is retained. Interactive results additionally contain a
`terminal` screen projection: dimensions, zero-based cursor position, cursor
visibility, alternate-screen state, bracketed-paste mode, application-cursor
mode, bounded text, and a truncation indicator. `ProcessSummary.tty` distinguishes
PTY chunks from noninteractive newline-delimited logs. PTY stdout and stderr
share one ordered stream, as in a normal terminal.

Output is captured in chunks, including prompts without a trailing newline.
UTF-8 code points split across reads are preserved. ANSI cursor motion and
screen redraws update the virtual screen. Host terminal escape sequences are
not executed by the human-facing terminal result renderer. Device/status query
responses are generated from virtual state; host clipboard, window title, and
other host terminal actions are never forwarded.

Current resource limits per registry:

| Resource | Limit |
|---|---|
| Live terminals | 16 |
| Retained live/completed terminal entries | 64 |
| Raw output retained per terminal | 1 MiB and 2048 events |
| Returned raw output / screen text | 16 KiB each |
| Single input | 64 KiB |
| Initial/read wait | 0–30,000 ms |
| Explicit lifetime timeout | 1–86,400,000 ms |
| Dimensions | 1–200 rows, 1–400 columns, at most 40,000 cells |
| Terminal query reply queue | 4096 bytes |
| OSC payload fed to the screen parser | 4096 bytes |

`dropped_bytes`, `dropped_lines`, `has_more`, and the screen's `truncated` flag
make lossy capture explicit. Output flooding cannot grow the capture ring,
reply queue, or unterminated OSC parser buffer without bound. A full-screen
projection is not a graphical terminal emulator: image protocols and arbitrary
desktop interactions are not supported.

## Ownership, cancellation, and permissions

Each terminal is bound to the trusted runtime's canonical workspace and Agena
session ID. Foreign sessions cannot list its metadata, read it, type into it,
resize it, or stop it, including through the `monitor.stop` alias. Stateless MCP
calls share that connector runtime's workspace owner; this is not an additional
per-client authentication boundary.

The launch and subsequent writes each go through the tool permission/effects
pipeline. Declare the filesystem/network effects of entered operations, not
just of the initial shell. Write paths are relative to the Agena workspace.
Shell-prefix allow rules are not approvals for persistent terminal input;
explicit `shell.write` policy and recognizable command denials still apply.
Arbitrary REPL, editor, or fragmented terminal input is not a statically
verifiable shell program. As with existing shell execution, these declarations
and tool checks are **not an operating-system sandbox**. Use restrictive tool
policy or an external sandbox for untrusted workloads.

Successful launch calls detach the PTY lifetime from the individual turn's
cancellation token. Cancelling a later output wait leaves the process alive.
Cancelling/dropping an unfinished launch requests cleanup of its unclaimed
process. Cancelled queued input is not delivered. Partial-write/acknowledgement
failures request termination to prevent ambiguous retries; never blindly resend
the entire input after such an error. If a write call is cancelled, inspect the
current screen before continuing: bytes already acknowledged by the operating
system cannot be rolled back.

Completed output remains readable until bounded-history eviction. Replaying a
retained launch identity returns the existing entry rather than launching it
again. Session end, explicit runtime shutdown, and registry destruction request
cleanup. Terminal handles are in-memory resources and do not survive restarting
the runtime; durable background reconciliation records interrupted operations.

Unix cleanup covers process groups in the PTY's session. A program deliberately
creating a separate daemon session is outside this containment boundary. Windows
uses ConPTY and a kill-on-close Job Object. Platform-specific behavior must be
validated on its target platform, not inferred from macOS tests.

## Validation

Core, lifecycle, screen/protocol and executor-route regressions:

```sh
cargo test --offline --locked -p agena-runtime-tools --lib terminal::
cargo test --offline --locked -p agena-runtime-contracts --lib current_input_contract_tests
cargo test --offline --locked -p agena-bundled-plugins --lib
cargo test --offline --locked -p agena-bundled-plugins --test human_rendering
cargo test --offline --locked -p agena-mcp-server --lib stateless_tool_policy
```

The Unix terminal suite launches real Python REPL-style fixtures, an interactive
bash with foreground job control, and a curses full-screen program. It covers separated typing/Enter, UTF-8, Ctrl-C/EOF,
SIGINT in raw mode, kernel resizing, terminal queries, alternate-screen drawing,
output bounds, cancellation, session isolation, child-group cleanup, completion
callbacks, native tool dispatch, and noninteractive compatibility.
