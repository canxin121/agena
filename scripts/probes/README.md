# Public HTTP and real TUI experience measurements

`http_experience.py` launches one explicitly supplied Agena binary and a local,
controlled Responses provider. It uses Python 3.10+ standard-library modules;
no package installation, real provider credentials, user database, or user
configuration is involved. It never invokes Cargo. Build both binaries first,
then run the measurements serially on the same machine.

The probe refuses an existing output directory. A run preserves its owned
workspace, SQLite files, configuration, process log, all raw HTTP/provider
requests and responses, SSE frames, exact received text, per-delta samples,
and SHA-256 artifact manifest. A failure also produces `result.json` and retains
partial artifacts. Do not reuse an old evidence directory.

## Workload and timing

By default each observer count, `1` and `8`, runs twenty independent trials.
Every trial uses three fresh sessions:

1. Submit a real Composer document through `POST /sessions/{id}/messages`,
   validate its accepted execution/turn/reply receipt, and measure the whole
   HTTP confirmation. The local provider emits exactly 100 Text deltas of
   1,024 UTF-8 bytes each. Each starts with its three-digit index and ends in a
   newline; the exact payload is the existing resource probe's Text payload.
   Each observer consumes the public **content SSE** endpoint. Resource
   discovery polls the public bounded Parts endpoint every 5 ms, beginning
   before submit. The first-character measurement includes discovery and SSE
   attachment. After delta 0, the provider waits until every observer receives
   that marker; deltas 1–99 then use a requested 2 ms spacing. This explicit,
   recorded bootstrap barrier makes the sustained increment measurement
   independent of initial attachment. It is identical for both binaries.
2. Submit a held real provider generation, consume its actual partial Text
   through content SSE, then cancel the accepted execution via HTTP. Measure
   cancellation confirmation and separately cancellation-to-Ready. Require
   the held provider socket to disconnect as well as the public terminal state.
3. Set workspace read/write and the exact owned `permission-fixture.txt`
   path rule to Ask (the shipped temporary-directory rule otherwise allows
   files in this fixture), then submit a native
   `tools_call` for `fs.read` on an owned fixture file. Wait for the actual
   pending permission in the public state, reply Allow Once through HTTP,
   measure the complete reply confirmation, and require the following
   provider request to contain the actual file bytes. Verify the final Text
   and absence of a pending interaction. This exercises the real permission
   gate and filesystem tool, rather than synthesizing a UI-only interaction.

Provider `send_started_ns` is sampled immediately before the SSE delta write;
`send_completed_ns` is also retained. Arrival is sampled after the entire
public content SSE event is received and before JSON decode. This is a
client-visible event boundary, not a claim about the first TCP packet. The
probe verifies every UTF-8 byte as an ordered prefix, rejects duplicated or
reordered cursors, gaps, epoch changes and dropped bytes, and maps arrival byte
ranges back to each original delta. Coalescing and arbitrary Text chunk sizes
cannot make missing, repeated, reordered, or truncated markers pass.

All provider and observer timestamps use the same harness process's
`perf_counter_ns` clock. Both first-byte and complete-delta arrival timestamps
remain in the raw data. `summary` reports the nearest-rank p50/p95/max for
send, source-to-first-content, submit-to-first-content, sustained complete
increment, cancel, terminal cancellation, and permission reply. It also reports
the slowest first-character observer per trial. Observer samples within one
trial share a provider and are correlated; the twenty independent trials and
three separate process repetitions remain identifiable.

## Isolation and process provenance

The parent environment is never modified. Each probe creates one private
`<run>/data` directory and selects it through `AGENA_DATA_DIR` and
`--data-dir`. The server and its thin CLI/TUI clients share that root. The
configuration is `data/agena.json`, the only SQLite database is `data/agena.db`,
and discovery uses `data/run/server.json`. Workspace memory, artifacts,
credentials and client state follow the same root. The project config remains
`.agena/agena.json` in the owned source workspace.

The child receives a small allowlist of OS process variables, including the
unchanged home and temporary-directory locations. Agena isolation does not
redirect `HOME` or `USERPROFILE`; the selected data directory determines every
application-managed path. There are no inherited proxy settings, provider
keys, credentials, passwords, shell profiles or server discovery records.
The optional Runtime ownership audit is recorded at
`data/run/runtime-ownership.jsonl`. The owned global config disables provider
client-version automatic updates, so no npm refresh competes with the
measurement.

Before any public mutation, the server record and `/health` must agree on the
launched child PID, server ID, start timestamp and protocol version. Results
record the exact binary path/hash/size, source revision supplied by the caller,
harness source hashes, command, cwd, complete child environment, PID, spawn,
ready, termination, wait/reap and exit boundaries. The source label describes
the caller's build; the binary hash is the concrete artifact identity. Root
validation should keep its build log and source manifest beside these results.

## Run each binary three times

Build the baseline and current release binaries before measurement, using
separate checkouts and output directories. Both must support the public
protocol exercised by the probe. Record each build's source revision and
binary identity, then use the same harness and workload for both artifacts.
The probe starts each binary with its own fresh database and configuration.

Replace the binary locations and source labels below with the actual build
artifacts. The source label must describe that binary's build, rather than the
checkout from which the Python harness happens to run:

```sh
python3 scripts/probes/http_experience.py \
  --binary /absolute/path/to/current/release/agena \
  --source-revision '<current-build-source-revision>' \
  --label current-r1 --output /absolute/owned/evidence/current-r1

python3 scripts/probes/http_experience.py \
  --binary /absolute/path/to/baseline/release/agena \
  --source-revision '<baseline-build-source-revision>' \
  --label baseline-r1 --output /absolute/owned/evidence/baseline-r1
```

Repeat serially as `baseline-r2/current-r2` and `baseline-r3/current-r3`, using
new directories. Alternate binary order to expose run-order effects. Keep the
same trial counts, observer counts, machine, release profile and provider
settings; the probe records actual provider send intervals so scheduler noise
can be inspected. A small smoke run may use `--trials 1`; keep that separate
from the formal twenty-trial results. Do not present a smoke run as the p95
baseline.

The run's `summary` and full `http-o*/trial.json` files provide the comparison
inputs. Preserve each run's p95, and report its three-run min/median/max along
with the original per-run sample distributions. Differences beyond measured baseline noise
need investigation. This harness does not invent an acceptance tolerance or
automatically convert a slow measurement into a pass.

## Real TUI keyboard, rendering, copy and release

Add `--tui` to a full HTTP run, or use `--tui-only` with a fresh output directory.
The real POSIX PTY companion launches `agena --server ... tui --workspace ...
--session ...`, using the supplied binary, owned environment and the same fake
provider. It runs three separate real TUI processes:

- Send through insert mode, bracketed paste and Enter. Verify the exact user
  document and assistant Text through HTTP reads, then check displayed first
  and last markers and the visible completed `● Answer` header. A normal
  reply with a body intentionally has no extra lifecycle row. Exercise
  Ctrl+Shift+Enter expansion, `gg`, PageDown and `G` scrolling on a 72-line
  Unicode/whitespace fixture with Markdown hard breaks. Invoke the real `/copy-message` action through
  the composer and verify the complete emitted clipboard bytes.
- Render the real `fs.read` permission request, type the published TUI `/allow`
  action and press Enter. Require the actual file bytes in the provider
  continuation, the permission's removal from HTTP state, and displayed
  continuation Text. This uses a real TUI permission command; it does not claim
  to test clicking the inline Allow Once control.
- Send a held provider generation through the keyboard, observe the actual
  partial Text on screen, then press Ctrl+C. Require public Ready, provider
  disconnect and a displayed `Response cancelled` lifecycle.

HTTP is read-only during these keyboard actions. Session/permission setup
happens before launching each TUI; no action has an HTTP mutation fallback.
The native clipboard provider is explicitly disabled, OSC 52 is forced, and
clipboard escapes are captured inside the owned PTY without forwarding them
to the outer terminal. The user's actual clipboard is untouched.

Each flow archives all raw `terminal.ansi` output, timestamped key bytes and
output boundaries, visible screen text at checkpoints, process facts and HTTP
verification. A bounded VT text projection handles crossterm's cursor/erase
operations, UTF-8 and wide characters; raw ANSI remains available for another
terminal renderer to inspect. When `lsof` is available, owned server TCP
connection snapshots are captured before and after the TUI exits and the
probe requires the connection count to return to its pre-TUI level. This
checks connection release on exit; it does not assert allocator/RSS release
or release when merely hiding a pane. Existing app/transport tests cover
those separate state transitions.

The pre-existing `server_restart.rs` PTY helper only attaches a TUI, answers
terminal queries and asserts liveness/single Runtime ownership. It does not
send keyboard input. App tests cover isolated keyboard dispatch and permission
state; the new actual PTY flow supplies the missing process→keyboard→HTTP→tool
and rendering boundary. Validation is pending until these new scripts are
actually run against the final release binary.

## Real CLI settlement and complete output regression

`cli_experience.py` reuses the same isolated server configuration and loopback
Responses fixture for six actual CLI invocations. Run it against the final
current binary after building; it never invokes Cargo:

```sh
python3 scripts/probes/cli_experience.py \
  --binary /absolute/path/to/current/release/agena \
  --source-revision 'completion-working-tree-with-source-manifest' \
  --label cli-current-r1 --output /absolute/owned/evidence/cli-current-r1
```

Four independent generations exercise `exec` and `review`, each in plain text
and JSON mode. The provider sends one partial delta, then blocks on a recorded
gate. While the session is publicly Running, the probe holds the gate for
350 ms and requires the CLI process to remain alive. Only then does it release
the rest of the generation. Exact output comparison preserves Unicode, tabs,
CRLF, whitespace and trailing newlines. An acceptance-only or partial-reply
CLI fails this barrier before it can report success.

The JSON exec case produces more than 9 MiB of native Text, crossing both the
former 1 MiB CLI projection and the SDK's 8 MiB preview ceiling. The probe then
invokes real `debug session` in both JSON and text modes against that session
and requires the same complete body. These three cases compare every byte
and record its length and SHA-256. `large-payload.txt`, every raw stdout/stderr,
each process/gate boundary, HTTP/provider trace, binary/harness identity and
the final artifact manifest remain in the new owned output directory. Client
processes set `AGENA_RUNTIME_BOOTSTRAP_FORBIDDEN=1`, so the thin CLI cannot
silently start another Runtime to obtain its result.

This is a functional regression fixture for the current implementation; its
six cases are separate from HTTP latency distributions and are not a new
performance baseline. Authoring the script does not establish a pass: retain
the actual run result from the final binary, including any failed run.

## Project-scoped TUI Hub

```sh
python3 scripts/probes/tui_hub.py --binary target/release/agena --output target/hub-validation/run-1
# Optional: exercise another terminal width (default: 120 cells).
python3 scripts/probes/tui_hub.py --binary target/release/agena --cols 220 --output target/hub-validation/wide-1
```

This probe starts an isolated instance and drives real PTY keyboard input. It
creates two projects, 24 older pinned sessions and 22 newer unmarked sessions.
It checks the initial project-local New Session selection, Enter creating in
the launch or selected project, independent project buckets, pin pagination,
selection after a live refresh, independent pin/favorite mutations, and compact
rows with explicit name/status/time columns. Root Escape stays on the Hub and
preserves selection; Escape cancels empty or populated searches in place. All
fixture data, HTTP traces and terminal captures stay in the new output directory;
the installed instance and user conversations are untouched.
