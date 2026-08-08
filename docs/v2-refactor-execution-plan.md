# v2 Refactor — Execution Plan (work in progress)

Source of truth: `docs/database-design-v2.md` + `docs/refactor-prompt.md` (prompt wins on conflict).
This file records the concrete per-crate decisions made while executing, so every phase
is traceable. Update it as the refactor lands.

## Final target architecture

```
External callers (TUI / Web / CLI / API / tests)
   |  depend only on facade trait + pure domain types
   v
SessionStore trait (agena-storage, 14.1)  <-- ONLY public entry
   |  implemented by SessionFacade (agena-storage): MemoryLayer + NotificationBus + engine
   |  internal: PersistenceEngine trait (agena-storage) — backend-neutral ops
   v
engines: SqliteEngine (agena-storage-sqlite, sole sea_orm owner)
         InMemoryEngine (agena-storage, no sea_orm)
```

- No event concept anywhere. `NotificationBus` emits `SessionChange` (UI notification only, never persisted).
- No projection, no watermark. Parts are the truth; `sessions.version`/`(created_at_ms, part_id)` for catch-up.
- No bare SQL outside `PersistenceEngine`. `DatabaseConnection` never exported.

## Crate-by-crate disposition

### crates/agena-storage (contract + facade; no sea_orm)
- KEEP: `StorageConfig`, `MemoryRepository`/`MemoryStore`, `WorkspaceRepository`,
  `PermissionRuleRepository`(+`PermissionRuleTransactionWriter`), `ModelCatalogRepository`,
  `TransactionEffects`, `MemoryType`/`MemoryFrontmatter`/etc.
- NEW (v2): `Part`, `PartKind`, `PartRole`, `PartState`, `PartVisibility`, `SessionMeta`,
  `SessionView`, `NewPart`, `SessionChange`, `SessionState`, `SessionPresentation`,
  `SessionSummary`, `SessionListQuery`, `UsageStats`/`UsageQuery`, `SessionStore` trait (14.1),
  internal `PersistenceEngine` trait, `InMemoryEngine`, `MemoryLayer`, `NotificationBus`,
  `SessionFacade` (composition), `SessionStoreError`.
- DELETE: `EventStore`/`StoreRange`/`ReverseStoreRange`/`EventStoreError`,
  `SequenceAllocator`/`InMemorySequenceAllocator` (v1),
  `ModelMessageRepository`/`ModelMessageTransactionWriter`/records,
  `ProjectionLookupRepository`, `SessionStatsRepository`/`SessionEventStats`,
  `UsageRepository`/`UsageSample`/`UsageRecord`,
  `SessionSummaryRepository`/`SessionMutationRepository`/records,
  `MessageIdAllocator`/`SequentialIdAllocator` (v1).

### crates/agena-storage-sqlite (engine; sole sea_orm owner for chat DB)
- REWRITE: `schema.rs` → v2 DDL (9 tables + indexes + seeds); `schema_lifecycle.rs` →
  bump `CURRENT_SCHEMA_VERSION` (4 → 5), fresh-DB-only; `schema_invariants.rs` → v2 triggers.
- NEW: `engine.rs` → `SqliteEngine` impl of `PersistenceEngine`.
- KEEP: `transaction.rs` (write-lock sentinel now on `sequences`), `workspace_repository.rs`,
  `permission_rule_repository.rs`, `model_catalog_repository.rs`.
- DELETE: `event_store.rs`, `model_message_repository.rs`, `projection_lookup_repository.rs`,
  `sequence_allocator.rs`, `stored_values.rs`.
- REWORK: `session_summary_repository.rs`/`session_stats_repository.rs`/`usage_repository.rs`
  onto v2 tables (or fold into engine; likely folded — engine is the only repo owner now).

### crates/agena-runtime-session-core (model + core db access)
- KEEP: leases (`db/leases.rs`), `model.rs` reworked (drop `SessionRuntimeState` blob →
  v2 derives state from parts), `PartContent` envelope (from runtime-contracts).
- DELETE: `db/entities/model_message.rs`, `model_message_part.rs`, `model_projection_state.rs`,
  `event_entity.rs`.
- REWORK: `db/crud/session.rs` + `entities/session.rs`/`session_lineage.rs` → v2 `sessions`
  (drop `runtime_state_json`, fold lineage in, add root_id/depth/task/subtask/config_json/
  provider_anchors_json).

### crates/agena-runtime-session (execution engine; KEEP-REWIRE)
- DELETE: `session/history/` (mod, event, transcript, run_buffer, store/*) — v1 projection engine.
- DELETE: `session/store/` v1 concrete store (core, fork, history, ids, event_rewrite, helpers,
  types, store, activity_v2, workspace seed) — replaced by facade.
- DELETE: `event/` (kind, publisher, client, bus, bridge, error), `event_bridge.rs`,
  `event_publish_service.rs`, `event_query_service.rs` — replaced by `SessionChange` bus.
- KEEP (stable contracts): `session_execution_service.rs`, `session_execution_control.rs`,
  `session_tool_execution.rs`, `session_requests.rs`, `task_control.rs`, `periodic.rs`,
  `session_maintenance.rs`, `usage_stats.rs` (aggregate), `execution_registry.rs`.
- KEEP-REWIRE: `session/manager/*` (mod, sessions, runs, replies/*, compact, history→rewired,
  helpers, permission_service, session_prompt, stats) onto the new facade.
- KEEP-REWIRE: `session/processor/*` (run, events, tool_calls) — streaming writes via facade
  append_parts/update_part/complete_run (throttled per D10).
- REWORK: `session_cache.rs`/`session/cache.rs`/`session_cache_policy.rs` → MemoryLayer.
- KEEP: `activity/` (activity-v2 model), `presentation_event.rs` (rewired to part patches).

### crates/agena-runtime (composition facade; re-export surface)
- Re-export `agena_storage::SessionStore` (or runtime-session facade). Rework
  `RuntimeEventQueryService`/`RuntimeEventStreamService`/`RuntimePresentationEvent` →
  part-patch surface. Keep `SessionQueryService`, `SessionExecutionCommandService`,
  `SessionToolExecutionService`, `SessionPluginCommandService`, `SessionExecutionControl`.
- `connect_runtime_database` → `initialize_schema` (v2) stays; wire facade in composition.

### Outer consumers (P5)
- `agena-tui-backend/src/backend_session.rs`: replace `list_events`/`list_timeline_events_before`
  + `subscribe_presentation_events` with facade `load`/`subscribe(SessionChange)` part patches.
- `agena-api-server`: `rest/events.rs` dropped (D11); `sse.rs` → version/(created_at,part_id)
  catch-up; `rest/sessions.rs` rewire.
- `agena-application`: `dispatch/queries.rs` drop `ListEvents`; `event_projection.rs` rework;
  `application.rs` facade wiring.
- `agena-api`/`agena-client`: drop `ListEvents`/`list_events`.
- `agena-cli`: rewire session list.
- `agena-tui-app`/`agena-tui-session`: transcript state consumes part patches.

## Phases (commits in this order)
- P0 `chore(db): delete v1 legacy layer` — delete the v1 lists above (build intentionally
  broken until P3–P5 rebuild; compiler = rewire worklist).
- P1 `feat(db): v2 schema` — 9 tables + sequences + leases + triggers + version bump 5,
  fresh-DB-only enforcement.
- P2 `feat(db): persistence engines` — SqliteEngine + InMemoryEngine behind `PersistenceEngine`.
- P3 `feat(db): SessionStore facade` — trait + MemoryLayer + NotificationBus + SessionFacade
  + dual-backend wiring; data layer compiles.
- P4 `feat(db): execution engine on parts` — manager/processor onto facade; streaming (D10);
  interactions; retries (18.2); compaction as part; crash resume (17.4).
- P5 `feat(db): query/UI surfaces on parts` — session list/tree, export/import JSONL,
  rename/cancel/compact, rendering (18.4), visibility (18.3), usage_stats; rewire
  api-server/tui-backend/application/client.
- P6 `refactor(db): dead-code sweep` — grep legacy identifiers → zero chat-data hits;
  clippy `-D warnings` clean.
- P7 `test(db): v2 concurrency/resume/retry/usage tests + benchmarks` — all 10 gates.

## Verification gates checklist (10)
1. grep legacy identifiers → zero chat-data hits
2. no migration code/files; fresh DB init only
3. cargo build --workspace clean; cargo test green; clippy -D warnings clean
4. v1 storage/execution tests re-expressed against SessionStore; no event/projection tests
5. concurrency: lease steal atomic terminate; fork-during-streaming; GC refcount; cross-process catch-up
6. resume: kill at every state → reopen → correct SessionState per 17.4
7. retry: failed→in_progress part update; error part persists next to success
8. usage perf: index-range queries (EXPLAIN); streaming amortized per D10
9. export/import JSONL round-trip
10. no bare SQL outside PersistenceEngine (structural audit)
