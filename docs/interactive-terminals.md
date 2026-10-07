# Interactive shell terminals

Agena's shell tools support persistent pseudo-terminals (PTYs) for REPLs,
interactive installers, command shells, and text-mode full-screen applications.
This is a native process backend; it does not depend on tmux or a desktop terminal.

## Tool contract

All command and terminal tools belong to `agena.shell` and are discoverable by
searching for `shell`, `terminal`, `TTY`, or `PTY`. Choose the launch tool by its
return semantics:

- `shell.exec`: run a noninteractive command and wait for its final result.
- `shell.spawn`: start a noninteractive background job, confirm process creation,
  and return immediately so the AI can continue. Completion is notified once.
- `shell.watch`: start a command with an atomic readiness/output watch, or
  attach, replace or remove the single watch on an existing background process.
- `shell.open`: open a persistent interactive PTY and return after a bounded
  initial output wait. Terminal exit does not automatically wake the AI.

The old `shell.run` execution entry point and its mode flags are retired. Old
transcripts remain readable and old command permission configuration is mapped
to the new launch tools without expanding old command-specific allows to watch controls or terminal input.

Start a terminal with `shell.open`:

```json
{
  "command": "python3 -q",
  "workdir": ".",
  "rows": 24,
  "cols": 80,
  "yield_time_ms": 1000,
  "reads": [],
  "writes": [],
  "network": []
}
```

Use the returned `process_id` for subsequent calls. Process creation must
succeed before the call returns, even with `yield_time_ms: 0`. The terminal
persists until exit, stop, or its explicit timeout. `yield_time_ms` is a bounded
wait for initial output, **not** a process deadline. `timeout_ms`, if supplied,
is the lifetime limit. A quiet prompt does not mean that the program finished.
Regex/quiet-period completion belongs to `shell.watch`; interactive launch
has no monitor mode. WebSocket subscriptions remain under `monitor.start`.

### Watched background commands

`shell.watch` shares the launch preparation, shell selection, command hooks,
permissions, sandbox and process registry used by `shell.spawn`. It returns the
same `process_id` contract: use `shell.read`, `shell.list` and `shell.stop` for
both. There is no separate command `monitor_id` or stop/log tool family.

```json
{
  "command": "tail -f application.log",
  "reads": ["application.log"],
  "writes": [],
  "network": [],
  "timeout_ms": 600000,
  "policy": {
    "ready_pattern": "READY",
    "include_pattern": "ERROR",
    "failure_pattern": "FATAL",
    "pattern_kind": "regex"
  }
}
```

Choose `shell.spawn` when only completion matters. For a server, bind
`ready_pattern` on launch so startup output cannot race with attachment. The
ready notification happens once and **does not stop the process**. `ready` in
shell results/summaries is independent of `status`; a ready service is normally
still `running`.

No ordinary output is notified without `include_pattern`. When supplied, the
first match notifies once per watch configuration. Set `notifications` to
`on_change` only for deliberate recurring updates. Unchanged matches are
suppressed; changes during `notification_interval_ms` are coalesced to the
latest state and delivered after the interval (default 30,000 ms, allowed
1,000–3,600,000 ms). Readiness and final completion bypass this throttle. The
bridge retains at most 6 KiB / 64 selected events for text plus bounded cursor
and archive recovery information. Single-line shortening includes an explicit
omission marker. Notification delivery sequences are independent of raw-log
sequences, so multiple conditions in one pipe read cannot erase each other.

Success/failure patterns stop the whole process tree with the corresponding
outcome; failure wins when both match one record. `quiet_period_ms` stops after
no stdout/stderr activity, including excluded output and incomplete lines.
Quiet time uses a monotonic clock. `timeout_ms` remains the launch's lifetime
deadline even when the watch changes or is removed. All patterns respect
`pattern_kind` (regex by default, or literal); whitespace is significant.
Pattern scanning uses bounded records while capture and diagnostic reads retain
raw chunks, including output with no newline and excluded lines. Readiness can
match an unfinished record; include and terminal conditions use completed
records or the final EOF fragment.

To observe an existing process, use the same tool with a `process_id`:

```json
{
  "process_id": "<existing process_id>",
  "policy": {"ready_pattern": "READY", "include_pattern": "ERROR"},
  "since_seq": 0
}
```

This replaces one watch, never starts a second process or durable operation,
and routes notifications through the original launch. Omit `since_seq` for
future output only; provide it to scan retained logs once. Identical policy
updates preserve readiness and once-only notification state. Invalid patterns
are compiled before replacing the live policy. PTYs, WebSocket subscriptions
and ended processes reject watch attachment.

Remove the watch with `{"process_id": "<id>", "policy": null}` (omitting
`policy` has the same effect). Removal leaves the process, launch deadline and
original completion notification intact. `policy: {}` is a valid quiet watch
with no selected output notifications. Use `shell.stop` to stop the process.

The bounded event worker flushes before completion when possible, with a
two-second wait limit if delivery stalls. The latest pending recurring match
is considered when the process exits, without producing more workers per line.

`shell.write` sends nonempty exact UTF-8 input and returns incremental output.
It never trims text or adds an implicit newline:

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

Use `shell.read` to read without typing; empty `shell.write.chars` is rejected:

```json
{
  "process_id": "<returned process_id>",
  "wait_ms": 250,
  "include_screen": true
}
```

`shell.read` supports background commands and PTYs. With `since_seq` and
`event_offset` omitted, reads (and PTY writes) consume only unread output.
Explicit cursors replay without moving the automatic cursor. `shell.logs`
remains the explicit-replay compatibility entry, defaulting to `since_seq: 0`.

`last_seq` refers to fully consumed events. If an event exceeds the output
budget, `next_event_offset` records the byte position in that next event; no
remaining text is silently discarded. Continue with both values:

```json
{
  "process_id": "<id>",
  "since_seq": 12,
  "event_offset": 4096,
  "max_output_bytes": 4096,
  "wait_ms": 0
}
```

Here `4096` is an example returned `next_event_offset`, not a guessed cursor.
Nonzero offsets require an explicit `since_seq`, must preserve UTF-8 boundaries,
and reject an evicted partial event with an archive recovery instruction.
Automatic cursors recover to the oldest retained output after buffer eviction;
loss counters remain visible. Cursor validation occurs before any input.

`max_output_bytes` is shared by exec/open/read/write/logs (default/max 16,384;
minimum 1,024). Escaping and event metadata count toward the content budget;
space is reserved for lifecycle/cursor/recovery information. Byte and line
bounds are applied before consumption, keeping normal results within the global
model-history boundary. Smaller previews do not reduce capture. `include_screen`
defaults to false; it is available only for PTYs and splits the content budget
between incremental output and bounded screen text. `shell.list` covers owned
jobs and terminals. Diagnostic waits are capped at 30 seconds and never change
process lifetime. Background completion notices make polling unnecessary.

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

### Windows input

ConPTY expects its own key encoding, so input bytes are normalized on the way to
the pseudoconsole:

- A line feed becomes a carriage return. A CRLF pair submits one line even when
  the two bytes arrive in separate `shell.write` calls, because the driver keeps
  the CR/LF state across writes.
- The C0 backspace byte (`0x08`) becomes DEL (`0x7f`), which ConPTY translates to
  `VK_BACK`.
- Every other byte, including UTF-8 sequences, `\t`, `\u0003`/`\u0004`, and
  escape sequences, passes through unchanged.

Normalization does not change the write contract: a successful write still
acknowledges the caller's own input bytes, so the partial-write detection above
keeps rejecting ambiguous input instead of retrying it. Unix terminals keep
their existing byte-for-byte behavior.

This normalization is unit-tested on every platform, and its wiring compiles for
the Windows target. A real ConPTY session must still be exercised on a Windows
host; platform behavior is not inferred from macOS tests.

## Output, screen, and bounds

Shell output has separate **preview** and **capture** limits. Foreground
commands show at most 16 KiB / 200 lines, retaining the beginning and end.
Background log events have a 16 KiB serialized budget, including metadata and
JSON escaping. PTY reads return at most 16 KiB of incremental text. The session
layer's existing 50 KiB / 2000-line model-result boundary remains the final
guard for every tool's complete rendered response.

Before preview shortening, filtering, line decoding or rolling-buffer
eviction, process output is captured into a private UTF-8 file under the
workspace's managed tool-output directory. Large foreground results and
background/PTY results expose an optional `output_archive` with `path`,
`retained_bytes`, `total_bytes`, `pending`, `complete`, `truncated`, `limit_bytes`
and optional `error`. Small foreground results already fully visible in the
preview discard their temporary file. Captured stdout/stderr are interleaved
in observed read order; command-after plugins may render a different preview.

Search an archive with `fs.grep` and read only relevant lines with `fs.read`.
Do not send its entire contents back to the model. For a giant single line,
line-based readers/searchers have their own limits: locate a match with a
bounded command such as `rg -b -o -- 'needle' '<archive path>'`, then request a
small byte range:

```json
{
  "file_path": "<output_archive.path>",
  "mode": "text",
  "byte_offset": 120000,
  "byte_limit": 4096
}
```

Byte positions are zero-based; continue with `read_info.next_byte_offset`.
Byte ranges preserve complete UTF-8 code points and cannot combine with
line/entry `offset` or `limit`. A growing file is reported as a live sample.

Capture retains at most **16 MiB per process** with a stable **4 MiB startup
prefix** and up to **12 MiB of recent output** in rotating 4 MiB segments. The
shared workspace quota is **256 MiB**. `output_archive.path` identifies the
stable prefix; `segments` lists every retained file with original captured
`start_byte`/`end_byte` ranges. Read a segment at file-local offsets beginning
at 0; subtract its `start_byte` when locating an original captured offset.
Missing ranges are unavailable output and are never concatenated as though
continuous. Rotated paths can expire while a process runs; read the current
segment list before recovery. Rotation or storage failure marks `truncated`;
`complete` is true only for a finished, fully captured archive with no pending
writes or gaps. Recent raw output also remains in the in-memory rolling buffer. `has_more=false`
describes buffered events, not archival completeness. Active captures can
have pending writes. Files of deleted sessions use the existing managed-output
cleanup; stateless calls use session directory `0` within the same quota.
One dedicated archive I/O thread and a bounded queue keep file creation, writes, rotation, deletion and normal close off Tokio workers. PTY queue pressure pauses only output draining while input,
signals, timeout checks and cleanup continue.

Interactive results return incremental `output`, a cursor, lifecycle state and
explicit loss indicators. They do not repeat the same output in event arrays or
process summaries. With `include_screen: true`, the optional `terminal` screen
projection contains dimensions, zero-based cursor position, cursor
visibility, alternate-screen state, bracketed-paste mode, application-cursor
mode, bounded text, and a truncation indicator. `ProcessSummary.tty` distinguishes
PTY chunks from other process types; noninteractive pipes also retain raw chunks. PTY stdout and stderr
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
| Model preview budget | 1–16 KiB, shared with an optional screen |
| Foreground capture in memory | 1 MiB per stdout/stderr stream, head/tail |
| Disk output retention per process | 4 MiB startup + up to 12 MiB recent segments |
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
the runtime. Interactive terminals do not create durable completion-notification
rows; durable reconciliation applies to background jobs and monitors.

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
