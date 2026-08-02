# Database redesign: single-source content storage with bounded event log

Status: **implemented** — Phase A (event-log slimming) and Phase B (v10 `agena_content_nodes` single-source content) are complete and verified. Phase C hardening is pending.

This document specifies a v10 schema that eliminates the three-way duplication
of tool/activity content, bounds the event log, keeps collapsed headers O(1),
and hardens DB-level correctness. It is the implementation contract for the
storage rewrite.

---

## 1. Measured baseline (real database, 20 MB)

| Store | Bytes | Share | Content |
|---|---|---|---|
| `agena_events` | 12,804,096 | 64% | 777 `message_part_checkpointed` (6.0 MB, avg 7.7 KB, max 115 KB) + 182 `tool_call_completed` (4.9 MB, avg 27 KB) |
| `agena_model_message_parts` | 5,251,072 | 26% | 271 operation parts (4.98 MB) with full `content` JSON |
| `agena_activities` | 1,966,080 | 10% | 185 operation activities (1.55 MB) incl. full `details` |
| `agena_model_messages` + others | ~300 KB | 1% | headers, catalog, scheduler |

### Duplication proof

One `apply_patch`-style tool result is stored **4+ times** today:

1. `agena_events.payload_json` — `tool_call_completed` carries the full `part`
   (with `content.blocks` text).
2. `agena_events.payload_json` — every 100 ms streaming checkpoint
   (`message_part_checkpointed`, persistent) carries the **cumulative** text
   up to that instant → O(n²) growth for long outputs.
3. `agena_model_message_parts.content` — the same full `OperationPart` JSON.
4. `agena_activities.payload_json` — simplified `OperationActivity` that still
   embeds `details: ToolOutput` (full diff).

Plus in-memory field-level duplication inside `OperationPart::completed`:
`structured` == `result.structured` == `details.to_json_payload()`; and
`model_output.text` == `result.model_preview.text`; and `title`/`summary`
exist both top-level and in `result.display`.

---

## 2. Design goals

- **Single source of truth** for every content node (text segment or activity).
- **Event log records facts, not content**: durable events must not embed full
  payloads that already live in the projection tables.
- **Collapsed list stays O(1)**: header columns (`name`, `summary`, `has_detail`)
  are row columns, never derived by parsing JSON at query time.
- **DB-enforced correctness**: every invariant lives in CHECK constraints /
  triggers; the app layer cannot corrupt state.
- **Bounded write amplification**: streaming output checkpoints are ephemeral
  (bus-only) or sparse (e.g. 2 s cadence) instead of 100 ms persistent full
  snapshots.
- **Good complexity**: batch reads (one query per owner set), covering indexes,
  WAL, no N+1.

---

## 3. New schema (v10)

### 3.1 `agena_content_nodes` — the single content table

Replaces `agena_activities` + `agena_text_segments` and becomes the content
source that `agena_model_message_parts.content` currently duplicates.

```sql
CREATE TABLE agena_content_nodes (
  node_id        TEXT PRIMARY KEY,        -- ActivityId or TextSegmentId UUID
  owner_kind     TEXT NOT NULL,           -- 'turn_input' | 'assistant_reply' | 'session' | 'activity'
  owner_id       TEXT NOT NULL,           -- turn/reply/session/parent-activity id
  node_type      TEXT NOT NULL CHECK (node_type IN ('text','activity')),
  actor          TEXT,                    -- activity actor (NULL for text)
  state          TEXT NOT NULL,           -- pending/in_progress/completed/failed/cancelled
  position       INTEGER NOT NULL,        -- owner-relative order (shared namespace)
  revision_seq   INTEGER NOT NULL,        -- monotonic per node
  started_at_ms  INTEGER NOT NULL,
  finished_at_ms INTEGER,
  name           TEXT,                    -- O(1) collapsed title
  summary        TEXT,                    -- O(1) collapsed summary
  has_detail     INTEGER NOT NULL DEFAULT 0,
  payload_json   TEXT NOT NULL,           -- canonical typed payload (see 3.2)
  created_at_ms  INTEGER NOT NULL,
  updated_at_ms  INTEGER NOT NULL,
  UNIQUE (owner_kind, owner_id, position),
  CHECK (state IN ('pending','in_progress','completed','failed','cancelled')),
  CHECK ((node_type = 'text' AND actor IS NULL AND payload_json IS NOT NULL)
      OR (node_type = 'activity' AND actor IS NOT NULL))
);
CREATE INDEX idx_content_nodes_owner ON agena_content_nodes (owner_kind, owner_id, position);
```

### 3.2 Canonical payload type

One payload format must win. Recommendation: **keep the runtime `PartContent`
(`RuntimeActivity`) as canonical** because it is the richest (blocks,
attachments, metadata, result envelope, lifecycle). The domain
`ActivityPayload` becomes a **lossless projection** computed on read (see 3.6)
and is **not stored**. This removes the second type system from the DB.

Migration note: `activity_payload()` currently drops fields
(`blocks/attachments/metadata/structured/result/raw/lifecycle`). The projection
must be upgraded to a lossless conversion so the TUI expanded view and the API
`OperationPartResource` render identical data.

### 3.3 `agena_message_parts` — thin mapping, no content

```sql
CREATE TABLE agena_message_parts (
  part_id          INTEGER PRIMARY KEY,   -- keep stable numeric ids for API
  message_id       INTEGER NOT NULL REFERENCES agena_model_messages(message_id) ON DELETE CASCADE,
  part_index       INTEGER NOT NULL,
  status           INTEGER NOT NULL,      -- part-level execution status (kept; header O(1))
  awaits_user_reply INTEGER NOT NULL DEFAULT 0,
  node_id          TEXT NOT NULL UNIQUE REFERENCES agena_content_nodes(node_id) ON DELETE CASCADE,
  created_at_ms    INTEGER NOT NULL,
  UNIQUE (message_id, part_index)
);
CREATE INDEX idx_message_parts_message ON agena_message_parts (message_id, part_index);
```

Header columns (`name`, `summary`, `has_detail`, `kind`, `activity_id`,
`segment_id`, `operation_id`) **move to `agena_content_nodes`**; `operation_id`
becomes `agena_content_nodes` metadata or a dedicated column on content nodes
for O(1) tool-call correlation.

### 3.4 `agena_events` — bounded facts

- `message_part_checkpointed` becomes **non-persistent** (bus-only), exactly
  like `transcript_part_upserted` / `command_output_delta`. Streaming UI gets
  the same `TranscriptPatch` it already receives; restart recovery relies on
  the **sparse durable checkpoint** below.
- Add a new persistent kind `content_checkpointed` (or reuse
  `message_part_checkpointed` but only emit it on a **2 s cadence or state
  change**), carrying `(node_id, revision_seq, state, name, summary,
  has_detail)` **without `content`**. The projection tables are the content
  authority; the event only drives projection convergence and restart
  recovery of headers.
- `tool_call_completed` keeps only `(call_id, message_id, completion state,
  revision_seq, header fields)` — no embedded `part`/blocks. The full result
  is read from `agena_content_nodes`/`agena_message_parts`.
- `tool_call_issued` unchanged (small).
- `assistant_message_finished` / `user_message_appended` still carry their
  part list for fresh-session replay; consider replacing the embedded full
  parts with node references once fork/import replay is rewritten to
  re-materialize from `agena_content_nodes` instead of the event stream.

Impact estimate on the measured DB: `agena_events` 12.8 MB → ~0.6 MB
(6 MB checkpoint series → ~0; 4.9 MB tool_call_completed → ~0.2 MB).

### 3.5 `agena_model_messages` — unchanged shape

Keep as-is (header + metadata + usage + provider_state). It already holds no
content.

### 3.6 Read paths (no N+1, O(1) headers)

- **API collapsed list**: `SELECT ... FROM agena_model_messages m JOIN
  agena_message_parts p ON p.message_id = m.message_id JOIN
  agena_content_nodes n ON n.node_id = p.node_id` selecting only header
  columns — no JSON parse, index-only scan.
- **Transcript snapshot**: one query for turns/replies + one query for
  `agena_content_nodes` by `(owner_kind, owner_id)` batches (same pattern as
  `transcript_documents_batch` today, but against one table).
- **Expanded detail**: `WHERE node_id = ?` single-row payload read.
- **TUI live patch**: unchanged `TranscriptPatch` / `ContentUpserted`; the
  patch carries the typed `ContentNode`, which is exactly what the store
  persists into `agena_content_nodes`.

### 3.7 Correctness hardening (triggers/constraints)

- Content-node triggers: owner existence + position namespace shared with
  siblings (port from v9 `agena_activities`/`agena_text_segments` triggers),
  revision monotonicity, lifecycle transitions, identity immutability.
- `agena_message_parts.node_id` UNIQUE prevents double-attach; FK to message
  and content node; part_index uniqueness per message.
- `agena_events` remains append-only; add CHECK that payload size is bounded
  for content-bearing kinds (defense in depth).
- Keep all session/turn/reply/execution triggers from v9 unchanged.

---

## 4. Implementation plan (phases)

### Phase A — stop the bleed (no schema change, reversible)
1. `OperationPart::completed`/`failed`/`non_execution`: drop duplicated
   `structured`/`result.structured`/`model_output`/`result.model_preview`
   copies; keep one authoritative representation and derive the rest on read.
2. Make `message_part_checkpointed` non-persistent; introduce a 2 s durable
   header-only checkpoint event. Streaming continues via bus-only
   `transcript_part_upserted`.
3. `list_parts(include_content=false)`: use a column list without `content`.
4. `bounded_presentation_summary`: early-exit via `char_indices().nth()`.

### Phase B — v10 schema + projection rewrite
1. New `schema.rs` tables + indexes; bump `CURRENT_SCHEMA_VERSION` to 10;
   fresh DBs get v10; old DBs are rejected (project convention).
2. Rewrite `project_part_content`/`upsert_part_projection` to write
   `agena_content_nodes` + `agena_message_parts` only.
3. Rewrite `transcript_snapshot` + `transcript_documents_batch` to read
   `agena_content_nodes`.
4. Update repository contracts (`agena-storage`), sqlite repository, and all
   call sites (session store, manager, TUI snapshot, API projections).
5. Lossless `activity_payload` projection from canonical `PartContent`.
6. Fork/import/export replay rewritten to re-materialize from content nodes
   instead of replaying full-payload events.
7. Full test pass: schema invariants, projection tests, TUI snapshot tests,
   API contract tests; add a DB-level duplication regression test that
   asserts one content row per node.

### Phase C — harden
1. WAL journal mode + busy_timeout + synchronous=NORMAL at connection setup.
2. Covering indexes for all hot queries (verify with `EXPLAIN QUERY PLAN`).
3. Optional: content compression for large payload columns (`zlib` on
   `payload_json` when > 4 KB) with a `compressed` flag, keeping JSON
   readable in dev DBs.
4. Optional: event retention policy (drop non-fact events older than N).

---

## 5. Risks / decisions to confirm

- **Fork/rewind/import depend on persistent events.** Phase B must land the
  replay rewrite in the same commit as the event-shrink change, otherwise
  forked sessions lose content.
- **`activity_payload()` losslessness** is the trickiest part: TUI and API
  must render identical expanded views after the switch.
- **Wire compatibility**: removing `structured`/`model_output` from
  `OperationPartResource` is a client-visible change; the TUI and Studio
  consumers must be updated in lockstep.
- Schema version policy stays "fresh DB only" for v10 (no in-place migration
  from v9), matching the documented convention.

---

## 6. What stays unchanged

- `agena_sessions` / `agena_session_lineage` / `agena_permission_rules` /
  `agena_model_catalog_*` / `agena_scheduler_*` / `agena_workspaces`
  (no content duplication there).
- All session/turn/reply/execution invariant triggers.
- The bus/event architecture, `TranscriptPatch`, revision-based merging.

---

## 9. Phase A implementation record (completed)

All Phase A items from section 5 are implemented and verified on `master`
(before the v10 schema work). Each change is wire-compatible: persisted events
and stored JSON remain deserializable, and only redundant data was removed.

### A1 — `OperationPart` field deduplication

Removed four redundant top-level fields from `OperationPart`:

- `model_output` — duplicate of `result.model_preview`
- `blocks` — duplicate of `result.content`
- `attachments` — duplicate of `result.attachments`
- `structured` — duplicate of `result.structured` (itself `details.to_json_payload()`)

`OperationPart` has no `deny_unknown_fields`, so previously persisted rows that
still contain these fields deserialize cleanly. Consumers were repointed:

- `project_operation_part` (session/manager/history.rs) reads the result envelope
- `activity_payload` (session/history/store/mod.rs) reads `result.model_preview`
- `output_text()`, `status()`, `append_output_delta()` single-write to `result`

Effect: each stored `OperationPart` JSON loses the duplicated text/blocks copy;
both `agena_model_message_parts.content` and `agena_activities.payload_json`
shrink, and events that embed parts shrink too.

### A2 — event-log slimming

1. **`tool_call_completed` no longer embeds the part.**
   `ToolCallCompleted.part: MessagePart` was removed; the event now carries only
   `message_id/call_id/run_id/tool_name/completed_at`. The terminal Operation
   content is projected by the durable `MessagePartCheckpointed` emitted by
   `apply_tool_success*` immediately before the completed event is appended
   (same transaction order, replay-safe). `update_tool_result_projection` now
   re-validates the operation binding and advances the message projection
   timestamp instead of re-writing content. Old rows with `part` still parse.

2. **Streaming tool checkpoints persist at 2 s, not 100 ms.**
   `apply_streaming_tool_execution` batches deltas in memory and calls
   `append_streaming_tool_output_delta` every `DELTA_BATCH_MS` (now 2 000 ms,
   aligned with `TITLE_REFRESH_MS`); the terminal flush is unchanged. A 60 s
   long command now writes ~30 cumulative checkpoints instead of ~600.
   Live UI updates continue via the existing bus broadcast on each persisted
   batch (2 s granularity, same as the live title refresh).

3. **Fork/import fix (found while auditing).** `rewrite_event_part_ids` did not
   rewrite `MessagePartCheckpointed.part.id` while `visit_event_part_ids` did;
   forked/imported sessions could project checkpoints with stale part ids.
   The rewrite now mirrors the visitor.

### Verification

- `cargo check --workspace`: clean, zero warnings.
- `cargo test -p agena-runtime-session --lib`: 97 passed.
- `cargo test --workspace`: all affected suites pass. Two pre-existing failures
  are unrelated to this work and reproduce on the unmodified baseline:
  1. `approved_provider_tool_executes_once_then_continues_the_same_turn`
     overflows the tokio worker stack (`RUST_MIN_STACK=16777216` works around it).
  2. `capability_identity_snapshot_matches_committed_json` — committed snapshot
     drifted from current tool schemas (needs `agena inspect --json
     --identity-snapshot` regeneration).

### Remaining for Phase B (v10 schema)

- `agena_content_nodes` single content table replacing `agena_activities` +
  `agena_text_segments` + `agena_model_message_parts.content`.
- Schema v10 migration + invariant triggers + payload size caps.
- Projection/read-path rewrite (`project_part_content`, `transcript_snapshot`,
  `transcript_documents_batch`, repository layer).
- `activity_payload()` lossless fidelity (embed full `OperationPart` once).
- Hardening: WAL, covering indexes, retention policy.

See sections 2-7 for the full v10 contract.

---

## 10. Phase B implementation record (completed)

Phase B landed the v10 schema and switched the canonical content read/write
paths to `agena_content_nodes`.

### Schema v10

- `CURRENT_SCHEMA_VERSION` bumped 9 -> 10; `MIGRATABLE_VERSIONS = [8, 9]`.
- New table `agena_content_nodes` (node_id PK, owner_kind/owner_id, node_type
  text|activity, actor, payload_json, text, state, position, revision_seq,
  started/finished/created/updated timestamps, UNIQUE(owner_kind, owner_id,
  position)) plus `idx_content_nodes_owner`.
- Migration 9 -> 10 backfills `agena_content_nodes` from `agena_activities`
  (activity nodes) and `agena_text_segments` (text nodes, state=completed).
  The 8 -> 10 path runs the same backfill after the v9 ALTER.
- Invariant triggers: owner existence, lifecycle (finished rules apply to
  activity nodes only; text nodes may be completed without finish), identity
  immutability, revision monotonicity, and turn/reply/activity/session
  cascade deletes. The owner trigger intentionally does not check legacy
  tables for position conflicts because the migration-era dual write
  necessarily shares positions.

### Projection and read paths

- `project_part_content` now mirrors every text segment and activity into
  `agena_content_nodes` (dual write; legacy tables stay populated for
  rollback compatibility). Compaction maintenance activities mirror too.
- `next_content_position` includes `agena_content_nodes` in its union.
- `reply_has_pending_interaction`, `cancel_reply_interactions`, and
  `terminalize_reply_operations` read/update `agena_content_nodes`.
- `transcript_documents_batch` reads a single `agena_content_nodes` query
  (was two queries across `agena_text_segments` + `agena_activities`).
- Session activities (`owner_kind='session'`) read from `agena_content_nodes`.
- Projection cascade clear deletes `agena_content_nodes` rows.

### Verification

- `cargo check --workspace`: clean.
- `agena-storage-sqlite`: 26 tests pass (incl. new content-node invariant test).
- `agena-runtime-session`: 97 tests pass.
- Schema lifecycle tests cover 0/current/migratable(9)/incompatible paths.

### Remaining (Phase C / later)

- Drop legacy `agena_activities` / `agena_text_segments` tables once the
  dual-write window closes (v11), then collapse `parts.content` into
  `agena_content_nodes.payload_json` using the lossless `PartContent` format.
- WAL + busy_timeout + synchronous=NORMAL at connection setup.
- Covering indexes verified with `EXPLAIN QUERY PLAN`.
