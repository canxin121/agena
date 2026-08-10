# AGENA DB v2 — Full Refactor Handoff Prompt

You are handed the complete redesign of agena's persistence + execution data
layer. Execute the ENTIRE refactor, from zero, with no historical baggage.

## Mission

Rewrite agena's data layer to the v2 **"everything is a part"**
(membership-first) design and rewire every dependent component onto it.
**From zero:** no backward compatibility, no migrations, no legacy shims, no
dead code, no dead data. Delete historical debt — do not carry it forward.

## Source of truth

Read `docs/database-design-v2.md` in full before writing any code. Every
section is normative except where this prompt explicitly overrides it. If the
doc and this prompt conflict, this prompt wins — report the conflict instead
of silently choosing.

## Hard mandates (non-negotiable)

1. **No backward compatibility.** No v1→v2 adapter, no dual-write, no compat
traits, no versioned API shims, no "legacy" modules kept for reference at
runtime.
2. **No migrations.** Bump `CURRENT_SCHEMA_VERSION`; older databases are
rejected. No migration tool, no import path, no data conversion. v1 data is
discarded.
3. **No dead code.** Delete every now-unused v1 component: event machinery,
projection/watermark, old store traits/impls/adapters, old caches, old
queries, dead exports, orphaned dependencies. Zero compiler warnings.
4. **No dead data.** Fresh schema only; nothing backfilled; no column/table
that v2 does not use.
5. **Sealed facade.** External code talks ONLY to `SessionStore` (14.1). No
raw SQL outside `PersistenceEngine`. The v1 bare-SQL bypass class must be
structurally impossible, not merely discouraged.
6. **No event concept anywhere.** No event log, no projection, no watermark,
no replay. Parts are the truth. (`NotificationBus` emits `SessionChange`
live-update notifications — that is UI notification, not a data log.)
7. **One session state.** `SessionState` is derived from parts + leases (17.3);
`sessions` stores only identity/lineage/`config_json`/`provider_anchors_json`.

## Final decisions (all settled — do not reopen)

| # | Decision |
|---|----------|
| D1 | fork child transcript renders shared prefix turns/content |
| D2 | in-flight streaming parts at fork: share by reference |
| D3 | fresh DB only; no migration code (decided) |
| D4 | ordering = `ORDER BY created_at_ms, part_id`; no seq |
| D5 | `config_json` = execution config only (permission ceiling, capability denials, workspace root override, selection/access defaults); compaction policy from agena GLOBAL settings, not stored per session |
| D6 | `runs` table dropped; run = `kind='run'` marker part |
| D7 | no `metadata` column; only `provider_state` on the run marker |
| D8 | `provider_anchors_json` persists (resume is BLOCKING on it) |
| D9 | message_count = number of run markers |
| D10 | streaming throttle: flush on N deltas or run end; memory bus is live |
| D11 | global runtime history dropped (no `list_events`) |
| D12 | `skill_ref` = reference-only `{skill, args}`; resolve on demand |
| D13 | `file_ref` extended shape (url/data_url/file_id/base64/media dims) |

## Schema (exact DDL in doc sections 4-5)

9 tables: `parts`, `session_parts`, `sessions`, `execution_leases`,
`sequences`, `workspaces`, `permission_rules`, `usage`, `idempotency`.

- `parts`: part_id (global sequence), kind (open set), role
(user/assistant/system/tool/runtime), state
(pending/in_progress/completed/failed/cancelled), content (JSON), summary,
visibility (`both|user|ai`), rendered_markdown (nullable), parent_part_id,
run_id, origin_session_id, revision, started_at_ms / finished_at_ms /
created_at_ms / updated_at_ms, provider_state (nullable; run marker only).
No metadata column.
- `session_parts`: PK (session_id, part_id) + added_at_ms; NO seq column;
ordering = `parts.created_at_ms, parts.part_id`.
- `sessions`: identity/lineage (parent_id/depth/root_id/workspace_id/title/
version/lifecycle_state/cutoff_part_id/task_id/subtask_*/config_json/
provider_anchors_json).
- `usage`: normalized scalar columns, integer micro-dollars, append-only;
indexes (workspace×time, session×time, provider×model×time); one row per
model call.
- Triggers/invariants: doc section 6. ID allocation: `sequences` (5.3).
Leases: 5.2, 7.2.

## Operations (doc section 7)

User send; crash recovery / lease steal (atomic terminate of residual markers
in the same transaction); fork/rewind (eager edge copy of
`(created_at_ms, part_id) <= cutoff`); read transcript (one query);
interaction (ask user / plan review); delete + refcount-guarded orphan GC.

## Concurrency (doc section 8, esp. 8.5 — four invariants)

1. Lease = write ownership. 2. Lease steal terminates residual markers
atomically. 3. Shared parts are read-only/append-only for non-creating
sessions; divergence = append a new part, never mutate a shared row.
4. GC safety via refcount guard (v1 had none). Cross-process catch-up by
version + (created_at_ms, part_id).

## Facade & internals (doc sections 14-15)

- Public: `SessionStore` trait — load, submit_user_message, append_parts,
update_part, complete_run, answer_interaction, fork, rewind, delete,
subscribe, **list_session_summaries, list_session_tree, rename, cancel_run,
compact_session, export_session_jsonl, import_session_jsonl, usage_stats,
session_state** (full list in 14.1).
- Internals (invisible to callers): `PersistenceEngine` (sole owner of
DatabaseConnection / sea_orm), `MemoryLayer` (LRU + streaming buffer),
`NotificationBus` (`SessionChange`; commit-then-notify). In-memory backend
for tests. Callers cannot distinguish memory from DB.

## State machine (doc 17) + errors/retries/visibility/rendering (doc 18)

- Single `SessionState` {Creating, Ready, Running, AwaitingUser, Interrupted,
Failed} derived from parts + leases; pending interaction has priority.
- Resume (17.4): in-flight marker + pending interaction → AwaitingUser, no
abort; fresh lease → Running; stale lease → reconcile → failed
(process_restart) + child parts cancelled → Ready.
- Part lifecycle includes retry `failed → in_progress` (run markers excluded:
a failed run = a new continue run). Error parts are durable, never deleted.
- Visibility: AI prompt receives only both/ai; UI renders only both/user.
- `rendered_markdown` = human view (plugin/tool rendered); AI gets raw
`content`.
- User message = 1 text part (with `[[ref:part_id]]` placeholders) + N ref
parts (`file_ref` / `skill_ref` / `paste_ref`). Chat DB never stores blobs;
paste_ref inlines text, file_ref stores path/name/mime/sha (extended per
D13).

## Usage (doc 16)

Normalized scalar columns; integer micro-dollars
(total_cost_micros/recorded_cost_micros); cost_estimate_incomplete flag;
detail_json nullable; workspace_id denormalized; queries are pure SQL over
index ranges.

## Implementation phases (in order; commit after each phase)

- **P0 Recon & delete.** Inventory v1 persistence/execution/query code.
Delete: event/projection/watermark machinery, old store traits/impls/
adapters, old caches, old schema/migration files, dead deps. Commit
`chore(db): delete v1 legacy layer`.
- **P1 Schema.** v2 DDL + sequences + leases + triggers/invariants + schema
version bump; enforce fresh-DB-only (reject older version).
- **P2 Engines.** `PersistenceEngine` (sea_orm) + in-memory engine for tests.
- **P3 Facade.** `SessionStore` + `MemoryLayer` + `NotificationBus` + dual
backend wiring + cross-process catch-up.
- **P4 Execution engine rewire.** runs→markers; streaming write policy (D10);
interactions; retries (18.2); compaction as a part; hooks/subagents;
cancel/steer/continue; crash resume (17.4).
- **P5 Query/UI surfaces.** session list/tree; export/import JSONL;
rename/cancel/compact; rendering contract (18.4); visibility filters
(18.3); usage_stats.
- **P6 Dead-code sweep.** Grep legacy identifiers (below) → zero hits;
remove leftovers; clippy `-D warnings` clean.
- **P7 Tests & perf.** Full suite per gates below; benchmarks for
read/usage/streaming.

## Verification gates (ALL must pass)

1. Grep for legacy identifiers → **zero hits**: `agena_events`,
`agena_turns`, `agena_assistant_replies`, `agena_reply_executions`,
`agena_content_nodes`, `agena_model_messages`, `agena_model_message_parts`,
`agena_session_messages`, `agena_model_projection_states`,
`agena_session_sequences`, `runtime_state_json`, `event_query_service`,
`ListEvents`, `list_events`, `projection`, `watermark`, old `SessionCache`,
`content_node`, `reply_execution`.
2. No migration code or files in the repo; fresh DB init only.
3. `cargo build --workspace` clean; `cargo test` green; clippy `-D warnings`
clean.
4. v1 storage/execution test coverage re-expressed against `SessionStore`;
no event/projection tests remain.
5. Concurrency tests: lease steal atomic terminate; fork during streaming
(shared parts read-only); GC refcount guard; cross-process catch-up.
6. Resume tests: kill at every state (mid-stream, mid-ask, mid-tool) →
reopen → correct `SessionState` per 17.4.
7. Retry tests: `failed → in_progress` part update; error part persists next
to success part.
8. Usage perf: query shapes use index ranges (EXPLAIN); streaming writes
amortized per D10.
9. Export/import JSONL round-trip test.
10. No bare SQL outside `PersistenceEngine` (structural audit).

## Working constraints

- Work in the repo/branch the user gives you. The files `claude-reverse/`,
`demo.md`, `demo.txt`, `hello.txt`, `sample.txt` and the pre-existing
modified files in the working tree are NOT yours — leave them untouched.
- Follow any project instruction files (AGENT.md / AGENA.md / CLAUDE.md).
- Commit per phase with `scope(db):` messages; keep commits reviewable.
- Never bypass runtime permission decisions; write only inside the permitted
workspace.

## Definition of done

All 10 gates pass; the legacy layer is absent; the v2 store/engines/facade
are live behind the sealed boundary; the execution engine runs on parts;
tests are green; performance is validated; the commit history tells the
phase-by-phase story.
