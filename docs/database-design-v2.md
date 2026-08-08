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
    content           JSON    NOT NULL,           -- typed payload per kind
    summary           TEXT,                       -- compact label (O(1) list updates)
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
| `file_ref` | user | `{"path":"...","name":"...","mime":"...","sha":"..."}` | file attachment reference |
| `paste_ref` | user | `{"path":"...","name":"...","mime":"...","sha":"..."}` | clipboard reference |
| `skill_ref` | user | `{"skill":"...","args":{...}}` | skill invocation reference |
| `attachment` | user / assistant | `{"items":[...]}` | attachment set |
| `notice` | runtime | `{"kind":"...","summary":"...","detail":"..."}` | system notice (hook runs etc.) |
| `hook` | runtime | `{"hook":"...","summary":"...","detail":"..."}` | hook activity |
| `compaction` | runtime | `{"summary":"...","window":[...]}` | compaction summary |
| `error` | runtime | `{"category":"...","message":"...","detail":...}` | failure |
| `interaction` | system | `{"type":"ask_user|plan_review|permission","prompt":"...","options":[...],"response":...}` | user processing point |

A `run` marker is created for every execution batch (user send / continue /
background). It carries the batch state and abort reason; the reply/status the UI
needs is the marker state. `parts.run_id` references the marker part so
「all parts of run X」 is a flat indexed query (no recursion).

### 4.2 `session_parts` — the only ownership mechanism

```sql
CREATE TABLE session_parts (
    session_id  INTEGER NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    part_id     INTEGER NOT NULL REFERENCES parts(part_id),
    seq         INTEGER NOT NULL,                 -- per-session order; MAX(seq)+1 on append
    added_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, part_id),
    UNIQUE (session_id, seq)
);

CREATE INDEX idx_session_parts_part ON session_parts(part_id);  -- reverse: which sessions share a part
```

- `seq` is assigned inside the same write transaction as the insert
  (`MAX(seq)+1` for the session), protected by the SQLite write lock, so
  concurrent appends cannot collide.
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
    config_json           JSON,                   -- execution config only: permission ceiling,
                                                  -- capability denials, workspace root override
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

### 5.6 `usage` — per-run metrics

```sql
CREATE TABLE usage (
    usage_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    INTEGER NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    run_id        INTEGER,                        -- run marker part_id
    provider_id   TEXT NOT NULL,
    model_id      TEXT NOT NULL,
    usage_json    JSON NOT NULL,                  -- tokens / cost / cache / reasoning
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_usage_session ON usage(session_id, created_at_ms);
```

Written at run completion from the provider usage event. Replaces v1 derivation
from message metadata.

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

1. **parts lifecycle**: legal state transitions (pending → in_progress → terminal),
   `revision` monotonic, timestamps self-consistent (CHECKs above).
2. **parts identity immutable**: `part_id, role, kind, parent_part_id,
   origin_session_id, created_at_ms` may not be updated.
3. **session_parts references**: part and session must exist; `seq` unique per session.
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
3. Insert membership rows: marker + content parts with `seq = MAX(seq)+1` each.
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
INSERT INTO session_parts (session_id, part_id, seq, added_at_ms)
SELECT :child_id, part_id, seq, :now FROM session_parts WHERE session_id = :parent AND seq <= :cutoff;
-- rewind: seq < :cutoff, relation_kind='rewind'
```

Cost O(shared edges); zero content copy, zero id remap. Forking a session while the
parent is streaming is safe under the shared-part rule (4.3 below).

### 7.4 Read a session transcript (one query)

```sql
SELECT p.*, sp.seq FROM session_parts sp JOIN parts p ON p.part_id = sp.part_id
WHERE sp.session_id = :session ORDER BY sp.seq;
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
| `agena_session_messages` | → `session_parts` (+seq) |
| `agena_sessions` + `agena_session_lineage` | → `sessions` (lineage folded in) |
| `agena_events` | deleted (parts are the truth; no replay) |
| `agena_model_projection_states` | deleted (no projection, no watermark) |
| `agena_session_sequences` | deleted (seq = MAX+1 in txn) |
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
| D4 | per-session ordering | explicit `seq` (stable, reorderable) |
| D5 | `config_json` scope | execution config only; workflow state derived from parts |
| D6 | run demotion | `runs` table dropped; run = `kind='run'` marker part (decided) |

## 12. Performance notes

- Write amplification is the theoretical minimum: one row per content part, one
  membership edge, plus one tiny run-marker row per batch. No events, no
  projections, no content-node mirror, no tail copy, no id remap.
- Fork: O(shared edges); rewind: O(cutoff); read: one JOIN ordered by seq;
  recovery: one indexed scan over `parts(kind, state)`.
- The run marker adds ~2 small rows and 2 state UPDATEs per batch; reads need no
  extra join (reply status is on the marker row).

---

_Companion docs: `docs/database-design-audit.md` (v1 audit), this file (v2 design)._
