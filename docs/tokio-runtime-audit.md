# Tokio runtime responsiveness audit

Audit date: 2026-10-06. This audit follows production call paths from HTTP,
session execution, plugin callbacks and runtime maintenance into synchronous
dependencies. A synchronous helper is a problem when its async caller runs it
on a Tokio worker, or when another operation holds a shared lock for a long
time. The presence of `std::fs` or `std::sync::Mutex` alone is not sufficient
evidence of a problem.

## Confirmed problems and repairs

| Area | Previous behavior | Repair |
| --- | --- | --- |
| Plugin storage and secrets | Async host callbacks directly entered synchronous storage, file locks and OS credential operations. | Resolve the task-local plugin/session/workspace identity first, then execute the synchronous port on bounded workers. |
| Configuration | Startup, reload and watch metadata scans performed synchronous file access on the async task. | Offload complete load/scan operations. Watch admission observes shutdown while waiting. |
| Settings and Provider Studio | HTTP/settings/save paths called synchronous reads, JSON validation and atomic edits directly. | Application-owned worker boundary covers file reads and edits. Reload is awaited after the edit finishes. |
| Provider authentication | OAuth completion/refresh and credential resolution synchronously read and rewrote configuration files. | Share a bounded credential-store worker pool; keep the store alive with `Arc`. |
| Provider cache identity | Google ADC prompt-cache identity could synchronously inspect and read credential files each time a session prepared a request or projected usage. | Async provider-shape projection moves adapter identity computation to bounded workers without changing its fingerprint rules. |
| MCP credentials | Async OAuth load/save/clear, connection setup and status projection invoked synchronous Keychain ports. | Bound and offload Keychain work, after releasing manager registry locks. Connection timeouts can continue to be scheduled. |
| Password authentication | Login verification and password setup performed Argon2 on the Tokio task. | Separate two-slot password pool. Worker errors fail closed; lockout and session activity timestamps use completion time. |
| OAuth callback waiting | Application held a blocking-pool thread while a synchronous adapter launched and joined another runtime thread for a potentially minutes-long callback wait. | Runtime authentication port awaits the existing Tokio/Axum listener directly. An abort-on-drop handle owns the callback server. |
| Concurrent session tools | Dropping a vector of `JoinHandle`s detached existing tools when permit acquisition was cancelled or a join failed. Detached work could continue to occupy shared tool permits. | A `JoinSet` owns all children through cancellation, early return and panic. Children await permits asynchronously. Results are restored to input order. |
| Prompt construction and compaction | Oversized tool output was hashed, truncated and synchronously spilled to disk while building each model request. | Move the owned prompt turns into a bounded worker, including the spill and preview construction. Cover ordinary replies and both compaction paths. |
| User media and direct image tools | Image decode/hash/base64 encoding ran partly on the async task; user media workers had no admission bound. | Bound image/media processing, including attachment base64 encoding and generated artifact preparation. |
| Provider-backed tools | Up to 96 MiB of JSON could be decoded on the async task; image decode/hash and receipt redaction/serialization also ran there. | Bound JSON and image CPU work, pass owned request bodies and share responses through `Arc` when preparing receipts/images. Existing response/image limits remain. |
| Native and Wasm plugin loading | File hashing, signature checks, dynamic library loading and Wasm compilation/instantiation ran in async preparation. | Separate bounded native/Wasm load workers. Keep fail-closed signature checks and Wasm initialization fuel. |
| Wasm dispatch | Per-plugin admission limited each plugin to one call, but there was no bound across plugins; JSON encode/decode still ran on Tokio. | Two-slot global Wasm execution pool, plus the existing per-plugin permit. Serialize, execute and decode inside the worker; keep fuel, pointer and frame checks. |
| Plugin stdio and LSP | Large outgoing JSON and incoming frames could monopolize a worker; buffered records could repeatedly be ready without yielding. | Bound outgoing serialization, offload incoming JSON above 64 KiB, and consume cooperative budget in buffered output loops. Capture host callback authority before offloading serialization. |
| SQLite transcripts | History/page/batch reads decoded part JSON directly after async database I/O. Single-part/run reads and principal part writes also performed codecs on the async task. | Separate read and mutation codec pools. Mutation decoding/encoding does not queue behind history reads, and codec closures never call the database. |
| Process tools | Foreground cwd checks, sandbox path inspection and background ownership canonicalization could perform filesystem work on Tokio. | Async cwd/ownership metadata paths, bounded sandbox preparation, and background ownership resolution inside its existing worker. |
| Process logs | Host/activity log long polls used unrestricted `spawn_blocking` calls, which could accumulate threads for 30-second waits. | Independent sixteen-slot log admission. Process stop/control does not wait for those slots. |
| Web UI terminals | PTY/tmux creation, restore, stop and post-exit checks were reachable directly from HTTP/background async tasks. Lifetime PTY readers and child waits occupied Tokio's shared blocking pool. | Bound lifecycle and control work separately; use dedicated read/wait threads for the at-most-twenty live terminals; separately bound writes/resizes. Creation and restore share capacity locking. |
| Web UI terminal exit | Exit notification could be lost before the lifecycle subscriber was installed. | `watch::send_replace` retains exit state even without a subscriber. |
| Preview processes | Async start synchronously checked directories, created log directories and opened output files. | Prepare cwd and log file handles in a bounded worker; retain Tokio process execution. |
| Browser activity | Browser startup held a synchronous registry mutex for seconds. Async actions, touch calls and lease destruction acquired that same mutex. | Separate activity bookkeeping into a short memory-only mutex. Idle shutdown checks registry and activity in a fixed lock order, then terminates the browser outside both locks. |
| Noisy shell output | Each invalid UTF-8 segment drained and shifted the remaining buffer, causing repeated copying for binary output. | Decode using a consumed offset and drain once, preserving incomplete trailing code points and EOF replacement behavior. |
| Noisy monitor feeds | Buffered lines or WebSocket frames could keep a loop ready for long periods. | Explicit cooperative budget consumption in the feed loops, including filtered lines. |

## Admission and cancellation contract

`crates/agena-async` supplies `BlockingPool`. Its sequence is:

1. Await an admission permit on the async task.
2. Move the permit and owned operation into `spawn_blocking`.
3. Abort queued work when the calling future is dropped.
4. If work has already started, retain the permit until the closure actually
   finishes, even if the calling future has disappeared.

The fourth rule prevents repeated request cancellation from bypassing the
concurrency limit. Rust/Tokio cannot forcibly interrupt a running synchronous
closure. Process operations that need prompt cancellation still require their
existing cancellation tokens, process kill handles and cooperative checks.

Different admission pools share Tokio's blocking executor. They independently
limit classes of work; they are not dedicated executors or a hard reservation
of CPU cores. Dedicated threads are used for lifetime PTY read/wait operations
because those tasks are intentionally not finite blocking jobs.

Initial limits are deliberately small for expensive CPU work:

| Class | Concurrent operations |
| --- | ---: |
| Argon2 checks/hash/setup | 2 |
| Runtime image processing / user media / generated images | 2 per class |
| Provider-backed tool JSON / image preparation | 2 per class |
| Plugin stdio JSON / LSP JSON | 2 per class |
| Wasm execution across plugins | 2; also 1 per plugin |
| Native load / Wasm load / signed stdio verification | 2 per class |
| SQLite history decoding / mutation codecs | 2 per class |
| Prompt output preparation/spill | 2 |
| Provider credential store | 8 |
| Provider prompt identity | 4 |
| MCP Keychain / plugin secret store | 4 per class |
| Runtime files / application files / plugin storage / LSP root lookup / sandbox policy / preview files | 8 per class |
| Host/activity long log reads | 16 |
| UI terminal lifecycle / stop-control / writes-resizes | 8 / 4 / 16 |

These are admission bounds, not measured throughput targets. Under saturation,
requests await admission instead of expanding the blocking queue. Limits may
need tuning from actual concurrent workloads; increasing Tokio worker counts
does not repair a blocking call left on an async worker.

## Reviewed boundaries that already use the appropriate model

- Ordinary shell execution uses Tokio child processes, concurrent pipe drains,
  bounded capture and process-tree cleanup. Its PTY tool driver sleeps on its
  dedicated native thread.
- The cdylib transport executes plugin dispatch on a plugin actor thread.
  Its `blocking_recv` is appropriate there; preparation/loading needed the fix.
- Built-in filesystem/code operations and bundled memory/Tantivy operations
  already acquire worker permits before synchronous work. Memory index rebuilds
  and search were followed through their async plugin entrypoints.
- Browser launch, process status/shutdown and extraction have worker entrypoints;
  the shared activity lock was the path that still blocked async actions.
- MCP lifecycle gates serialize operations per server. Registry snapshots are
  taken before awaiting external connection/credential work.
- SeaORM/SQLx supplies async database operations. Offloading row/payload codecs
  does not move the database transaction or its SQL operations to blocking code.
- Short in-memory permission, execution, activity and revision-clock locks do
  not become I/O locks merely because they use a synchronous mutex. The
  execution-permit drop fallback uses `blocking_lock` only without a Tokio
  runtime; in a runtime it schedules asynchronous cleanup.
- Runtime shutdown admission already uses a cancellation token and releases
  its publication lock before dropping task guards.

Synchronous public helpers retained for synchronous callers still require an
appropriate worker boundary when reused by new async code. The audit does not
claim that every CPU-heavy projection in the repository has become asynchronous.

## Verification and practical limits

Only compilation and source checks were requested. No new tests were added and
no test suite or load benchmark was run. Existing private test fixtures were
adapted where their helper's argument or bookkeeping type changed.

The modified Rust files pass `rustfmt` checks and the patch passes
`git diff --check`. Compilation checks use the locked dependencies and the
local `aarch64-apple-darwin` target:

```sh
cargo check --offline --locked -p agena --release --target aarch64-apple-darwin
cargo check --offline --locked --workspace \
  --features agena-plugin-host/wasm,agena-plugin-host/signing \
  --release --target aarch64-apple-darwin
```

No dependency version upgrade or increase of Tokio runtime worker counts is
part of this repair. Cross-target behavior, real throughput, memory peaks and
tail latency require runtime measurements. Useful follow-up measurements are
HTTP response latency and cancel/stop latency while running concurrent
terminals, large transcript reads, image tools, plugin callbacks and reloads.
