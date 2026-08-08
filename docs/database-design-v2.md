# Agena Database Design v2 — Everything is a Part (membership-first)

Status: design draft for review
Branch: research/db-design-audit (worktree .agena/worktrees/db-design-audit)

This document defines a from-scratch SQLite schema for Agena chat data. It replaces
`docs/database-design-audit.md` (the audit of the current v1 schema).

---

## 1. Motivation

The current (v1) schema is complex because content and container are entangled and
ownership is single-session:

- Five parallel content layers: `agena_model_messages`, `agena_model_message_parts`,
  `agena_content_nodes`, `agena_turns` / `agena_assistant_replies` /
  `agena_reply_executions`, plus the event log `agena_events` that is the real source
  of truth with derived projections (`agena_model_projection_states` watermark).
- Fork/rewind is a hybrid: view definition + membership edges for the completed
  prefix, plus a physical copy (with id remap) of the in-flight tail
  (`agena-runtime-session/src/session/store/fork.rs`, `ids.rs`).
- Activity content (tool activity body) is keyed by a single session/turn/reply and
  does NOT follow fork sharing (`agena_content_nodes.owner_kind/owner_id`;
  `transcript_snapshot` reads only the current session).

v2 collapses everything into two core tables:

> **A part is the only chat-content entity, global and owned by no session.**
> **A session is an ordered membership view over parts.**

Fork/rewind become pure membership operations. There is no event log, no projection,
no watermark, no turn/reply/execution table.

---

## 2. Design principles

1. **Parts are the only chat-content entity.** Text, tool calls, results, think,
   file/skill/paste references, notices, hooks, compaction, errors, interactions
   (ask user / plan review / permission) are all parts with different `kind`.
2. **Ownership = membership edges.** A part belongs to every session that has a
   `session_parts` row, and to no session otherwise. `origin_session_id` on a part
   is provenance only (who created it), never ownership.
3. **A session is a view.** Identity/metadata + ordered membership. A message is a
   batch of parts sharing a run; a turn is a run marker plus its parts; a reply is
   the parts of one run — all emergent concepts, no tables.
4. **Fork/rewind = copy membership edges.** Zero content copy, zero id remap, zero
   projection rebuild. The run markers are parts too, so the turn structure is
   shared automatically.
5. **No events, no projections.** Parts are the truth. Runs are demoted to a system
   part (`kind = run`), so execution bookkeeping is just data. Crash recovery
   inspects in-flight run markers, never replays events.
6. **Streaming = in-place part updates** guarded by a monotonic `revision`
   (optimistic concurrency, `ON CONFLICT ... WHERE excluded.revision >= parts.revision`).
7. **Deletion is refcount-guarded.** A part is deleted only when no session
   references it; orphans are reclaimed by a background GC.
8. **Multi-process safety** reuses the proven v1 machinery: WAL,
   `busy_timeout`, `.schema-lock` file, write-lock sentinel, busy retry with
   backoff, global sequences.

---

## 3. Table inventory

Nine tables. Core chat data is two tables (`parts`, `session_parts`); the rest are
session metadata, execution infrastructure, policy, or metrics.

| # | Table | Category |
|---|-------|----------|
| 1 | `parts` | Core: all chat content |
| 2 | `session_parts` | Core: multi-session ownership |
| 3 | `sessions` | Session identity + lineage + config |
| 4 | `execution_leases` | Cross-process single-writer |
| 5 | `sequences` | ID allocation |
| 6 | `workspaces` | Workspace identity |
| 7 | `permission_rules` | Policy |
| 8 | `usage` | Metrics |
| 9 | `idempotency` | Dedup |

Unchanged infrastructure outside the chat-data model (same DB file, not redesigned
here): `model_catalog_entries`, `model_catalog_state`, `scheduler_jobs`,
`scheduler_history`.

---

## 4. Core tables

### 4.1 `parts` — the only chat content table

```sql
CREATE TABLE parts (
    part_id           INTEGER PRIMARY KEY,        -- global sequence (sequences: part_id)
    kind              TEXT    NOT NULL,           -- open set, see 4.1.1
    role              TEXT    NOT NULL
                      CHECK (role IN ('user','assistant','system','tool','runtime')),
    state             TEXT    NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending','in_progress','completed','failed','cancelled')),
        content           JSON    NOT NULL,           -- typed payload per kind (raw data the AI sees)
    summary           TEXT,                       -- compact label (O(1) list updates)
    visibility        TEXT    NOT NULL DEFAULT 'both'
                      CHECK (visibility IN ('both','user','ai')),  -- who sees this part (18.3)
    rendered_markdown TEXT,                       -- human-friendly markdown rendered by the producing plugin/tool (18.4)
    parent_part_id    INTEGER REFERENCES parts(part_id),  -- tool_result→tool_call; reply→interaction; child→parent activity
    run_id            INTEGER REFERENCES parts(part_id),  -- the run marker part of this batch; NULL on the marker itself
    origin_session_id INTEGER,                    -- provenance only, informational
    revision          INTEGER NOT NULL DEFAULT 1, -- optimistic concurrency for streaming
    started_at_ms     INTEGER NOT NULL,
    finished_at_ms    INTEGER,
    created_at_ms     INTEGER NOT NULL,
    updated_at_ms     INTEGER NOT NULL,
    CHECK (finished_at_ms IS NULL OR finished_at_ms >= started_at_ms),
    CHECK ((state IN ('pending','in_progress') AND finished_at_ms IS NULL)
        OR (state IN ('completed','failed','cancelled') AND finished_at_ms IS NOT NULL))
);

CREATE INDEX idx_parts_parent ON parts(parent_part_id);
CREATE INDEX idx_parts_run    ON parts(run_id);
CREATE INDEX idx_parts_origin ON parts(origin_session_id, created_at_ms);
CREATE INDEX idx_parts_recover ON parts(kind, state);        -- crash recovery scan
CREATE INDEX idx_parts_origin_state ON parts(origin_session_id, state); -- recovery / GC
```

`kind` is deliberately an open set (plugins may add kinds). `role` and `state` are
closed enums enforced by CHECK.

#### 4.1.1 Part kinds and content shapes

| kind | role | content JSON | notes |
|------|------|--------------|-------|
| `run` | user / runtime | `{"run_kind":"user_send|continue|background","abort_reason":...}` | turn/run marker; state mirrors reply status |
| `text` | user / assistant / system | `{"text":"..."}` | plain text |
| `think` | assistant | `{"summary":[...],"raw":[...]}` | reasoning |
| `tool_call` | assistant | `{"name":"...","plugin":"...","input":{...}}` | tool invocation |
| `tool_result` | tool | `{"output":"...","ok":true}` | child of tool_call via parent_part_id |
| `file_ref` | user | `{"path":"...","name":"...","mime":"...","sha":"..."}` | file reference only; no blob stored, AI reads on demand |
| `paste_ref` | user | `{"text":"..."}` | pasted text stored INLINE (full content; no blob cache) |
| `skill_ref` | user | `{"skill":"...","args":{...}}` | skill name/args reference only |
| `notice` | runtime | `{"kind":"...","summary":"...","detail":"..."}` | system notice (hook runs etc.) |
| `hook` | runtime | `{"hook":"...","summary":"...","detail":"..."}` | hook activity |
| `compaction` | runtime | `{"summary":"...","window":[...]}` | compaction summary |
| `error` | runtime | `{"category":"...","message":"...","detail":...}` | failure |
| `interaction` | system | `{"type":"ask_user|plan_review|permission","prompt":"...","options":[...],"response":...}` | user processing point |

A `run` marker is created for every execution batch (user send / continue /
background). It carries the batch state and abort reason; the reply/status the UI
needs is the marker state. `parts.run_id` references the marker part so
「all parts of run X」 is a flat indexed query (no recursion).

#### 4.1.2 User message shape — one text with ref placeholders + N ref parts

A user message is NOT one blob: it is **1 `text` part + N reference parts**
(`file_ref` / `skill_ref` / `paste_ref`), all under the run marker:

```
run marker (role=user)
  ├─ text part:      "please review [[ref:102]] and [[ref:103]]"
  ├─ file_ref part (id=102): {path, name, mime, sha}
  ├─ skill_ref part (id=103): {skill, args}
  └─ paste_ref part (id=104): {text: "...full pasted content..."}   (optional)
```

- The `text` part carries **placeholder markers** (`[[ref:<part_id>]]`) so the AI
  knows exactly which reference corresponds to which slot in the sentence. The
  placeholder embeds the ref part's `part_id` — global, unique, stable across
  forks (shared refs keep their ids).
- Reference payloads live in their own parts (paths, skill names, full pasted
  text); the text part stays small and readable.
- Prompt assembly (execution engine) resolves placeholders when building the
  model request: file refs → the file path/contents, skill refs → the skill
  descriptor, paste refs → the pasted text (full text is stored inline, never a
  blob reference).
- Every part carries `created_at_ms` (ordering basis) and a unique `part_id`
  (tie-breaker); ordering is always `ORDER BY created_at_ms, part_id` (4.2).

### 4.2 `session_parts` — the only ownership mechanism

```sql
CREATE TABLE session_parts (
    session_id  INTEGER NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    part_id     INTEGER NOT NULL REFERENCES parts(part_id),
    added_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, part_id)
);

CREATE INDEX idx_session_parts_part ON session_parts(part_id);  -- reverse: which sessions share a part
```

- No per-session ordering column: order is derived from the part itself
  (`ORDER BY parts.created_at_ms, parts.part_id`), so every part needs only its
  own timestamp + unique id (see 4.1.2). Appends need no MAX+1 bookkeeping.
- Deleting a session cascades only the edges, never the parts (other sessions may
  still reference them).
- This is where 「one part belongs to many sessions」 lives: the same `parts` row is
  referenced by N `session_parts` rows.

---

## 5. Sessions and supporting tables

### 5.1 `sessions` — session identity, lineage, config

```sql
CREATE TABLE sessions (
    session_id            INTEGER PRIMARY KEY,
    workspace_id          INTEGER NOT NULL REFERENCES workspaces(workspace_id) ON DELETE CASCADE,
    parent_id             INTEGER REFERENCES sessions(session_id),  -- fork/rewind/subagent lineage
    relation_kind         TEXT NOT NULL DEFAULT 'root'
                          CHECK (relation_kind IN ('root','fork','rewind','subagent')),
    cutoff_part_id        INTEGER,                -- fork/rewind cutoff (provenance / display)
    title                 TEXT NOT NULL,
    version               INTEGER NOT NULL,       -- optimistic lock; cross-process cache invalidation
    lifecycle_state       TEXT NOT NULL DEFAULT 'creating'
                          CHECK (lifecycle_state IN ('creating','ready','failed')),
    creation_failure_json JSON,
    task_id               TEXT,                   -- subagent task
    subtask_status        TEXT,
    subtask_started_at_ms INTEGER,
    subtask_finished_at_ms INTEGER,
    subtask_failure_json  JSON,
            config_json           JSON,                   -- session CONFIG, not state: permission ceiling,
                                                  -- capability denials, workspace root override,
                                                  -- execution selection/access defaults; state is
                                                  -- derived from parts (17.3); compaction policy
                                                  -- comes from global settings (D5), not stored
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL
);

CREATE INDEX idx_sessions_workspace ON sessions(workspace_id, updated_at_ms);
```

v1 lineage table is folded in. The v1 `runtime_state_json` snapshot shrinks to
`config_json` because workflow state is derived from parts:

- Blocked = exists an `interaction` part with `state = 'pending'`;
- ToolPending = exists an `in_progress` `tool_call` part;
- Quiescent = otherwise.

### 5.2 `execution_leases` — cross-process single writer

```sql
CREATE TABLE execution_leases (
    session_id          INTEGER PRIMARY KEY REFERENCES sessions(session_id) ON DELETE CASCADE,
    owner_id            TEXT NOT NULL,            -- per-process id
    run_id              INTEGER,                  -- the in-flight run marker part_id
    lease_started_at_ms INTEGER NOT NULL,
    heartbeat_at_ms     INTEGER NOT NULL          -- refreshed every 5s; stale after 15s
);
```

Semantics (identical to v1 with one addition):

- Acquire: `INSERT ... ON CONFLICT(session_id) DO UPDATE SET ... WHERE heartbeat < stale`
  (only a stale lease may be stolen).
- **Steal-with-reconcile**: acquiring a stale lease aborts that session's stale
  in-flight run markers in the same transaction (see 7.2).
- Release on run end; reap stale leases on startup and in a periodic maintenance loop.

### 5.3 `sequences` — ID allocation

```sql
CREATE TABLE sequences (
    seq_name TEXT PRIMARY KEY,
    next_val INTEGER NOT NULL
);
-- seeds: part_id = 1; reserved row __agena_write_lock__ = 1 (write-lock sentinel)
```

`part_id` is allocated atomically (`INSERT ... ON CONFLICT DO UPDATE SET
next_val = next_val + 1 RETURNING next_val - 1`). `run_id` is a `part_id` (the
marker), `session_id`/`workspace_id` use AUTOINCREMENT; no other sequences needed.

### 5.4 `workspaces` — unchanged

```sql
CREATE TABLE workspaces (
    workspace_id  INTEGER PRIMARY KEY AUTOINCREMENT,
    path          TEXT NOT NULL UNIQUE,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
```

### 5.5 `permission_rules` — policy, unchanged shape

```sql
CREATE TABLE permission_rules (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    action_key      TEXT NOT NULL,
    mode            TEXT NOT NULL,
    scope           TEXT NOT NULL,
    session_id      INTEGER,
    workspace_id    INTEGER,
    source          TEXT NOT NULL,
    reason          TEXT,
    operator        TEXT,
    revoked_at_ms   INTEGER,
    revoked_reason  TEXT,
    revoked_by      TEXT,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL
);
-- plus the same partial unique index per subject scope as v1
```

### 5.6 `usage` — append-only, normalized metrics (low footprint, high performance)

One row per provider response (model call). Scalar integer columns are directly
SUM-able in SQL; optional `detail_json` keeps rare provider-specific fields out
of the hot path. Cost is stored as integer micro-USD (no float drift, compact
varint encoding). Immutable: rows are inserted once at model-call completion and
never updated, so aggregation queries never contend with writers.

```sql
CREATE TABLE usage (
    usage_id               INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id           INTEGER NOT NULL,          -- denormalized: workspace aggregates need no join
    session_id             INTEGER NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    run_id                 INTEGER,                   -- run marker part_id (a turn may contain many model calls)
    provider_id            TEXT NOT NULL,
    model_id               TEXT NOT NULL,
    created_at_ms          INTEGER NOT NULL,          -- model-call completion time
    input_tokens           INTEGER NOT NULL,
    output_tokens          INTEGER NOT NULL,
    reasoning_tokens       INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens      INTEGER NOT NULL DEFAULT 0,
    tool_use_tokens        INTEGER NOT NULL DEFAULT 0,
    other_tokens           INTEGER NOT NULL DEFAULT 0,
    total_cost_micros      INTEGER NOT NULL DEFAULT 0,    -- computed/estimated cost, micro-USD
    recorded_cost_micros   INTEGER,                       -- provider-reported cost when available
    cost_estimate_incomplete INTEGER NOT NULL DEFAULT 0,  -- surfaced as "cost may be incomplete"
    detail_json            JSON                           -- optional full provider breakdown (5m/1h cache, per-category, provenance)
);

CREATE INDEX idx_usage_ws_time ON usage(workspace_id, created_at_ms);
CREATE INDEX idx_usage_session ON usage(session_id, created_at_ms);
CREATE INDEX idx_usage_provider_model ON usage(provider_id, model_id, created_at_ms);
```

Written once per model call through the facade (internal to the persistence
engine). Replaces v1 derivation from message metadata. See section 16 for the
full low-storage/high-performance design.

### 5.7 `idempotency` — user-send dedup

```sql
CREATE TABLE idempotency (
    session_id      INTEGER NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    run_id          INTEGER NOT NULL,             -- run marker part_id
    created_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (session_id, idempotency_key)
);
```

---

## 6. Invariants (triggers)

1. **parts lifecycle**: legal state transitions (pending → in_progress →
   completed/failed/cancelled, plus retry `failed → in_progress` with `revision++`
   and `finished_at` cleared), `revision` monotonic, timestamps self-consistent
   (CHECKs above); `visibility` CHECK enforced in DDL.
2. **parts identity immutable**: `part_id, role, kind, parent_part_id,
   origin_session_id, created_at_ms` may not be updated.
3. **session_parts references**: part and session must exist.
4. **sessions relations**: `fork`/`rewind` require `parent_id` and `cutoff_part_id`;
   lifecycle transitions legal; `version` monotonic.
5. **runs (marker parts)**: a `run` part must be the root of its batch
   (`run_id IS NULL`); `abort_reason` required when terminal state is failed/cancelled.
6. **Cascades**: session delete → membership edges cascade; part deletion is handled
   by GC bottom-up (children first).

---

## 7. Key operations

### 7.1 User send (one transaction)

1. Allocate a `part_id` for the run marker; create marker part
   (`kind='run'`, `role='user'`, `state='pending'`).
2. Create user content parts (`text`, `file_ref`, ...) with `run_id = marker`.
3. Insert membership rows: marker + content parts (order derives from
   `created_at_ms`/`part_id`; no sequence bookkeeping).
4. Insert `idempotency` row (if keyed).
5. Acquire the session lease first (whole batch is the executing run).

### 7.2 Crash recovery / lease steal (one transaction)

```sql
UPDATE execution_leases SET owner_id=?, run_id=?, ... WHERE session_id=? AND heartbeat stale;
UPDATE parts SET state='failed', content=json_set(content,'$.abort_reason','lease_stolen')
 WHERE origin_session_id=? AND kind='run' AND state IN ('pending','in_progress');
UPDATE parts SET state='cancelled', ...
 WHERE origin_session_id=? AND run_id IN (aborted markers) AND state IN ('pending','in_progress');
```

A startup maintenance loop (any process, idempotent) also reaps stale leases and
GCs orphans even for sessions nobody reopens.

### 7.3 Fork / rewind (one transaction, eager edge copy)

```sql
INSERT INTO sessions (session_id, workspace_id, parent_id, relation_kind, cutoff_part_id, title, ...)
VALUES (...);
-- copy membership edges whose part sorts at or before the cutoff part
-- (cutoff = (created_at_ms, part_id) of the cutoff part)
INSERT INTO session_parts (session_id, part_id, added_at_ms)
SELECT :child_id, sp.part_id, :now FROM session_parts sp
JOIN parts p ON p.part_id = sp.part_id
WHERE sp.session_id = :parent
  AND (p.created_at_ms < :cutoff_created_at
       OR (p.created_at_ms = :cutoff_created_at AND p.part_id <= :cutoff_part_id));
-- rewind: strictly before the cutoff part; relation_kind='rewind'
```

Cost O(shared edges); zero content copy, zero id remap. Forking a session while the
parent is streaming is safe under the shared-part rule (4.3 below).

### 7.4 Read a session transcript (one query)

```sql
SELECT p.* FROM session_parts sp JOIN parts p ON p.part_id = sp.part_id
WHERE sp.session_id = :session
ORDER BY p.created_at_ms, p.part_id;   -- time is the ordering basis; id breaks ties
```

UI groups by run markers; model prompt assembly groups by run and splits by role.

### 7.5 Interaction (ask user / plan review)

1. Create an `interaction` part (`state='pending'`) in the session.
2. The user answers by **appending a new reply part** (`parent_part_id` → the
   interaction, `role='user'`) and marking the interaction `completed` — or, when
   the interaction is single-session owned, by editing `content.response` in place.
3. A new `continue` run proceeds from the reply part.

### 7.6 Delete session / GC

1. Delete the session row (membership edges cascade).
2. GC: `DELETE FROM parts WHERE part_id NOT IN (SELECT part_id FROM session_parts)
   AND (run_id IS NULL OR run is terminal)`, children before parents, in the
   maintenance loop.

Parts are always created with their membership in the same transaction, so a crash
rollback never leaves half-created orphans.

---

## 8. Concurrency model (multi-instance)

Agena runs as multiple processes (TUI, server, CLI) sharing one SQLite file, and
may run different sessions concurrently or attempt to run the same session.

### 8.1 Baseline (reused from v1)

- WAL, `busy_timeout=15s`, `synchronous=NORMAL`, `foreign_keys=ON`, bounded pool.
- Schema creation serialized by a `<db>.schema-lock` advisory file lock.
- Every write transaction starts with the write-lock sentinel
  (`INSERT OR IGNORE INTO sequences VALUES ('__agena_write_lock__', 1)`) so the busy
  timeout applies at lock acquisition.
- Busy detection (SQLITE_BUSY codes 5/31) + up to 5 retries with exponential backoff.

### 8.2 Different sessions, possibly different instances

- Leases are per-session; different sessions never contend on the lease table.
- Writes touch disjoint part/membership rows; SQLite serializes the physical writes,
  handled by the busy machinery. Reads are concurrent under WAL.
- `part_id` allocation is atomic across processes.

### 8.3 Same session, multiple instances — prevented by the lease

- Only the lease owner may start a run for a session; a second instance gets
  `HeldBy` and refuses.
- A crashed owner's lease becomes stale after 15s and may be stolen; stealing
  atomically aborts the stale in-flight run markers (7.2), so a session never has
  two live run markers.
- In-process, the execution registry additionally prevents double execution within
  one instance.

### 8.4 Fork concurrent with parent streaming — the shared-part rule

Shared (multi-session) parts are **read-only and append-reference-only for every
session except the one that created them**:

> A session may update in place only parts it created (`origin_session_id = this
> session`, enforced in the storage update functions which also require the lease).
> To diverge, a session appends new parts; it never mutates a shared part.

Consequences:

- The child of a fork never writes the parent's rows; parent and child executions
  are row-disjoint and safe concurrently.
- An interaction part shared with a fork is answered by appending a reply part, not
  by in-place edits (which would leak into the parent).
- In-flight parts shared at fork time complete once under the parent's writer; both
  sessions observe the same final content. Membership is snapshotted; content
  objects are shared and viewer-read-only.

### 8.5 Four concurrency invariants (must hold, documented for implementation)

1. **Lease write-ownership**: every write to a session's parts/membership requires
   holding that session's lease (exceptions: fork/rewind create new sessions;
   recovery and GC touch only stale/terminal data).
2. **Steal = atomic abort**: acquiring a stale lease aborts stale in-flight run
   markers of the session in the same transaction.
3. **Shared parts read/append-only**: non-creator sessions never update a part in
   place; divergence is by appending.
4. **GC safety**: orphan parts are deleted only when zero membership AND not
   referenced by an active run.

### 8.6 Cross-process cache invalidation

Per-process `SessionCache` entries are validated against `sessions.version`;
version changes caused by another instance force a reload. Version bumps on every
session mutation.

---

## 9. Old schema → new schema mapping

| v1 table | v2 disposition |
|----------|----------------|
| `agena_model_messages` | folded into `parts` (role/state/metadata/usage move onto the part or `usage`) |
| `agena_model_message_parts` | → `parts` |
| `agena_content_nodes` | → `parts` (node_id→part_id, owner→membership, title→summary, payload→content, lifecycle→part columns) |
| `agena_turns` / `agena_assistant_replies` / `agena_reply_executions` | deleted (run marker + interaction parts cover turn/reply/execution) |
| `agena_session_messages` | → `session_parts` (no order column; order = created_at_ms, part_id) |
| `agena_sessions` + `agena_session_lineage` | → `sessions` (lineage folded in) |
| `agena_events` | deleted (parts are the truth; no replay) |
| `agena_model_projection_states` | deleted (no projection, no watermark) |
| `agena_session_sequences` | deleted (no per-session sequence needed; order = part timestamps) |
| `agena_sequences` | → `sequences` (part_id + write-lock sentinel) |
| `agena_execution_leases` | kept (same, + steal-with-reconcile) |
| `agena_user_message_idempotency` | → `idempotency` (run_id → marker part_id) |
| `agena_permission_rules` / `agena_workspaces` | kept unchanged |
| usage (derived from message metadata) | → `usage` table |
| `agena_scheduler_jobs/history`, `model_catalog_*` | kept unchanged (infra) |

## 10. Migration strategy

- Bump `CURRENT_SCHEMA_VERSION` to the new version; per the project policy, older
  databases are rejected rather than migrated (fresh DB).
- Optional one-shot migration tool if preserving data is required:
  - `parts` from `model_message_parts` + `content_nodes`, aligned by
    `activity_id` / `segment_id`; message role/state/metadata folded onto the part.
  - membership from `agena_session_messages` (shared) + origin sessions;
  - turn/reply boundaries become `run` markers + interaction parts;
  - `usage` rows derived from message metadata;
  - events are dropped (optionally backfilled to a thin `part_oplog` if audit is
    ever needed — not in this design).

## 11. Open decisions

| # | Decision | Default (recommended) |
|---|----------|-----------------------|
| D1 | fork child transcript renders shared prefix turns/content | render (by design of run-marker sharing) |
| D2 | in-flight streaming parts at fork time | share by reference (child sees them complete) |
| D3 | migration policy | fresh DB + optional one-shot migration tool |
| D4 | per-session ordering | part timestamps: `ORDER BY created_at_ms, part_id` (no seq; id breaks same-ms ties) |
| D5 | `config_json` scope | execution config only (permission ceiling, capability denials, workspace root override, selection/access defaults); workflow state derived from parts; compaction policy from global settings, not stored per session |
| D6 | run demotion | `runs` table dropped; run = `kind='run'` marker part (decided) |

## 12. Performance notes

- Write amplification is the theoretical minimum: one row per content part, one
  membership edge, plus one tiny run-marker row per batch. No events, no
  projections, no content-node mirror, no tail copy, no id remap.
- Fork: O(shared edges); rewind: O(cutoff); read: one JOIN ordered by `created_at_ms, part_id`;
  recovery: one indexed scan over `parts(kind, state)`.
- The run marker adds ~2 small rows and 2 state UPDATEs per batch; reads need no
  extra join (reply status is on the marker row).

---

_Companion docs: `docs/database-design-audit.md` (v1 audit), this file (v2 design)._

---

## 13. Gap closure — v1 → v2 completeness (added after detailed v1 review)

A line-by-line review of the v1 schema and storage/query APIs against v2 found
the following gaps. This section closes them; it extends sections 4, 5, 6, 7, 8
and the open-decision table (section 11).

### 13.1 Sessions: restore tree columns and index (BLOCKING)

v1 `SessionSummary` carries `depth`, `root_id`, `message_count`, `child_session_count`,
`last_message_at`; session-tree listing and descendant cancellation walk
`root_id`/`parent_id`. v2 initially dropped `root_id`/`depth`. Restore them:

```sql
ALTER TABLE sessions ADD COLUMN root_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN depth   INTEGER NOT NULL DEFAULT 0;
-- sessions.parent_id gets ON DELETE CASCADE (deleting a root deletes the subtree,
-- matching v1; shared parts survive via the GC guard because edges cascade but parts do not).
CREATE INDEX idx_sessions_root  ON sessions(root_id, updated_at_ms);
CREATE INDEX idx_sessions_task  ON sessions(parent_id, task_id);   -- get_subagent_by_task_id
```

### 13.2 Parts: only provider_state persists on the run marker (BLOCKING, revised)

v1 `MessageMetadata` is **dissolved, not stored**: a per-message metadata JSON
column would multiply tiny records across every message and is unnecessary in
v2 (a message == one run marker; metadata was a v1 artifact). Field by field:

| v1 MessageMetadata field | v2 disposition | evidence |
|--------------------------|----------------|----------|
| `idempotency_key` | `idempotency` table (already exists) | |
| `model_turn_id` | dropped (marker == turn) | |
| `parent_message_id` | dropped (never read at runtime; derivable = previous marker in membership) | only set at creation (replies_state.rs:504) |
| `generated_by_call_id` | dropped (never read) | only set to None (run.rs:143) |
| `externally_initiated_tool` | boolean on the tool_call part content | read at replies.rs:856 |
| `model_provider_id` / `model_adapter_id` / `model_id` | run-level: `usage` table (attribution/cost, cost.rs:154) + continuation check reads the last run marker (replies.rs:371) | no per-message need |
| `model_thinking_mode` / `model_speed_mode` | run-level settings on the run marker content if replay fidelity requires (one per run, not per message) | |
| `source` | derived from `role` + `kind`; compaction uses role==User && source==User (compact.rs:535) == role='user' && kind='text' | |

Only `MessageProviderState` persists, as a nullable JSON column on the run
marker of assistant messages that need provider continuation (response_id,
thought signatures, thinking blocks). It is NOT on every part — only on
assistant run markers, and empty for the rest.

```sql
ALTER TABLE parts ADD COLUMN provider_state JSON;  -- MessageProviderState, assistant run markers only
```

### 13.3 Provider anchors must persist (BLOCKING)

`ProviderPromptAnchor` (provider_id, model_id, previous_response_id,
assistant_message_id, prompt_window_generation, system_fingerprint,
request_options_fingerprint, provider_request_shape, transcript_digest) is
provider-side continuation/cache state that is NOT derivable from parts (the
provider returns it at runtime). v1 stores it in `runtime_state_json`, and the
import path explicitly restores it so the next run does not re-prime caches.
Dropping it in v2 would silently lose prompt-continuation caching across
restarts and across import/export.

```sql
ALTER TABLE sessions ADD COLUMN provider_anchors_json JSON;  -- map<provider_id, ProviderPromptAnchor>
-- maintained per run completion; cleared on compaction (mirrors v1 clear_provider_anchors)
```

### 13.4 Prompt window / token accounting (SHOULD, derive + slim persist)

v1 `PromptWindowRuntime` / `PromptTokenRuntime` (prompt_window, prompt_tokens)
are reconstructible in v2: the window is the membership parts after the last
compaction part; the compaction part itself marks the boundary; token counts
can be recomputed or taken from `usage`. Keep only what is not derivable:
`provider_anchors_json` (13.3) and `config_json` (5.1). No prompt-window
columns are added; this is a decision to validate during implementation.

### 13.5 Session stats are derived — define the queries (SHOULD)

v1 `SessionStatsRepository` (workspace_counts, event_stats, child_counts)
becomes derived SQL over v2 tables:

```sql
-- message_count per session: count run markers (a message == one run)
SELECT m.session_id, COUNT(*) FROM session_parts m
JOIN parts p ON p.part_id = m.part_id WHERE p.kind = 'run' GROUP BY m.session_id;
-- last_message_at per session
SELECT sp.session_id, MAX(p.created_at_ms) FROM session_parts sp
JOIN parts p ON p.part_id = sp.part_id GROUP BY sp.session_id;
-- child_counts per parent
SELECT parent_id, COUNT(*) FROM sessions GROUP BY parent_id;
-- workspace_counts
SELECT workspace_id, COUNT(*) FROM sessions GROUP BY workspace_id;
```

Decision D9 (message_count semantics): count `kind='run'` markers = messages;
UI part counts are a separate query if needed.

### 13.6 Part-lookup semantics change in a multi-session world (SHOULD)

v1 `ProjectionLookupRepository.session_id_for_message/part` answered 「which
session owns this」. v2 has two answers:

- ownership: `parts.origin_session_id` (fast, indexed `idx_parts_origin`);
- visibility: `SELECT session_id FROM session_parts WHERE part_id = ?`
  (`idx_session_parts_part`).

API consumers (TUI navigation via `find_session_id_for_message/part`) default to
ownership (origin session); visibility is available for fork-aware UIs.

### 13.7 Timeline / event API surface redefinition (SHOULD, API breaking)

| v1 API | v2 replacement |
|--------|----------------|
| `list_session_events` / `stream_session_events` (REST, event envelopes) | persisted history = ordered parts (no event concept remains); live updates = part-patch stream over the in-memory bus plus ephemeral runtime signals (retry/progress) that are never persisted; wire format changes from event envelopes to part patches |
| `latest_event_seq` | `sessions.version` or the newest member part (`MAX(created_at_ms, part_id)`) |
| `export_session_jsonl` | serialize session meta + ordered parts (markers carry provider_state; anchors from 13.3); one part per line |
| `import_session_jsonl` | re-create session + parts + membership with fresh ids; remap `parent_part_id`/`run_id` chains; restore provider_state/anchors; drop ALL lineage — import always yields an independent root session (matches v1: create_session with parent None; subtask events filtered) |
| `TranscriptSnapshot`/`TranscriptPatch` (turns) | marker-grouped parts (turns = run marker + its parts); UI renders markers as turn boundaries |
| `ActivityId`/`TextSegmentId`/`MessageId` | dissolve into `part_id` (API uses part ids) |

Note — 「events/timeline」 conflates three needs, and only the first disappears:
(1) persisted history → ordered parts (no event concept remains);
(2) live incremental updates → a UI/transport requirement that survives as part
patches (new/updated/removed part) plus ephemeral bus signals (retry/progress)
that are never persisted — the in-memory bus stays, its payload language changes;
(3) catch-up after reconnect / cross-process refresh → `sessions.version` /
newest member part (`created_at_ms`, `part_id`) based 「parts after (time, id)」. Consumers: TUI
(TranscriptPatch → part patches), web UI session timeline command (reads ordered
parts grouped by run markers).

### 13.8 Usage queries need session context (SHOULD, updated for normalized usage)

v1 usage records carry session title + is_subagent and filter by workspace /
session list / time range / `include_subagents`. v2 `usage` denormalizes
`workspace_id` and stores scalar token/cost columns (section 16); title and
is_subagent come from a JOIN when the caller needs them:

```sql
SELECT u.*, s.title, (s.relation_kind = 'subagent') AS is_subagent
FROM usage u JOIN sessions s ON s.session_id = u.session_id
WHERE u.workspace_id = ? AND s.lifecycle_state = 'ready'
  AND (s.relation_kind != 'subagent' OR :include_subagents)
  AND u.created_at_ms BETWEEN :from AND :to;
```

### 13.9 Streaming write policy (SHOULD, explicit)

v1 deliberately had 「0 writes while streaming」 for activities (memory-only
deltas) and checkpoint events for parts. v2 writes part rows directly and
updates `content` + `revision` per delta; that is more write amplification per
stream than v1's activity path. Policy: throttle in-place part updates (flush
at most every N deltas or on run completion), keep live deltas on the in-memory
bus as v1. This is an implementation knob, not a schema change.

### 13.10 Attachment semantics: inline text vs references (REVISED)

Chat data never stores blobs:

- `paste_ref` carries the full pasted text INLINE in the part content (the paste
  IS the content and must be stored);
- `file_ref` carries only a reference (path/name/mime/sha) — the AI reads the
  file from the filesystem when needed; no blob is stored;
- `skill_ref` carries only a reference (skill name/args) — the skill is resolved
  at execution time.

The app server's attachment cache (a separate sqlx DB) serves non-chat server
features (terminal/preview/…) and is unrelated to chat parts.

### 13.11 Non-DB surfaces unchanged (INFO)

`MemoryRepository` / `MemoryDir` (filesystem memory docs), `model_catalog_*`,
`scheduler_*`, permission policy resolution, leases heartbeat timing all stay
as-is; they are not part of the chat-data schema.

### 13.12 Open-decision additions

| # | Decision | Default (recommended) |
|---|----------|-----------------------|
| D7 | message-level metadata | dissolved: no `metadata` column; only `provider_state` on assistant run markers; all other fields redistributed (13.2) |
| D8 | provider anchors | persist `sessions.provider_anchors_json` (not derivable; required for cache continuation + import round-trip) |
| D9 | message_count semantics | count of run markers (a message == one run) |
| D10 | streaming write policy | throttle part updates; live deltas stay in-memory |

---

_End of v2 design (gap closure included)._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 14. Unified session facade — no event concept anywhere

External callers (TUI, Web, CLI, tests) interact with ONE facade that hides the
memory/DB boundary completely. Persistence is purely internal.

### 14.1 Public shape

```rust
// Pure data model — no DB types leak out
pub struct Part { part_id, kind, role, state, content, summary, parent_part_id, run_id, ... }
pub struct SessionView { meta: SessionMeta, parts: Vec<Part> }  // ordered by created_at_ms, part_id

// The ONLY live-update concept: change notifications, not an event log.
// Derived from operations, emitted after commit, never persisted, never replayed.
pub enum SessionChange {
    PartAdded   { part },
    PartUpdated { part_id, revision, state, content },   // streaming deltas
    PartRemoved { part_id },
    SessionMetaUpdated { meta },
}

#[async_trait]
pub trait SessionStore {
    async fn load(&self, session_id: i64) -> Result<SessionView>;
    async fn list_session_summaries(&self, query: SessionListQuery) -> Result<Vec<SessionSummary>>;
    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>>;
    async fn session_state(&self, session_id: i64) -> Result<SessionPresentation>;
    async fn submit_user_message(&self, session_id, parts: Vec<NewPart>) -> Result<RunId>;
    async fn append_parts(&self, session_id, run_id, parts: Vec<NewPart>) -> Result<()>;  // streaming
    async fn update_part(&self, session_id, part_id, delta) -> Result<()>;                // streaming delta
    async fn complete_run(&self, session_id, run_id, outcome) -> Result<()>;
    async fn answer_interaction(&self, session_id, interaction_part_id, reply) -> Result<()>;
    async fn fork(&self, session_id, at_part_id, title) -> Result<i64>;
    async fn rewind(&self, session_id, at_part_id, title) -> Result<i64>;
    async fn rename(&self, session_id, title) -> Result<SessionMeta>;
    async fn cancel_run(&self, session_id, run_id) -> Result<()>;
    async fn compact_session(&self, session_id) -> Result<RunId>;
    async fn delete(&self, session_id) -> Result<()>;
    async fn export_session_jsonl(&self, session_id) -> Result<String>;
    async fn import_session_jsonl(&self, bundle: &str) -> Result<i64>;
    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats>;
    fn subscribe(&self, session_id, observer: impl Fn(SessionChange)) -> Subscription;
}
```

### 14.2 Internals (invisible to callers)

- `load`: memory cache first, then a single membership JOIN against the DB.
- Writes: validate lease -> one transaction (parts + membership + `sessions.version`
  bump) -> after commit, notify subscribers -> return. Callers never see a
  memory/DB split.
- Recovery (lease steal -> abort stale run markers) and GC are maintenance
  internals.
- In-memory backend for tests (v1 MemoryStore precedent): tests run without SQLite.

### 14.3 The event concept is gone

| v1 event role | v2 replacement |
|---------------|----------------|
| source of truth / audit / replay | parts are the truth (no replay) |
| internal projection driver | no projections; domain operations persist directly |
| notify subscribers (live updates) | `SessionChange` notifications derived from the same operation, emitted after commit |
| catch-up (reconnect / cross-process) | `sessions.version` / newest member part (`created_at_ms`, `part_id`) |

`SessionChange` is a notification (observer pattern), not an event: never
persisted, never replayed, no causality chain. History = ordered parts.

### 14.4 Cross-process live updates (honest boundary)

- Same-process subscribers: backed by the in-memory bus (TUI is in-process, zero cost).
- Cross-process (server executes, another instance's UI watches): `subscribe` is
  backed by the existing notification stream (`agena-runtime-notifications` / SSE)
    plus `version`/`(created_at, part_id)` catch-up for late joiners. The facade hides the transport;
  it is a notification channel, not an event log.

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 15. Encapsulation architecture — memory + DB are one sealed facade

Goal: external callers cannot tell memory from database. All state is managed
internally; live updates and reads are excellent; intrusive direct-DB writes
(the v1 audit findings) become impossible by construction.

### 15.1 Layering

```
External callers (TUI / Web / CLI / API server / tests)
        |  depend only on the facade trait + pure domain types
        v
+------------------------------------------------------------+
| SessionFacade (trait SessionStore, section 14)             |  <-- ONLY public entry
|   read   : cache -> persistence                            |
|   write  : validate -> txn -> cache -> notify              |
|   live   : subscribe(SessionChange)                        |
|------------------------------------------------------------|
| internal: MemoryLayer      (per-session LRU cache,         |
|                             streaming buffers, pending ops)|
| internal: PersistenceEngine(SQLite repos, transactions,    |
|                             leases, recovery, GC)          |
| internal: NotificationBus  (in-process bus + cross-process |
|                                                          stream + version/(created_at, part_id) catch-up) |
+------------------------------------------------------------+
       ^
       | the only place that imports sea_orm / holds DatabaseConnection
       v
   SQLite file (parts, session_parts, sessions, ...)
```

### 15.2 The sealed DB boundary (eliminates the v1 audit findings)

- `DatabaseConnection` and every raw SQL statement live ONLY inside the
  persistence engine. No other crate imports `sea_orm` for the chat DB.
- The facade is the ONLY write path for chat data. Everything an external
  caller can do goes through `SessionStore` methods.
- v1 leaks fixed by construction: the raw SQL in `agena-runtime-session`
  (content nodes, turns, replies, leases, membership, session CRUD) becomes
  facade/internal-engine code; the scheduler keeps its own repository
  interface or moves off the chat DB; the app-server KV/cache DB stays a
  separate sealed subsystem (out of scope).
- Enforcement: crate dependency rules (only the engine crate may depend on
  `sea_orm`/`sqlx` for chat data) + module privacy (the connection type is
  not exported) + review checklist (any new `DatabaseConnection`/raw SQL
  outside the engine is a violation).

### 15.3 Memory layer (evolved from v1 `SessionCache`)

v1 `SessionCache` (LRU + TTL + byte budget + max sessions + stats,
`session_cache.rs`) is kept and extended:

- Per-session LRU cache of `SessionView` (parts ordered by `created_at_ms, part_id`).
- Streaming buffers: in-progress parts held in memory, flushed per the
  throttle policy (13.9) with `revision` guards; UI sees deltas instantly.
- Read hot path: cache hit -> zero DB; miss -> one membership JOIN -> insert.
- Invalidation: same-process writes discard/update the entry after commit;
  cross-process changes detected by `sessions.version` comparison on hit.

### 15.4 Persistence engine (internal, swap-friendly)

- Owns the connection and the repository implementations: parts, membership,
  sessions, usage, idempotency, leases, sequences.
- Transactions with the write-lock sentinel + busy retry (section 8.1).
- Recovery: lease steal -> atomically abort stale run markers (7.2); GC.
- Two backends behind one engine trait: `SqliteEngine` (production) and
  `InMemoryEngine` (tests / small deployments) — the facade is composed with
  either at runtime (v1 `MemoryStore` precedent, extended to all tables).

### 15.5 Notification bus (the only live mechanism)

- `SessionChange` is emitted after commit: same-process subscribers via the
  in-memory bus; cross-process via the notification stream plus
    `version`/(`created_at`, `part_id`) catch-up for late joiners (14.4).
- `SessionFacade::subscribe` hides the transport; callers see one API.

### 15.6 Write path (commit-then-notify)

```
submit_user_message(session, parts):
  1. acquire session lease (if not held)
  2. transaction: run marker + content parts + membership edges + version++
  3. commit
  4. update memory cache
  5. emit SessionChange
  6. return RunId

streaming append/update:
  1. mutate in-memory part immediately (UI latency ~0)
  2. persist throttled (13.9) with revision guard, inside lease ownership
  3. notify on each flush
```

### 15.7 Relationship to the execution engine

`SessionFacade` is the DATA layer. The runtime execution engine (model calls,
tool loop, interaction answering) is a higher-level service that orchestrates
the work and calls the facade for every read/write. External callers use the
runtime services; the facade stays the only data/state surface. No layer
outside the engine touches the DB.

### 15.8 Performance posture

- Reads: cache-first; live updates push, no polling.
- Writes: memory-first latency, throttled persistence, revision guards.
- Fork/rewind: membership operations through the facade (section 7.3).
- Multi-instance: version/(created_at, part_id) catch-up; single-writer lease per session.

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 16. Usage storage — low footprint, high performance

### 16.1 Why v1 was suboptimal

- v1 stored the full `CompletionUsage` JSON (10+ token counters, f64 cost,
  cost provenance) inside every assistant message row of the big messages
  table: roughly 200-400 bytes per model call as opaque JSON.
- Aggregation (workspace totals, per-provider/model, time ranges) required
  pulling rows and summing in code, or awkward JSON extraction — no pure-SQL
  SUM.
- Cost was `f64` USD (8 bytes each, float drift, hard to sum exactly).

### 16.2 v2 design (table in 5.6)

1. **Normalized scalar columns**: `input/output/reasoning/cache_write/cache_read/
   tool_use/other_tokens` as INTEGER, `total_cost_micros`/`recorded_cost_micros`
   as integer micro-USD. SQLite varint encoding makes typical token values 1-2
   bytes; a row is roughly 60-120 bytes versus 200-400 for JSON — 2-3x smaller
   AND directly SUM-able.
2. **Rare detail in `detail_json`**: provider-specific extras (5m/1h cache
   buckets, per-category breakdowns, provenance) stay NULL for almost all rows.
3. **Append-only + immutable**: one row per provider response, inserted once at
   model-call completion, never updated — aggregation never contends with
   writers; no hot rows.
4. **Denormalized `workspace_id`**: the most common query (workspace-wide
   totals) filters on `usage` directly with no join. `is_subagent`/title stay in
   a cheap PK join when needed (13.8).
5. **Three indexes** cover the query shapes: workspace×time, session×time,
   provider×model×time.

### 16.3 Query shapes (all pure SQL over index ranges)

```sql
-- per-session totals
SELECT provider_id, model_id, COUNT(*), SUM(input_tokens), SUM(output_tokens),
       SUM(reasoning_tokens), SUM(cache_write_tokens), SUM(cache_read_tokens),
       SUM(total_cost_micros)
FROM usage WHERE session_id = ? GROUP BY provider_id, model_id;

-- workspace-wide totals over a time range
SELECT provider_id, model_id, COUNT(*), SUM(total_cost_micros)
FROM usage WHERE workspace_id = ? AND created_at_ms BETWEEN ? AND ?
GROUP BY provider_id, model_id;

-- per-day chart
SELECT created_at_ms / 86400000 AS day, SUM(input_tokens), SUM(total_cost_micros)
FROM usage WHERE workspace_id = ? GROUP BY day ORDER BY day;
```

### 16.4 Granularity and provenance

- One row per provider response (model call). A turn (run marker) may contain
  many calls (tool loop); `run_id` groups them, so per-turn cost = `SUM` over
  the run, per-call detail remains available.
- `cost_estimate_incomplete` mirrors v1's "cost may be incomplete" signal;
  `recorded_cost_micros` (provider-reported) takes precedence over
  `total_cost_micros` (computed/estimated) in reporting, matching v1 cost
  logic (`usage_cost.rs`).

### 16.5 Long-term storage (future, not v1)

For very large installations, add daily/hourly rollup rows (workspace ×
provider × model × day) and prune raw rows older than a retention window. The
rollup table is the same scalar shape, so queries are unchanged; this is an
optional maintenance job, not a schema redesign.

### 16.6 Through the facade

Usage is written once per model call by the execution engine via the facade
(internal to the persistence engine); all reads go through a facade
`usage_stats` query (per-session / workspace / provider-model / time-range).
External callers never touch the table and cannot distinguish memory/DB.

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 17. Data-driven session state machine

Goal: every session condition is reconstructible from parts alone. Closing or
crashing at ANY point leaves enough data to reopen, derive the exact state, and
continue the right next step. There is exactly ONE session state for display —
no conflicting fields.

### 17.1 Principles

1. **State is a projection of parts, not stored separately.** Run markers and
   interaction parts carry the execution state; a pure derivation function maps
   DB rows to the single `SessionState`. Any process derives the same state
   from the same rows (consistent, unique).
2. **Everything recoverable**: no in-memory state that is not rebuildable from
   parts. The runtime's execution registry is an execution cache, not state.
3. **One uniform part lifecycle** for every part kind; transitions enforced by
   triggers (6).

### 17.2 Part lifecycle (uniform)

Every part (text / think / tool_call / tool_result / interaction / notice /
hook / run marker) uses the same state enum with the same transitions:

```
pending ──► in_progress ──► completed
   │            │  ▲           failed
   └────────────┴──┴──────►  cancelled
                  (retry)
```

- `pending → in_progress | cancelled`
- `in_progress → completed | failed | cancelled`
- `failed → in_progress` (retry; `revision++`, `finished_at` cleared, retry policy
  lives in the engine, 18.2)
- terminal states (`completed` / `cancelled`) are immutable; `revision` is
  monotonic on every update.
- Errors are durable records: an `error` part is never deleted; after a retry
  succeeds, the error part(s) AND the successful result part both remain (18.1).
- `abort_reason` (JSON in content) is required on run markers reaching
  failed/cancelled, and mirrors v1 reasons: `process_restart`, `user_cancelled`,
  `provider_error`, `fork_cutoff`, `replaced`, `budget_limited`, `lease_stolen`.

### 17.3 The single derived SessionState

```rust
enum SessionState {
    Creating,      // sessions.lifecycle_state = creating
    Ready,         // no in-flight run, no pending interaction
    Running,       // in-flight run marker, fresh lease (this process or another)
    AwaitingUser,  // a pending interaction part (ask_user / plan_review / permission)
    Interrupted,   // in-flight run marker, stale/no lease (crash) — reconciling
    Failed,        // lifecycle failed, or last run terminally failed and not resumable
}

fn derive_session_state(session, in_flight_run, pending_interactions, lease) -> SessionState {
    if session.lifecycle_state == Creating { return Creating }
    if session.lifecycle_state == Failed  { return Failed }
    if !pending_interactions.is_empty()   { return AwaitingUser }   // user's turn wins
    if let Some(marker) = in_flight_run {
        return if lease_is_fresh(lease) { Running } else { Interrupted }
    }
    Ready
}
```

Only **interactions** (pending) and **run markers** (in-flight) gate the session
state. All other parts (text, tool calls, hooks, notices) have their own
lifecycle but never block the session — hooks and notices are non-gating by
construction.

### 17.4 Resume algorithm (exactly one correct next step on open)

```
open_session(session_id):
  1. lifecycle = creating  -> finalize or fail the session row, then re-derive
  2. for every in-flight run marker (state pending|in_progress):
       a. if the run owns a pending interaction -> leave it (the run is paused
          on the user, NOT crashed); state = AwaitingUser
       b. else if lease is fresh -> another process is actively running it;
          state = Running
       c. else (stale/no lease) -> reconcile: mark the run failed
          (abort_reason = process_restart), mark its non-terminal child parts
          cancelled; state becomes Ready
  3. derive SessionState and return it together with the pending interaction
     (if any) and last failure (if any)
```

Every possible DB combination has a defined outcome: creating → finalize;
ready + pending interaction → AwaitingUser; ready + in-flight run + fresh lease
→ Running; ready + in-flight run + stale lease → Interrupted → reconcile →
Ready; failed → Failed.

### 17.5 Answer-and-continue flows

- **ask_user / plan_review / permission** (`interaction` part, `state=pending`):
  UI shows the interaction; the user answers via `answer_interaction` ->
  transaction { interaction → completed; append reply part
  (`parent_part_id` = interaction, `run_id` = owning run marker) }.
  Then: if the owning run marker is still in_progress, the engine resumes that
  run (rebuild prompt from parts including the answer, call the provider);
  otherwise it starts a new continue run. State → Running.
- **continue after interruption**: the run was reconciled to failed
  (`process_restart`); the user (or policy) starts a `continue` run — a new run
  marker whose prompt is rebuilt from the completed parts. The prompt builder
  decides how to present interrupted/failed parts (exclude, or include a
  synthetic tool_result for an aborted tool_call).
- **user cancel**: current run marker → cancelled, its non-terminal parts →
  cancelled; state → Ready.
- **steer input**: appends a user part to the in-flight run; state unchanged
  (Running).

### 17.6 Display contract

UI reads ONE object from the facade:

```rust
struct SessionPresentation {
    state: SessionState,              // the only session-level state
    pending_interaction: Option<InteractionRef>,  // type + prompt + part_id when AwaitingUser
    active_run_id: Option<i64>,       // run marker part_id when Running
    last_failure: Option<Failure>,    // when Failed / after Interrupted reconcile
    // streaming progress is in-memory only, never persisted, never part of state
}
```

No other field may contradict `state`; part-level details (tool progress,
activity statuses) are per-part data shown alongside, not session state.

### 17.7 Consistency rules

- One derivation function, one enum — same rows, same state, any process.
- Triggers enforce part transition validity (6) and run-marker invariants
  (`run_id IS NULL` on markers; abort_reason on terminal failure).
- Lease is the only cross-process arbiter: it converts 「in-flight run」 into
  Running vs Interrupted; the reconcile step is the only mutator of a crashed
  run's parts, and it runs exactly once (idempotent transitions).
- Recovery is idempotent: re-running the resume algorithm on an already
  reconciled session is a no-op (markers already terminal, interactions answered
  or cancelled).

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 18. Errors, retries, visibility, and rendering

### 18.1 Errors are parts, and they are durable

Every runtime failure produces an `error` part (`kind='error'`, `role='runtime'`
or `'tool'`, `state='failed'`) so the session keeps a complete record:

```json
{"code":"provider.response_failed","category":"dependency",
 "message":"...","detail":{...},"retryable":true,"attempt":1}
```

- Error parts are **never deleted** — they are history, exactly like messages.
- They are non-gating: an `error` part alone never changes `SessionState`
  (17.3). A run that ended in failure is expressed by the run marker's terminal
  state, with the `error` part(s) as the explanation.

### 18.2 Retry semantics (part updates, nothing deleted)

1. The operation part (e.g., `tool_call`) transitions `in_progress → failed`,
   and a child `error` part is created for the attempt.
2. Retry: the operation part transitions `failed → in_progress`
   (`revision++`, `finished_at` cleared); the engine re-runs the operation.
3. On success the result part (`tool_result`) is created `completed` and the
   operation part finishes `completed`.
4. History: the `error` part(s) and the successful `tool_result` part both
   remain — retries leave an audit trail, success leaves the usable result.

Retry policy (max attempts, backoff, when to give up) is engine logic recorded
in the error part's `attempt`/`retryable` fields, not storage logic. The same
pattern applies to provider calls (run marker `failed → in_progress` is NOT
allowed for a marker — a failed run is terminal; retrying a run creates a new
`continue` run, matching 17.4).

### 18.3 Visibility: who sees each part

```sql
visibility TEXT NOT NULL DEFAULT 'both' CHECK (visibility IN ('both','user','ai'))
```

| value | prompt (AI) | UI (human) | examples |
|-------|-------------|------------|----------|
| `both` | yes | yes | normal text, tool calls/results, interactions |
| `ai`   | yes | no | internal tool data, hidden context, raw function payloads |
| `user` | no  | yes | session notices, user-facing errors, permission explanations (v1: 「session Notice nodes never reach the model prompt」) |

Rules: the prompt builder includes parts with `visibility IN ('both','ai')`;
the UI renders parts with `visibility IN ('both','user')`. Fork shares parts as
they are — visibility is per part, not per session.

### 18.4 Rendering: plugin/tool renders Markdown for humans; AI gets raw data

- `content` is the canonical raw data — exactly what the AI sees.
- `rendered_markdown` (nullable) is the human-friendly Markdown produced by the
  plugin or tool that created the part (its own renderer, its own formatting).
- UI rule: show `rendered_markdown` when present; otherwise fall back to a
  generic rendering of `content`/`summary`.
- AI rule: the prompt always uses `content` (raw), never the rendered markdown.
- `rendered_markdown` may be (re)rendered at any time (`revision++`); it is
  cached on the part so display works even when the producing plugin is not
  loaded (offline/other process).

### 18.5 State-machine impact

- Errors and retries never introduce a new session state: `error` parts are
  non-gating, and retrying an operation is an internal part transition
  (`failed → in_progress`).
- Visibility and rendering do not affect the state machine at all — they only
  change prompt/UI filtering and display. `SessionPresentation` (17.6) stays the
  single source of session state; per-part visibility/rendering is detail.

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._

---

## 19. v1 coverage matrix and gap closure

Four read-only sub-audits walked every v1 surface (storage ops, execution flows,
query/REST/UI, domain types) against this design. Verdict: storage operations
and execution flows are 100% covered (the event-log/projection machinery is
intentionally dropped); query/UI and domain payloads need the closures below.

### 19.1 Storage operations — covered

All `agena-storage` traits, session-store operations, leases, sequence
allocation, transactions, reconcile, workspace/catalog/permission/usage/stats,
memory docs map to v2 engine/facade or kept-infra; `EventStore` and
`ModelMessage*`/projection writers are dropped by design. v2 even ADDS orphan
part GC that v1 lacked. Open items are implementation knobs (D4 ordering,
D10 streaming throttle).

### 19.2 Execution flows — covered

Run lifecycle, streaming/checkpoints, tool calls/results, ask-user, permission,
plan review, compaction, hooks, subagents, cancel, steer, continue, crash
reconcile all map to part kinds + states + the state machine (17). Three v1
runtime fields need homes (19.5).

### 19.3 Facade completeness (14.1) — closed

Added to `SessionStore`: `list_session_summaries`, `list_session_tree`,
`session_state`, `rename`, `cancel_run`, `compact_session`,
`export_session_jsonl`, `import_session_jsonl`, `usage_stats`. (Session listing,
export/import, cancel, compact, meta-update were missing from the first draft.)

### 19.4 Content shape breadth (4.1.1) — extended shapes

Payload breadth, not missing kinds:

- `file_ref`: extend to `{path|url|data_url|file_id|base64, name, mime, sha,
  width, height, duration_ms, page_count}` (covers v1 Resource/Attachment
  sources and media dims).
- `tool_result`: extend to `{output, ok, structured, model_preview,
  managed_outputs, display, attachments, metadata, raw}` (covers v1
  `ToolResultEnvelope`); the human view is `rendered_markdown` (18.4).
- `error`: extend to full `UserProblem` shape `{code, category, message,
  detail, responsibility, retry, recovery, impact}`.
- `notice`: add `title`; `hook`: add `plugin_id`.
- `think`: allow `encrypted_content` (v1 encrypted reasoning); `text`: allow
  `synthetic` flag.
- `skill_ref`: KEEP reference-only `{skill, args}` (user decision: the AI
  resolves the skill on demand; v1's snapshot of instructions is dropped).
- provider-replay state stays in the `provider_state` column (13.2).

### 19.5 Runtime-state field homes

| v1 field | home |
|----------|------|
| `consecutive_compaction_failures` / `auto_compaction_disabled` / `remote_compaction_disabled_models` | **not stored per session** (D5): policy knobs read from agena global settings; backoff counter transient, in-memory only |
| `model_context_window_tokens` | derived from model catalog at prompt build (not persisted) |
| execution `selection` / `access` (model choice, execution access) | run marker content (per run) + `config_json` (session defaults) |
| exclusive-reply matching (turn_id/reply_id) | marker-based matching (run marker part_id is the turn identity) |

### 19.6 Activities/operations registry mapping

v1 activity registry (logs, stop/dismiss, background tasks, `operation_detail`)
maps to parts: activity log = parts history; stop/dismiss = part state
transitions (cancelled/removed); background tasks = `run_kind='background'`
markers; `operation_detail` = `rendered_markdown` (18.4). A separate activity
registry table is not needed.

### 19.7 Global runtime history — decided: dropped

v1 `list_events` / `ListEvents` / `event_query_service` (cursor-paged global
event history across sessions) is **dropped** with no replacement. The event
log it exposed is gone (2.2); chat history is per-session ordered parts
(4.4), browsed through `SessionStore.list_session_summaries` (14.1). A
cross-session diagnostic stream, if ever needed, can be derived later from
`parts` directly.

### 19.8 Open-decision additions

| # | Decision | Default |
|---|----------|---------|
| D11 | global runtime history | **dropped** (no global event history; chat history is per-session ordered parts) |
| D12 | `skill_ref` depth | reference-only `{skill, args}`; resolve on demand |
| D13 | attachment source breadth | extended `file_ref` shape (url/data_url/file_id/media dims) |

---

_End of v2 design._ _Companion docs: `docs/database-design-audit.md` (v1 audit)._
