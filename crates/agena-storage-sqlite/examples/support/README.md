# P1 immutable payload placement experiment

`event_blocks_prototype` compares the same deterministic immutable payloads in
SQLite `BLOB` rows and in merged immutable files. It extends the earlier
`content_blocks_prototype` Text-only experiment. Both implementations live
exclusively in examples; the file adapter is not a production backend.

The implementation is split between
[`event_blocks_prototype.rs`](../event_blocks_prototype.rs),
[`block_comparison_fixture.rs`](block_comparison_fixture.rs),
[`block_comparison_store.rs`](block_comparison_store.rs), and
[`run_event_blocks_matrix.py`](run_event_blocks_matrix.py).

## Equivalent contract

Both adapters use the production `ImmutableTextBlock` and
`ImmutableEventBlock` codecs. Ordinary blocks target 32 KiB; checkpoint/patch
records remain typed, with their exact native sequence, timestamps, channels,
mutation bases, terminal geometry, cell attributes and cursor modes. Blob
fixtures use raw 128 KiB SHA-256 chunks matching the production chunk size.

The adapters share one SQLite schema, one 256 KiB sealed-payload commit target,
one deterministic round-robin producer schedule, the same pagination ranges
and the same observer counts. A batch is published atomically with its source
heads and idempotent receipt in a SQLite WAL/FULL transaction. File placement
concatenates the already encoded blocks of one batch into one immutable
content-addressed `.blocks` file. SQLite stores each block's offset, length,
digest and native range. It never creates a file per producer packet.

File creation writes every payload byte, calls `sync_all` on the file, then
calls `sync_all` on the containing directory, **before** beginning the SQLite
transaction that publishes references. An unreferenced prepared file can be
reused only after checking its complete length and bytes and syncing it again.
No SQLite transaction spans file preparation, observer decoding or comparison.
The SQLite adapter publishes its identical encoded payload in that same
reference/head/receipt transaction, with `synchronous=FULL`.

Each measurement uses one writer connection, up to four reader connections,
eight bounded read permits covering query/file/decode work for both adapters,
WAL autocheckpoint at 1,000 pages, an 8 MiB soft SQLite cache target per
connection, mmap disabled, and the same bounded shared verified-body LRU
(16 MiB / 2,048 entries). A new SQLite pool and empty LRU are used for reopen
verification. Neither OS cache eviction nor a production kernel scheduler is
part of this experiment.

## Workloads and matrix

| Workload | Full payload per source | Correctness comparison |
| --- | --- | --- |
| `text-small` | 8,192 packets × 17 bytes = 139,264 bytes | Every byte and full SHA-256 |
| `text-large` | 4 packets × 34,816 bytes = the same Text body | Every byte and full SHA-256 |
| `log` | 65,536 sequential events × 17 payload bytes | Every packet, stdout/stderr order, timestamp, sequence and full canonical-event SHA-256 |
| `structured` | 1,024 events, 1 KiB text document replacements, checkpoint every 32 events | Every typed record and base plus exact replayed final bounded document |
| `terminal` | 2,048 24 × 80 frames, checkpoint every 16 events, one changed row per patch | Every typed record and base plus exact replayed final screen, cells, attributes and modes |
| `blob` | 64 chunks × 128 KiB = 8 MiB | Every binary byte, chunk digest and full SHA-256 |

`full` runs every workload with 1 / 8 sources and 1 / 8 observers per source,
both adapters, and **three independent child processes per configuration**:
144 measurements. The runner executes them serially and alternates adapter
order between repeats. The maximum generated blob payload in one child is
64 MiB. Fixtures are generated incrementally; the expected event stream and
the full blob are never retained as a whole.

After each durable batch, independent observer tasks concurrently catch up
using actual indexed reads through the reader pool. Event pages contain at
most 256 events, Text pages 64 KiB, and blob pages 128 KiB. This is a controlled
batch/catch-up schedule; it measures placement and reader costs without
claiming steady simultaneous producer/reader pressure or UI latency. Production
B02 still uses its separate 32-event pagination pressure probe.

Every live observer verifies each byte/event against a source-specific
random-access fixture. Document and terminal observers apply the actual
production `EventProjection`, then compare the resulting document/screen
against the producer's separately maintained projection. On reopen, one new
observer per source replays the complete committed stream again and verifies
the same hashes/projections. Seeks include offsets immediately around page,
32 KiB and 128 KiB boundaries, the tail, and EOF. The final committed batch is
replayed after reopen; it must return the existing receipt without writing new
payload or committing a second reference transaction.

The runner additionally performs **36 process-crash recovery cases**, each
once, separate from the three resource-measurement repetitions:
6 workloads × 2 adapters × 3 protocol boundaries. An owned child commits a
baseline block and then pauses while publishing its second block:

1. `prepared`: the file and directory have been synced, references have not
   begun; SQLite placement still holds its second payload only in memory.
2. `transaction`: block references, heads and receipt have been written inside
   the transaction, but COMMIT has not run.
3. `committed`: FULL COMMIT has returned, but no completion reaches the parent.

After an explicit ready notification the runner sends SIGKILL to its own
child and launches a new recovery process. It verifies SQLite integrity,
exactly the expected pre-replay head, all committed ranges, orphan-file count,
successful retry, full event/byte equality, final document/screen projection,
and a second idempotent retry. The two pre-COMMIT file cases must reuse exactly
one orphan prepared file without rewriting payload. Only directories created
by the runner with a matching private ownership marker are accepted. No
existing database is initialized, truncated, repaired or deleted.

These are actual process kills at explicit publication boundaries. They do
not simulate device power loss, arbitrary fsync failures or all production
kernel crash scenarios. Production P2/P8 tests remain independently required.

## Run after implementation is stable

Build/check/test first in the coordinated validation window. No compilation,
tests or other resource probes should overlap a formal full run. The runner
does not build or stop any application.

```sh
cargo build --release --locked -p agena-storage-sqlite --example event_blocks_prototype
python3 crates/agena-storage-sqlite/examples/support/run_event_blocks_matrix.py \
  --root "$PWD" \
  --binary "$PWD/target/release/examples/event_blocks_prototype" \
  --mode smoke \
  --out /tmp/agena-p1-placement-smoke-NEW
python3 crates/agena-storage-sqlite/examples/support/run_event_blocks_matrix.py \
  --root "$PWD" \
  --binary "$PWD/target/release/examples/event_blocks_prototype" \
  --mode full \
  --out /tmp/agena-p1-placement-full-NEW
```

Use the actual configured target directory if it differs from `target/`.
Output directories must not already exist. Smoke runs use one process per
adapter/configuration, one source/observer, 256 Log events, 64 Document/PTY
events and four blob chunks, followed by all 36 full-codec crash boundaries.
Smoke timings are behavior checks, never formal performance evidence.

## Measurements and interpretation

Each child emits one JSON record. Raw stdout/stderr, per-child start/end/load,
binary SHA-256, source/lock fingerprints before and after, platform, git state,
filesystem volumes, and sanitized basename-only process activity are retained.
The runner aborts if any byte, record, projection, adapter-pair contract,
receipt, integrity or crash-boundary check fails. It also rejects source or
binary changes during the window. Failures keep the raw files already written.

Recorded outcomes include actual encoded payload bytes and blocks, reference
transactions, merged-file count, cumulative file payload writes, explicit file
and directory fsync calls, SQLite page/file bytes, sampled and post-commit WAL
file peaks, per-run commit/prepare/SQLite transaction latency distributions,
observer page/catch-up latency, decoded event counts, cache usage, reopen time,
and 10 ms sampled process RSS. RSS includes the fixture, projections, latency
vectors, allocator and SQLite; the 16 MiB modeled cache bound is not an RSS
limit. WAL file size is not cumulative I/O.

On macOS, `kernel_resources.rs` provides process-attributed cumulative disk
read/write deltas via `proc_pid_rusage`. Those deltas include SQLite WAL and
checkpoint writes as attributed by the OS; they are neither device-wide write
amplification nor SQLite fsync counts. Unsupported platforms report `null`.
SQLite sync-call counts remain unmeasured. The raw Darwin logical-write field
is retained as an explicitly labeled raw counter delta without assuming its
units. The main I/O window starts after schema initialization and ends after
the complete reopen/replay/close; it excludes TempDir deletion and final
inspection. Separate live and reopen deltas are available.

Each child calculates its own nearest-rank p95. `summary.json` preserves the
three independent per-run p95 values and reports their min/median/max; it never
pools latency samples across runs. The host is not assumed idle, and unrelated
applications are left running.

P1 requires a justified ordinary-block backend and explicit layering
conditions. It does not prescribe a file-backed blob tier or a numeric tier
threshold. The extra blob fixture provides a same-durability comparison for
the existing 128 KiB SQLite blob decision. After collecting the full evidence,
record the measured tradeoff together with ownership, recovery, backup/export
and maintenance complexity. Keep the current single SQLite engine unless the
equivalent experiment gives clear evidence for a separate file tier. This
prototype neither selects a threshold automatically nor enables a file tier.
