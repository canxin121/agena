# Agena Database Design & API Distribution Audit

Audit branch: `research/db-design-audit` (worktree `.agena/worktrees/db-design-audit`)
Base commit: `b7575a6c` (master)
Method: read-only source research (greps + targeted reads + 4 delegated read-only sub-audits). No source changes made.

## 1. Database design overview

Agena persists state in one SQLite database (`agena.db`), opened through `agena-runtime`'s connection bootstrap and shared by every process (TUI, server, CLI). The file is resolved by `agena_storage::StorageConfig`:

- `AGENA_DATABASE_URL` -> `AGENA_DATABASE_PATH` -> default `$HOME/agena/agena.db` (`crates/agena-storage/src/lib.rs:64-102`).
- Connection hardening: WAL, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=15s`, pool max 16 (`crates/agena-runtime/src/tracing_config.rs:53-85`).
- Schema owned by `agena-storage-sqlite`, applied by `initialize_schema` (`crates/agena-storage-sqlite/src/schema.rs:92-133`), versioned via SQLite `PRAGMA user_version`; `CURRENT_SCHEMA_VERSION = 4` (`crates/agena-storage-sqlite/src/schema_lifecycle.rs:25`).

### 1.1 Tables (schema.rs:152-174)

| Table | Purpose |
|---|---|
| `agena_workspaces` | workspace identity (path unique) |
| `agena_sessions` | session tree (parent/depth/root, lifecycle_state, runtime_state_json) |
| `agena_session_lineage` | 1:1 provenance: relation_kind, source cutoff, task/subtask state |
| `agena_permission_rules` | permission rules with revocation columns |
| `agena_events` | append-only event log: event_uuid, seq_global (both UNIQUE), seq_session, payload_json |
| `agena_turns` | canonical turn identity (UNIQUE(session_id, turn_seq)) |
| `agena_assistant_replies` | reply identity/status/revision |
| `agena_reply_executions` | execution identity/status per reply |
| `agena_content_nodes` | transcript nodes (text or activity, shape CHECKs) |
| `agena_model_messages` | materialized message projection (INTEGER ids from sequence allocator) |
| `agena_model_message_parts` | materialized part projection (activity_id/segment_id exclusive) |
| `agena_session_messages` | fork/branch membership join |
| `agena_model_projection_states` | per-session projection watermark (last_seq_global) |
| `agena_model_catalog_entries` / `agena_model_catalog_state` | model catalog cache |
| `agena_scheduler_jobs` / `agena_scheduler_history` | scheduler queue/history |
| `agena_sequences` | global sequence counters (+ `__agena_write_lock__` sentinel) |
| `agena_session_sequences` | per-session sequence counters |
| `agena_execution_leases` | cross-process execution lease (owner/heartbeat) |
| `agena_user_message_idempotency` | user-message dedup |

Seeds: `seq_global`, `message_id`, `part_id` start at 1 (`schema.rs:148-150`).

### 1.2 Schema lifecycle and invariants

- Versioning history: v1 sequences/leases/idempotency; v2 `content_nodes.title`; v3 `agena_session_messages`; v4 `lineage.view_materialized_seq_global` (`schema_lifecycle.rs:8-24`). No incremental migration path: a DB whose `user_version` differs from 4 is rejected with a hard error (`schema.rs:127-132`). Schema evolves in place with `IF NOT EXISTS`.
- Invariant triggers (`schema_invariants.rs:11-275`): session hierarchy immutability, lifecycle transitions, lineage/task uniqueness, turn/reply shape and revision monotonicity, message/part identity immutability, content-node owner/lifecycle checks + cascade cleanup, events append-only and scope-ownership.
- Sequencing: atomic `INSERT ... ON CONFLICT ... RETURNING next_val-1` on `agena_sequences`/`agena_session_sequences` (`sequence_allocator.rs:33-132`); `seq_session` seeds from `MAX(seq_session)+1`.
- Multi-process: schema creation serialized by a `<db>.schema-lock` filesystem advisory lock (`schema.rs:13-83`); every write transaction starts with an `INSERT OR IGNORE` write-lock sentinel so the busy timeout applies at lock acquisition (`transaction.rs:22-50`); busy retry with exponential backoff (100ms to 1.6s, at most 5 retries, `transaction.rs:59-160`); concurrency tests include a real child OS process sharing one DB (`concurrency_tests.rs:296-348`).

## 2. Unified storage API distribution

Two crates define the sanctioned database surface:

- `agena-storage` — backend-neutral trait contracts, no DB dependency (`crates/agena-storage/src/lib.rs`):
  - `EventStore<K>` — append/range/watermarks (:1133)
  - `SequenceAllocator` — seq_global/seq_session/message_id/part_id + block reserve/seed (:97)
  - `MemoryRepository` / `MemoryDir` — filesystem memory docs (:402, :1073)
  - `WorkspaceRepository` — create/update/delete/get/list/lookup/ensure (:423)
  - `ModelCatalogRepository` — read/write cache (:504)
  - `PermissionRuleRepository` + `PermissionRuleTransactionWriter` (:554, :564)
  - `SessionStatsRepository` (:630), `UsageRepository` (:676), `ProjectionLookupRepository` (:696), `SessionSummaryRepository` (:916), `SessionMutationRepository` (:962)
  - `ModelMessageRepository` + `ModelMessageTransactionWriter` (:754, :840)
- `agena-storage-sqlite` — concrete SeaORM adapters implementing the contracts (`SeaEventStore`, `SeaWorkspaceRepository`, `SeaPermissionRuleRepository`, `SeaSessionStatsRepository`, `SeaUsageRepository`, `SeaProjectionLookupRepository`, `SeaSessionSummaryRepository`, `SeaModelMessageRepository`, `SeaModelCatalogRepository`, `SqliteSequenceAllocator`; `crates/agena-storage-sqlite/src/lib.rs`).

Consumers that correctly use the unified API: `agena-application` (service layer, trait-only), `agena-runtime` (wiring + `SeaModelCatalogRepository`), `agena-runtime-session` (event store / sequence / permission / workspace / stats / usage / summary / projection repos, see section 3), `agena-bundled-plugins` (MemoryRepository), `agena-memory-index` (MemoryDir), `agena-cli` (env passthrough only).

## 3. Components that touch the database directly (bypass findings)

### 3.1 `agena-runtime-session` + `agena-runtime-session-core` — raw SQL on the shared connection

The session runtime holds the same `sea_orm::DatabaseConnection` that the `Sea*` repositories use (`session/manager/mod.rs:1234-1299`, `SessionStore::new(db)` at :1271) and executes raw SQL / legacy entity CRUD outside the storage traits for a large part of the write path:

| Table | Write sites (production) | Unified trait? |
|---|---|---|
| `agena_sessions` | `db/crud/session.rs:416,423,559,572,645,667` (entity + raw UPDATE) | partial: `SessionMutationRepository` covers only create/rename/delete with (workspace_id, parent_id, title); runtime uses legacy CRUD for lineage/version/lifecycle/subtask (`store/core.rs:215`) |
| `agena_session_lineage` | `db/crud/session.rs:442,629`; `store/fork.rs:186` | none |
| `agena_execution_leases` | `db/leases.rs:70,124,145,190` (via `execution_registry.rs:217-291`) | none |
| `agena_content_nodes` | `store/activity_v2.rs:47,121,154,194-316`; `store/history.rs:318`; `history/store/mod.rs:966,1249,1317,1571,1632,1705` | none |
| `agena_turns` / `agena_assistant_replies` / `agena_reply_executions` | `history/store/mod.rs:869,883,907,1004,1020,1033,1117,1140,1158` | none |
| `agena_session_messages` | `store/core.rs:997-1010` (insert_session_message_memberships), `store/fork.rs:172` | none (only an internal helper in `agena-storage-sqlite::SeaModelMessageRepository`) |
| `agena_model_projection_states` | `history/store/mod.rs:677` (fence) + unified watermark writer | partial: `ModelMessageTransactionWriter::upsert_projection_watermark_in_transaction` |
| `agena_model_messages` / `agena_model_message_parts` | mixed: unified `SeaModelMessageRepository` writer (`history/store/mod.rs:1852-2006`) but raw SQL in `store/activity_v2.rs` / `manager/history.rs` paths | partial — duplicate write paths |

Additionally `agena-runtime-session-core/src/db/entities/*` (model_message, model_message_part, model_projection_state, permission_rule, session_lineage, session, workspace) form a second SeaORM entity layer over the same tables owned by `agena-storage-sqlite` — they duplicate table knowledge and bypass the repository boundary.

### 3.2 `agena-scheduler` — raw SQL on its own dedicated database

`crates/agena-scheduler/src/store.rs` executes raw `sea_orm::Statement` SQL on `agena_scheduler_jobs` / `agena_scheduler_history` (upsert, delete, select, update, history insert/prune). Since the audit this component no longer shares the chat database: the scheduler owns a dedicated SQLite database (`~/.agena/scheduler.db` by default, overridable via `AGENA_SCHEDULER_DATABASE_URL`/`AGENA_SCHEDULER_DATABASE_PATH`) whose schema and `PRAGMA user_version` live in `crates/agena-scheduler/src/schema.rs`. The runtime opens it beside the chat database (`tracing_config.rs::connect_scheduler_database`), threads the connection to the scheduler (`runtime/snapshot/builders.rs::build_scheduler`), and degrades to the in-memory store when no scheduler database is configured (e.g. in-memory chat databases in tests). There is still no storage trait for scheduler jobs — raw SQL is the only path — but it is now isolated to a database the chat stack never touches.

### 3.3 `apps/agena` HTTP server — a completely separate sqlx database

`apps/agena/src/server/persistence/db.rs` opens its own `sqlx::SqlitePool` (`ServerStateDb::open`, :36-51) on a file also named `agena.db` (`paths.rs:8`) with its own private `initialize_schema` (:118-157):

- `server_kv` (settings, terminal registry, preview registry — KV API, no raw SQL outside the pool)
- `attachment_cache_blob_store`, `attachment_cache_source_index` (upserts at `attachment/cache.rs:74-98`)

Default path differs from the storage layer (`~/.config/agena/agena.db` / `$AGENA_SERVER_DATA_DIR` vs `~/agena/agena.db`), so by default the two pools do not collide. But the file name is the same and neither side coordinates: if `AGENA_DATABASE_PATH`/`AGENA_DATABASE_URL` is set to the server data dir (or `AGENA_SERVER_DATA_DIR` points at the runtime database), two independent pools + two independent schemas + zero shared versioning operate on one file (split-brain). The server schema has no versioning at all. `server/app.rs:76-92` bootstraps the runtime DB first, then opens `ServerStateDb`.

### 3.4 `agena-runtime` — sanctioned wiring layer that also holds the raw connection

`tracing_config.rs:101-136` is the only place that opens the pool and calls `initialize_schema`; `process_state.rs:18` and `runtime/snapshot/mod.rs:53,170-212` carry `Arc<DatabaseConnection>` as plumbing. This is legitimate composition, but the raw connection is then handed to services, which enables the raw-SQL bypasses in 3.1/3.2.

### 3.5 Clean components (no DB access)

`agena-plugin-sdk`, `agena-plugin-host`, `agena-mcp-server`, `agena-tool`, `agena-marketplace-server`, `agena-keyring-store`, all `agena-tui*` crates, `agena-web`, `packages/agena-web-ui`, `packages/agena-vscode` — no sqlite/sea-orm usage. `tools/agena-e2e` and `agena-api-server` use `sqlite::memory:` only in tests. No component deletes/renames/copies the DB file (no filesystem manipulation found).

## 4. Summary of bypass risk

1. Runtime session write path bypasses the unified API on the majority of tables: content nodes, turns, replies, executions, leases, session membership, and session-row mutations have no storage trait — they are raw SQL in `agena-runtime-session`/`agena-runtime-session-core`. Model messages/parts/projection watermark have a trait but also raw SQL paths, so two writers exist for the same rows.
2. Scheduler queue is raw SQL with no trait, but it now runs against a dedicated scheduler database, so it is no longer an invasive interface on the chat database.
3. The app server is a second, unversioned database with the same file name; a path misconfiguration creates split-brain on one file.
4. Schema versioning guards only the storage file: the server DB and any raw-SQL writer are invisible to `CURRENT_SCHEMA_VERSION`.

## 5. Recommended follow-ups (not implemented in this audit)

- Promote the runtime-session raw-SQL write paths (content nodes, turns, replies, executions, leases, membership, session CRUD) into `agena-storage` traits with `agena-storage-sqlite` adapters, or at minimum route them through the existing `ModelMessage*`/`Session*` adapters.
- The scheduler now owns a dedicated database (`agena-scheduler::schema`), which resolves the original "scheduler in the shared DB" concern. A storage trait for scheduler jobs remains optional; the remaining exposed surface is the app-server split-brain risk above.
- Align the app-server DB: either keep `server_kv`/attachment cache on the unified storage API + schema versioning, or guard against both pools resolving to the same file (reject/resolve the conflict at bootstrap).
- Consider one schema owner + incremental migrations instead of version-mismatch hard errors, so the server DB can share versioning.
