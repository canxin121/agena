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

## Follow-up: database contention and storage locks

The second pass follows database callers through transaction ownership, codecs,
connection checkout, retry, cleanup, and cancellation. Async SQL alone did not
prevent a writer queue from monopolizing connections or CPU preparation from
prolonging a write transaction.

### Database topology and write admission

Chat/session data, the scheduler, and web-server state use separate SQLite
files. Crawl metadata uses a workspace-specific redb file and document/index
files. A single SQLite database remains a single writer in WAL mode; extra
connections permit concurrent reads, not parallel writes.

`agena-async::WriteQueue` now provides process-local FIFO writer admission:

- Each canonical database file has one shared queue, including independently
  opened connection pools and path aliases. Different files have independent
  queues. Named/anonymous memory databases use their SQLx identities.
- Admission precedes connection checkout and transaction creation. At most 128
  writers are admitted, including the active writer; queued admission expires
  after 15 seconds. Capacity is returned on cancellation/error.
- SQL write permits are retained through commit/rollback and released before
  transaction effects. SQLite/SQLx still handles statement execution and queued
  rollback cleanup when an async future is cancelled.
- Runtime file-backed pools retain 16 connections, with a three-second checkout
  timeout. SQLite busy waits are one second per connection instead of 15.
- The session transaction fence still acquires SQLite's write lock before
  application reads. Cross-process fence conflicts roll back before backoff;
  at most five retries occur, checking a ten-second budget between attempts.
  The mutation/effects closure is not replayed. The budget does not forcibly
  interrupt an in-flight SQL operation or impose a deadline on an entire import.
- Queue/checkout congestion preserves a transient failure classification.
  Session/runtime API boundaries expose dependency-unavailable/503 responses;
  an exhausted optimistic message update exposes a conflict/409 response.

The queue coordinates this process. Other processes are coordinated by SQLite's
write lock and version/claim predicates, not by the in-process semaphore.

### Call paths reviewed and repaired

| Boundary | Finding | Change |
| --- | --- | --- |
| Session engine and shared transaction helpers | Writers occupied pool connections while racing the one SQLite write lock | Shared bounded admission before checkout; short driver waits and bounded fence retries |
| Message delta and run completion | Large decode/compare/clone/encode work occurred while holding the writer transaction | Read and prepare outside the transaction; update with a part revision predicate; bump member-session versions atomically after a successful CAS; reload/reapply pure deltas on a bounded conflict loop |
| New user messages, appended parts, background companion parts | One encoder admission and one allocator UPDATE per part inside the transaction | Encode the owned batch first; reserve the ID range with one transactional allocator UPDATE; insert in existing reference order |
| Background launch validation, fork cutoff validation | Validation fetched whole tool results/provider continuations | Select only part identity, role, state, ownership, references, and timestamps |
| Background events and transitions | Replacement outcome/failure/notification payloads were serialized under the write lock, sometimes twice | Prepare replacements before admission; SQL COALESCE preserves omitted fields; move rather than clone the runtime-ingress notification |
| Delivery retry/failure and usage detail | Potentially large JSON encoding ran on the async caller or in a transaction | Bounded encoding before write admission; direct usage/cancellation statements also join the shared writer queue |
| Cancel/reconcile | One full result read/decoder per changed part under the write lock | One result-row query; decode the captured rows after commit and release of write admission |
| In-flight session/run listing | Pure SELECTs opened fenced write transactions | Read directly through the pool without writer admission |
| Session fork/rewind | Fetch every history edge and issue one insert per included edge | One INSERT ... SELECT with the original cutoff comparison, inside the atomic transaction |
| JSONL export/import | Full serialization/parsing and repeated payload encoding ran on the async task or in the write transaction | Bounded workers prepare payloads first; import retains atomicity and remaps IDs without cloning large part payloads under the lock |
| Orphan maintenance | Unbounded deletion scans held the writer lock; empty ticks also acquired it | Select candidates using reads, revalidate/delete at most 256 leaves per transaction, at most four batches per tick; commit/yield between batches; preserve membership, parent/run and background foreign-key references |
| Workspace resolution | Known workspaces still issued INSERT ... DO NOTHING on common request paths | Return the read lookup for known paths; only misses join the writer queue |
| Workspace/permission CRUD | Independent writes bypassed shared admission; read/write/read sequences raced other writers | Shared admission and RETURNING where appropriate; permission upsert's insert/update/read-back is one fenced transaction |
| Model catalog | Per-model serialization/search-text preparation and one SQL round trip per model under the lock | Prepare statements on a bounded worker before admission; insert up to 100 models per statement; retain the atomic freshness gate |
| Model catalog reads/invalid-cache cleanup | Metadata and entries could come from different snapshots; stale corruption cleanup could clear a newer refresh | Capture state and entries in one read transaction, release before decoding, and recheck the captured timestamp before clearing |
| Scheduler writes and audit history | Claim/renew/edit/finalize writes competed for connections; history encoding ran after the job update | Per-file writer admission on every mutation; prepare job/history JSON first; preserve claim CAS, renewal ownership, and job/history atomicity |
| Scheduler reads | JSON decoding ran on async callers; a boolean pending-jobs query materialized all pending job payloads | Bounded row decoding after checkout release; SELECT EXISTS for the boolean query |
| Server-state KV | Every snapshot wrote again and typed JSON passed through two conversions | Per-file writer queue; encode/decode owned typed values once on bounded workers; unchanged payloads skip the UPSERT update |
| Database bootstrap/schema locks | Directory creation, schema-lock file open/try_lock ran synchronously inside async bootstrap | Bounded worker boundaries; retry advisory-lock contention with async sleeps |
| Revision safety refresh | Concurrent browsers queued behind one slow cross-process metadata read; failures immediately retriggered reads | One refresh proceeds; other polls use the locally observed revision clock; retain a two-second attempt cooldown on errors/cancellation and the existing 30-second successful-check interval |
| Async web crawl | redb access, document/cache file I/O, content preparation, prune, and index rebuild were invoked directly on Tokio workers | Bounded read/content/write workers; per-store mutation admission with permits retained inside started closures; network fetch/authorization futures stay in the original task |
| Crawl metadata handle registry | Opening/validating one file held the global registry mutex across disk I/O | Snapshot a per-file initialization slot under the registry lock; open/validate outside the global lock; retain/release shared redb handles correctly |
| redb last-handle release | Closing the database can synchronously flush its shutdown header on the async caller | The shared resource destructor hands the retained metadata handle to a separate bounded close worker, including concurrent clone releases; serialize actual close and new open under the same per-file slot lock |
| Web local search | A local query waited on the crawl mutex throughout a crawl's network phase | Share only the storage/index gate needed for a consistent snapshot; no wait on the crawl's network phase |

SQLite statement execution remains asynchronous through SeaORM/SQLx. Neither
transactions nor async driver operations are wrapped in `spawn_blocking`.
Some reads required for atomic metadata/background validation still decode
inside a transaction; these have a reserved decoder limit of one and do not
queue behind transcript reads or message preparation. Normal payload decoders
and mutation preparation remain separately limited to two workers per class.

These changes do not alter the persistent table/index/trigger schema or require
recreating a database. GC is now incremental: a tick reports only the parts
removed by its bounded batches, and later ticks drain the remaining orphans.
Long atomic imports/catalog replacement can still hold a writer while their SQL
statements execute; preparation and avoidable round trips have been removed,
while splitting a single logical import across commits would change its contract.

## Follow-up: transport, snapshots, streaming, and native cleanup

The third pass checks the CPU and resource-lifetime work around otherwise async
database and network calls. Async SQL or `reqwest` does not move a caller's
history clones, JSON codecs, filesystem validation, or native destructors off
Tokio workers. Cancellation also does not stop an already-running blocking job.

| Boundary | Finding | Change |
| --- | --- | --- |
| Workspace file/list APIs | Canonical path checks, metadata, directory scans, file reads, hash/base64 work, writes and fsync ran in application async methods | Move the complete filesystem operation to admitted workers; leave SQL in its async driver; use bounded reads rather than trusting metadata size |
| Plugin marketplace | Async REST handlers invoked a synchronous marketplace client using `reqwest::blocking`, including client construction/destruction | Separate read and mutation worker admission; construct, use and release the blocking client on the worker |
| Memory REST and cron tools | Memory file helpers and workspace canonicalization remained synchronous at some async entrypoints | Use application file workers for all memory routes and Tokio filesystem canonicalization for cron ownership checks |
| HTTP, WS, SSE, and IPC payloads | Large snapshots and protocol JSON were serialized or decoded on Tokio | Separate bounded encode/decode pools; pre-encode principal HTTP/conditional transcript responses; move large incoming frames to workers without first copying their full string |
| Connection task ownership | Dropping a writer handle or subscription map could leave writer/producer/subscription children running after the connection owner disappeared | Abort-on-drop ownership for WS/IPC writers, subscription children and the SSE producer; consume cooperative budget in buffered loops |
| Legacy JSON-RPC | Envelope, params, response-value conversion and result encoding ran on async callers; notification subscribers copied full payloads | Bound the complete codecs, share notifications through Arc, poll the WS reader during event subscriptions, and bound stdio records |
| Filesystem watcher lifetime | macOS notify shutdown can synchronously join its native thread when an SSE stream is dropped | Worker setup, two dedicated close threads, and a 64-resource lifetime limit that includes queued closes; Drop only hands ownership to the closer |
| Watcher event callback | A native event could map many paths while holding the mutex used by the async stream | Prepare at most 256 source paths and a 256-path output batch outside the shared mutex; retain truncation/rescan/reconnect recovery |
| Session snapshots and cache | Full histories were cloned/projected on async callers or under a cache-wide payload mutex; each part merge scanned history | Registry holds handles/recency only; per-session payload locks and deep copies live on workers; part-index lookup merges only changed parts; stale/version-gap updates cannot make a cache look current |
| Streaming text and reasoning | Every text increment copied the accumulated content; each reasoning increment decoded/cloned/encoded the entire summary array; stream initialization loaded full history | Append in place on a per-part worker, return a lightweight revision/time checkpoint, initialize from one part row, and retain the original flush-count/terminal-boundary rules |
| Streaming flush failure/cancellation | Removing buffers before durable success lost retry state; cancellation could release a part gate while detached worker cleanup still ran | Keep buffers until successful flush, remember requested flushes across errors, and let each started mutation/cleanup worker retain the acquired part gate until it finishes; release the gate before observer notification |
| Plugin HTTP transport | Owned request/response JSON and stream request values were copied/encoded/decoded on async tasks | Two-slot HTTP codec admission; capture callback context before offloading, preserve frame/ID/authority checks and shutdown selection |
| Provider request/response path | Large prompt/media projection, request JSON and diagnostics repeated on async callers or credential retries | Owned worker projections for OpenAI Chat/Responses, Anthropic, Gemini HTTP/realtime, Ollama and both Bedrock protocols; encode once into shared Bytes; offload normal/error response decoding and diagnostic preparation; Bedrock signing has separate bounded admission |
| OpenAI compact/direct-image paths | Compact preparation cloned the entire history; image inputs and image result data URIs were copied before/after the wire codec on Tokio | Project compact/image requests on the request worker, temporarily take/restore compact input fields instead of cloning history, and project image artifacts on the independent response worker |
| Provider extension headers | The synchronous header-enrichment port can call arbitrary extension code from an async request path | Four-slot worker boundary, with the original task's cancellation context captured before the move; no installed production hook was found during this review |
| Provider SSE/JSON-lines | Recursive UTF-8 repair and repeated tail draining copied noisy buffers; large frame JSON parsed inline; buffered loops could stay ready | Iterative UTF-8 handling, one tail drain per chunk, reuse parsed JSON for completion checks, offload large chunks, bound frames/pending data and consume cooperative budget |
| Gemini realtime frames | Text/binary WS messages were copied into new strings and decoded inline; outgoing setup/content copied their JSON subtrees | Move owned/shared bytes into size-aware decoding; project and encode both outbound records on the request worker without subtree copies |

Additional admission limits introduced in this pass:

| Class | Concurrent operations |
| --- | ---: |
| Workspace scans / transfers / canonical identity | 4 / 2 / 8 |
| Marketplace reads / mutations | 4 / 2 |
| API JSON encoding / large-frame decoding | 2 / 2 |
| Legacy JSON-RPC codecs | 2 |
| Watcher setup / native close threads / admitted live-or-closing watchers | 4 / 2 / 64 |
| Session snapshot/projection / updates and notification preparation | 2 / 4 |
| Runtime SessionView-to-Session projection | 2 |
| Plugin HTTP codecs | 2 |
| Provider request projection/codecs / response codecs / stream decoders | 2 / 2 / 2 |
| Provider header hooks / Bedrock signing | 4 / 2 |

The per-part streaming gate is an async mutex. Buffer payload mutexes are only
taken on admitted workers. A clone of the *already acquired* owned gate moves
with a started mutation or cleanup; no worker uses `block_on` to reacquire it.
The async caller retains the gate across its durable flush, then releases it
before observer fanout. Timer notifications clone only the relevant live part,
and do not announce terminal state while its flush is still pending. Run-tail
flushes announce earlier successful parts even if a later part fails. A failed
or cancelled flush retains its registered buffer for retry.

Workspace download/media/upload bounds remain 100/20/50 MiB. Reads use a
limit-plus-one check, and uploads check encoded length before base64 allocation.
IPC and legacy JSON-RPC stdio now accept at most 64 MiB of record content, with
separate LF/CRLF allowance. Provider SSE frames/pending data and realtime JSON
frames are bounded at 64 MiB. Small incoming JSON frames below 64 KiB stay
inline; large ones pass worker admission. These are size and concurrency bounds,
not a universal memory budget or a measured latency guarantee.

Wire JSON and conditional-response headers are preserved. The Rust-only legacy
JSON-RPC notification subscription now returns `Receiver<Arc<...>>`; consumers
share the immutable notification instead of cloning its payload. The API Cargo
features declare the HTTP common-code dependencies of WS/SSE/JSON-RPC, with IPC
depending on WS, so reduced-feature builds do not rely on default features.

The audit follows the production call paths recorded above; it is not a proof
that every possible future or public synchronous helper has an async boundary.
Some typed response-to-domain projections and small per-event projections
remain synchronous. Short handle-registry locks remain synchronous where their
critical sections perform no I/O or deep payload copies. Those boundaries
should be reviewed again when payload shapes or extension callers change.

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
cargo check --offline --locked -p agena-api-server --no-default-features \
  --features http --release --target aarch64-apple-darwin
cargo check --offline --locked -p agena-api-server --no-default-features \
  --features sse --release --target aarch64-apple-darwin
cargo check --offline --locked -p agena-api-server --no-default-features \
  --features ws --release --target aarch64-apple-darwin
cargo check --offline --locked -p agena-api-server --no-default-features \
  --features jsonrpc --release --target aarch64-apple-darwin
```

No dependency version upgrade or increase of Tokio runtime worker counts is
part of this repair. Cross-target behavior, real throughput, memory peaks and
tail latency require runtime measurements. Useful follow-up measurements are
HTTP response latency and cancel/stop latency while running concurrent
terminals, large transcript reads, image tools, plugin callbacks and reloads.
