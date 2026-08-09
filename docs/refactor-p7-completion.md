# Agena DB v2 refactor — P7 completion evidence

Date: 2026-08-09  
Branch: `research/db-design-audit`  
Authoritative requirements: `docs/refactor-prompt.md` and
`docs/database-design-v2.md`

## Outcome

The v2 parts-first persistence/execution refactor is complete through P7.
Persisted chat history is session membership over ordered parts; run markers
are parts; session state is derived from parts plus leases; old event-history,
projection-state, and runtime snapshot persistence paths are absent. External
chat-data consumers use `agena_storage::store::SessionStore`.

The original P7 pass fixed two correctness defects found during verification:

- Reconciliation-on-open now preserves both `Running` and `AwaitingUser`.
  A pending interaction wins even when the owning run has no lease, so opening
  a deliberately paused session cannot abort its run or cancel the interaction.
- A manager fork request that names a message marker now resolves that marker
  to the message's final member part. Both the default full-history path and an
  explicit message-marker path therefore include all content in that message.

D10 is implemented as a real bounded write policy. `content_text_delta`
updates are overlaid from `MemoryLayer`, committed every eight deltas by
default, and flushed at run completion/cancellation. Ordinary semantic
checkpoints remain commit-synchronous. The threshold is configurable for
deterministic tests and benchmarks.

A stricter post-P7 requirement audit then found and closed four additional
contract gaps before this document was finalized:

- `sessions.version` now advances once for every committed part/membership
  mutation in both backends, while idempotency replay remains version-neutral.
  Updating a shared prefix part advances every member session, so a cached fork
  cannot retain a stale marker/content row.
- Mutation outcomes now carry the exact committed rows. Interaction answers
  emit the completed interaction before the added reply; cancel, reconcile,
  and stale-lease steal emit every changed marker/child before metadata;
  user-cancel correctly terminalizes its marker as `cancelled` rather than
  `failed`.
- Subscription handles own unique observer IDs. Dropping one session/global
  subscription removes only that observer. Delete emits real nonzero
  `PartRemoved` patches for every removed membership (including descendants),
  fork/rewind and import announce the new session, and lease reaping announces
  affected metadata so consumers refresh derived state.
- The cross-process boundary is explicit and tested: live callbacks are
  process-local best effort, while reconnect uses the ordered parts snapshot
  and monotonic version. In-place updates whose newest-member cursor does not
  move, plus updates to a shared fork marker, both invalidate another facade's
  cache through version changes.

## Verification commands

All of the following completed successfully in this worktree after the final
code changes:

```text
cargo fmt --all --check
cargo test -p agena-storage --lib                         # 47 passed
cargo test -p agena-storage-sqlite --lib                  # 43 passed
cargo test -p agena-runtime-session --lib                 # 106 passed
cargo test -p agena-application --lib                     # 17 passed
cargo test -p agena-api-server --lib                      # 8 passed
cargo test --workspace                                    # all unit/integration/trybuild/doc tests passed
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p agena-storage-sqlite --bench v2_store --no-run
cargo bench -p agena-storage-sqlite --bench v2_store
git diff --check
```

## Ten-gate audit

### Gate 1 — deleted persistence identifiers

This code-scope audit returns no matches:

```sh
rg -n 'agena_events|agena_turns|agena_assistant_replies|agena_reply_executions|agena_content_nodes|agena_model_messages|agena_model_message_parts|agena_session_messages|agena_model_projection_states|agena_session_sequences|runtime_state_json|event_query_service|ListEvents|list_events|event_projection|list_session_events|content_node|reply_execution|\bSessionCache\b' \
  crates apps tools packages examples scripts --glob '*.rs'
```

The prompt's two unqualified English words `projection` and `watermark` are
not useful literal repo-wide checks: this repository legitimately contains
display/model/failure projections, while notification SSE uses a transient
timestamp cursor named `watermark`. Neither is the deleted persisted chat
projection/watermark subsystem. The exact old subsystem identifiers above are
zero, and Gate 10 separately proves that no chat-table SQL escaped the engine.

### Gate 2 — fresh schema only

- `CURRENT_SCHEMA_VERSION` is 5.
- Version 0 creates the complete schema in one DDL transaction.
- Any nonzero version other than 5 is rejected without mutation.
- There is no DB conversion, version-to-version migration, backfill, or dual
  schema path.
- `fresh_database_is_initialized_at_current_version`,
  `incompatible_older_database_is_rejected_without_mutation`, and
  `newer_database_is_rejected` pass.
- `v2_schema_initializes_with_only_nine_chat_tables` asserts the exact complete
  Agena-owned table set, including the nine chat tables and only the retained
  model-catalog/scheduler infrastructure tables.

Other uses of “migration” in the repository refer to unrelated Git worktree or
MCP credential workflows, not the chat database.

### Gate 3 — build, test, and lint

- Workspace build passed.
- Full workspace tests passed, including trybuild and doctests.
- Full all-targets clippy passed with warnings denied.
- Formatting and whitespace checks passed.

### Gate 4 — v1 coverage re-expressed on parts/facade

The obsolete manager event/table test module was removed in P6. Current
manager regressions use the sealed facade and assert:

- create/reload through `SessionStore`;
- run-marker plus ordered-part message shape;
- exact-part checkpoint revisions;
- shared-prefix fork rendering;
- full default manager fork behavior;
- manager JSONL import as an independent root;
- query projection derived from persisted parts;
- derived state from marker plus lease;
- open-session recovery behavior for fresh and awaiting-user runs.

No persisted event/projection test remains.

### Gate 5 — concurrency

Direct evidence:

- `lease_steal_aborts_stale_run_atomically`
- `lease_steal_aborts_stale_run_across_processes`
- `fork_during_streaming_shares_parent_updates_and_child_diverges_by_append`
- `fork_during_streaming_shares_completion_and_rejects_child_mutation`
- `gc_deletes_only_refcount_orphans`
- `facade_cross_process_cache_invalidation`
- `updating_a_shared_part_bumps_every_member_session_version`
- `fork_cannot_answer_a_shared_interaction_in_place`
- `sqlite_fork_cannot_answer_a_shared_interaction_in_place`
- `every_part_mutation_bumps_version_but_idempotency_replay_does_not`
- `every_sqlite_part_mutation_bumps_version_but_idempotency_replay_does_not`
- `second_process_reads_committed_parts`
- `two_os_processes_share_one_database_without_locking`

The fork tests prove the complete D2/D8.4 sequence: fork while a parent-origin
part is in progress, observe its parent-written completion from the child,
reject a child in-place update as shared/read-only, then allow child divergence
by appending a child-origin part under its independent lease.

The cross-process facade test covers two distinct invalidation mechanisms. An
append moves both the version and newest-member cursor. A subsequent in-place
update leaves the cursor unchanged but still reloads through `sessions.version`;
completing a marker shared with a cached fork also advances the child version.
The REST `GET /sessions/{id}/parts` and session-change stream's initial
`session_snapshot` expose that full ordered snapshot plus version for reconnect.

### Gate 6 — resume at every state

Direct evidence:

- Mid-stream: `resume_mid_stream_without_a_lease_reconciles_to_ready` derives
  `Interrupted`, reconciles the marker to failed with
  `abort_reason=process_restart`, cancels the streamed child, then derives
  `Ready`.
- Mid-ask: `resume_mid_ask_without_a_lease_remains_awaiting_user`, the
  in-memory `state_derivation_covers_all_sessions_states`, and manager test
  `open_session_preserves_a_run_paused_for_user_input_without_a_lease` prove
  that the pending interaction remains pending and the marker stays in flight.
- Mid-tool: `resume_mid_tool_preserves_error_context_and_cancels_the_tool`
  cancels the nonterminal tool, fails its run marker with `process_restart`,
  and preserves the durable diagnostic error part.
- Fresh lease: `open_session_leaves_another_process_fresh_run_intact` proves
  manager open returns `Running` without reconciliation.
- Facade guard/patch contract:
  `reconcile_emits_committed_part_updates_and_preserves_live_or_paused_runs`
  proves direct reconciliation is a no-op for both a fresh running session and
  an awaiting-user session, while an interrupted run emits exact terminal rows.

### Gate 7 — retry and durable errors

Both backends cover the lifecycle transition and complete history:

- `retry_transitions_failed_to_in_progress_with_revision_bump_but_not_for_runs`
- `retry_history_retains_failed_error_and_successful_result_parts`
- `retry_history_keeps_the_durable_error_beside_the_successful_result`

The complete-history tests fail a `tool_call`, append a durable failed `error`
child, retry the same operation part with a revision bump and cleared terminal
timestamp, append a successful `tool_result`, finish the operation/run, reload,
and assert that both the error and success remain with remapped parent links.
Failed run markers remain terminal; a run retry is a new continue marker.

### Gate 8 — usage plans and streaming amortization

`usage_query_shapes_use_their_covering_range_indexes` executes
`EXPLAIN QUERY PLAN` over production-shaped aggregates and asserts:

| Query shape | Required selected index |
|---|---|
| session + time range | `idx_agena_usage_session` |
| workspace + time range | `idx_agena_usage_ws_time` |
| provider + model + time range | `idx_agena_usage_provider_model` |

`text_stream_deltas_are_amortized_and_run_end_flushes_the_tail` uses a flush
threshold of three and proves:

- two deltas are visible to same-process facade reads but do not change the
  persisted part revision;
- the third delta produces one durable update;
- two more deltas remain buffered;
- run completion flushes that tail;
- five deltas produce two content-part persistence updates/notifications.

The execution manager additionally bounds provider/tool streamed content in
memory and writes terminal snapshots rather than cumulative per-chunk bodies.

P7 also includes the executable production-SQLite benchmark target
`crates/agena-storage-sqlite/benches/v2_store.rs` (`harness = false`). It uses
the real `SqliteEngine` behind `SessionFacade`, asserts the D10 revision count,
and measures all three prompt-mandated paths. The final run on this worktree
reported:

| Benchmark | Dataset / operation | Result |
|---|---|---:|
| `read/warm_facade` | cached 257-part view with persisted-position validation | 225,611 ns/op |
| `usage/indexed_workspace_range` | indexed aggregate over a 1,500-row range | 946,805 ns/op |
| `streaming/65_deltas_plus_completion` | threshold 8, including run-end tail flush | 21,039,540 ns/op |

These numbers are evidence from one machine rather than stable pass/fail
thresholds; correctness is enforced by the benchmark assertions and the
`EXPLAIN QUERY PLAN`/amortization tests above.

### Gate 9 — JSONL round trip

JSONL tests exist at in-memory engine, SQLite engine, facade, and manager
boundaries. The production SQLite round trip explicitly proves:

- all imported part IDs are fresh;
- `run_id` and `parent_part_id` chains are remapped consistently;
- run-marker `provider_state` and session provider anchors are restored;
- the import is an independent depth-0 root with no parent or cutoff;
- canonical `(created_at_ms, part_id)` ordering and content/state are
  preserved.

### Gate 10 — sealed SQL boundary

This table-name audit:

```sh
rg -l 'agena_(parts|session_parts|sessions|execution_leases|sequences|usage|idempotency)' \
  crates apps tools packages examples scripts --glob '*.rs'
```

returns only:

- `agena-storage-sqlite` engine/schema/invariant/lifecycle/transaction and
  concurrency/engine test modules;
- one `agena-storage` in-memory-backend comment naming the corresponding
  SQLite trigger.

No manager/application/API/TUI/CLI module issues chat SQL. `SessionManager`
accepts a `DatabaseConnection` only at composition time to build the sealed
facade and retained permission/workspace infrastructure repositories; it does
not retain or query chat tables. The concrete facade engine accessor is
test-only and crate-private. Production chat-data access is through
`Arc<dyn SessionStore>`.

## Phase history

- `a1430cc3 feat(db): move query and UI surfaces onto parts (P5)`
- `ac8b1512 refactor(db): remove final legacy data paths (P6)`
- P7: `test(db): verify v2 concurrency recovery and performance (P7)`
- Post-P7 closure: `fix(db): complete v2 versioning and live change contracts`
- Post-P7 evidence: `test(db): add v2 store benchmarks and final audit evidence`

The protected paths `claude-reverse/`, `demo.md`, `demo.txt`, `hello.txt`, and
`sample.txt` were not modified.
