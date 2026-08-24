//! The persistence engine contract — the internal data boundary behind the
//! sealed `SessionStore` facade.
//!
//! Two backends implement this trait behind one engine type: `SqliteEngine`
//! (production, sole owner of the `DatabaseConnection` and every raw SQL
//! statement) and `InMemoryEngine` (tests / small deployments). Callers cannot
//! distinguish memory from DB. See `crates/agena-storage-sqlite/src/engine.rs`
//! and `crates/agena-storage/src/store/in_memory.rs`.

use std::collections::HashMap;

use async_trait::async_trait;

use super::{
    BackgroundDelivery, BackgroundEventRequest, BackgroundOperation, BackgroundOperationTransition,
    BackgroundSettleOutcome, LeaseAcquire, NewBackgroundOperation, NewPart, NewSession, Part,
    PartCursor, PartDelta, PartState, ReconcileOutcome, RunOutcome, SessionListQuery, SessionMeta,
    SessionMetadataPatch, SessionPartPage, SessionState, SessionSummary, SessionView, StoreError,
    SubmitOutcome, UsageQuery, UsageRecord, UsageStats,
};

/// A live-update notification derived from an operation and emitted after
/// commit. Never persisted, never replayed — it is observer notification, not
/// an event log (14.3).
#[derive(Debug, Clone, PartialEq)]
pub enum SessionChange {
    PartAdded {
        session_id: i64,
        part: Part,
    },
    /// Streaming delta: a part was updated (revision bumped, state/content
    /// changed).
    PartUpdated {
        session_id: i64,
        part: Part,
    },
    PartRemoved {
        session_id: i64,
        part_id: i64,
    },
    SessionMetaUpdated {
        session_id: i64,
        meta: SessionMeta,
    },
}

/// Outcome of the maintenance loop: leases reaped and orphan parts GC'd.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintenanceOutcome {
    /// Session ids whose leases were stale and were reaped.
    pub reaped_sessions: Vec<i64>,
    /// Orphan parts deleted (refcount-guarded, 7.6).
    pub gc_deleted_parts: usize,
}

/// The internal persistence boundary. The facade composes with either backend
/// at runtime; external code never talks to this trait directly.
#[async_trait]
pub trait PersistenceEngine: Send + Sync {
    // --- sessions ---

    /// Create a session row and return its metadata.
    async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError>;

    /// Load a session's metadata.
    async fn session_meta(&self, session_id: i64) -> Result<SessionMeta, StoreError>;

    /// Count durable user-send run markers without materializing the
    /// transcript. This is used by the chat history navigator to show the
    /// exact session size before older pages are loaded.
    async fn user_message_count(&self, session_id: i64) -> Result<u64, StoreError>;

    /// Load a session's metadata plus all parts ordered by
    /// `(created_at_ms, part_id)` — one membership JOIN.
    async fn load_session(&self, session_id: i64) -> Result<SessionView, StoreError>;

    /// Load one newest-first keyset page of session parts. `before` excludes
    /// that position and is interpreted against the canonical
    /// `(created_at_ms, part_id)` ordering. The backend fetches one extra row
    /// to populate `has_more` without counting the complete transcript.
    async fn load_session_page(
        &self,
        session_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError>;

    /// Load one newest-first keyset page of content parts owned by a single
    /// run. Run markers are not returned; callers use this for expanding a
    /// folded transcript activity sequence without scanning or transferring
    /// the rest of the session.
    async fn load_run_page(
        &self,
        session_id: i64,
        run_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError>;

    /// The newest member part's `(created_at_ms, part_id)` cursor, if the
    /// session has any parts. Used by the facade's memory layer for
    /// cross-process catch-up. `sessions.version` advances for every
    /// session-visible mutation (including shared-part updates); this cursor is
    /// an additional membership-position check (14.4). A change in either
    /// invalidates the cached view.
    async fn newest_member_cursor(&self, session_id: i64)
    -> Result<Option<(i64, i64)>, StoreError>;

    /// Rename a session (bumps `version`).
    async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<SessionMeta, StoreError>;

    /// Atomically update user-editable session metadata (bumps `version`
    /// exactly once, regardless of how many fields are present).
    async fn update_session_metadata(
        &self,
        session_id: i64,
        patch: SessionMetadataPatch,
    ) -> Result<SessionMeta, StoreError>;

    /// Persist `provider_anchors_json` (resume is blocking on it, D8).
    async fn set_provider_anchors(
        &self,
        session_id: i64,
        anchors: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// Persist `config_json` (execution config only, D5).
    async fn set_config_json(
        &self,
        session_id: i64,
        config: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// Find a subagent session by its unique `(parent_id, task_id)` pair
    /// (schema `uq_agena_session_parent_task`). Used to resume a delegated
    /// subtask instead of creating a duplicate child.
    async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionMeta>, StoreError>;

    /// Create a subagent session: `relation_kind = subagent`, parented under
    /// `parent_session_id`, `task_id` recorded on the row. The unique
    /// `(parent_id, task_id)` pair makes create idempotent at the schema level;
    /// callers check `find_subagent_by_task_id` first so a conflicting create
    /// is an unexpected error.
    async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
        now_ms: i64,
    ) -> Result<SessionMeta, StoreError>;

    /// Update a session's subtask columns (status / started / finished /
    /// failure), bumping `version`. Returns the fresh metadata.
    async fn update_subtask_state(
        &self,
        session_id: i64,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// List session summaries, newest first (section 14.1).
    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError>;

    /// Derive processing states for a set of sessions in one backend read.
    /// Missing ids are omitted from the returned map.
    async fn session_states(
        &self,
        session_ids: &[i64],
        now_ms: i64,
    ) -> Result<HashMap<i64, SessionState>, StoreError>;

    /// Fetch one session's summary row, or `None` when it does not exist.
    /// A cheap single-row projection of [`Self::list_session_summaries`]
    /// (13.1) for existence/lifecycle/version checks on the application path.
    async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, StoreError>;

    /// Session counts per workspace (13.5 `workspace_counts`), for the
    /// workspaces listing surface. `0` is returned for a workspace with no
    /// sessions.
    async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, StoreError>;

    /// List every session in one root's subtree, newest first.
    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError>;

    /// Delete a session (membership edges cascade; shared parts survive and
    /// are GC'd by reference count, 7.6).
    async fn delete_session(&self, session_id: i64) -> Result<(), StoreError>;

    // --- leases (cross-process single writer) ---

    /// Try to acquire the session lease. Acquiring a stale lease aborts the
    /// session's stale in-flight run markers atomically in the same
    /// transaction (invariants 1-2, 7.2).
    async fn try_acquire_lease(
        &self,
        session_id: i64,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<LeaseAcquire, StoreError>;

    /// Refresh the lease heartbeat; `false` when this caller no longer owns it.
    async fn heartbeat_lease(
        &self,
        session_id: i64,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError>;

    /// Release the lease when this caller owns it.
    async fn release_lease(&self, session_id: i64, owner_id: &str) -> Result<bool, StoreError>;

    /// Current lease row, if any.
    async fn current_lease(&self, session_id: i64)
    -> Result<Option<super::LeaseState>, StoreError>;

    /// Delete leases whose heartbeat is stale and return their session ids.
    async fn reap_stale_leases(&self, stale_before_ms: i64) -> Result<Vec<i64>, StoreError>;

    // --- durable background-operation aggregate ---

    /// Create the idempotent launch intent before starting the external side
    /// effect. Replays for the same `(session, tool_part)` return the existing
    /// aggregate.
    async fn create_background_operation(
        &self,
        operation: NewBackgroundOperation,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError>;

    /// Load one aggregate by its stable internal id.
    async fn background_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError>;

    /// Resolve a runtime completion signal without an in-memory correlation
    /// index. The unique `(kind, external_id)` index is authoritative.
    async fn background_operation_by_external_id(
        &self,
        kind: super::BackgroundOperationKind,
        external_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError>;

    /// List non-terminal aggregates in deterministic creation order. This is
    /// the durable reconciliation source after restart; observer buses and
    /// runtime registries are only latency optimizations.
    async fn active_background_operations(
        &self,
        kind: Option<super::BackgroundOperationKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundOperation>, StoreError>;

    /// Advance one aggregate through the validated state machine with an
    /// optimistic revision check.
    async fn transition_background_operation(
        &self,
        transition: BackgroundOperationTransition,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError>;

    /// Atomically record a unique operation event, append its notification to
    /// the assistant run that launched it, and enqueue the durable delivery.
    /// Operations without an assistant launch run (scheduled/external ingress)
    /// keep an explicit chronological Runtime run. The store serializes
    /// aggregate mutation internally; callback callers do not perform an
    /// optimistic read/write pair that could lose concurrent monitor events.
    async fn record_background_event(
        &self,
        request: BackgroundEventRequest,
        now_ms: i64,
    ) -> Result<BackgroundSettleOutcome, StoreError>;

    /// Claim a pending or expired delivery. Returns `None` when another live
    /// owner holds the claim or the delivery is already consumed.
    async fn claim_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        claim_until_ms: i64,
        now_ms: i64,
    ) -> Result<Option<BackgroundDelivery>, StoreError>;

    /// Mark a claimed delivery consumed after the model wake completes.
    async fn consume_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError>;

    /// Release a failed claim back to pending with a durable diagnostic and a
    /// durable next-attempt deadline so a restart or later dispatcher pass can
    /// retry it without a hot loop.
    async fn retry_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: serde_json::Value,
        next_attempt_at_ms: i64,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError>;

    /// Terminalize one claimed delivery after a non-retryable or exhausted
    /// wake failure. This is idempotent after another cancellation/recovery
    /// path has already terminalized the same row.
    async fn fail_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: serde_json::Value,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError>;

    /// Suppress queued notification wakes for a session when the user cancels
    /// its current execution. Pending and claimed rows are terminalized
    /// together so a delivery racing with cancellation cannot relaunch the
    /// session after the execution is gone.
    async fn fail_pending_background_deliveries(
        &self,
        session_id: i64,
        error: serde_json::Value,
        now_ms: i64,
    ) -> Result<usize, StoreError>;

    /// Pending or expired deliveries, oldest first, for restart recovery.
    async fn pending_background_deliveries(
        &self,
        limit: usize,
        now_ms: i64,
    ) -> Result<Vec<BackgroundDelivery>, StoreError>;

    // --- writes (all require the session lease) ---

    /// User send (7.1): create the run marker + content parts + membership
    /// edges + optional idempotency row in one transaction. The marker's
    /// `run_kind` is taken from `content.run_kind`. When every submitted input
    /// part is already terminal, the marker is terminal too: a user input is a
    /// committed external event, not an execution-liveness guard.
    async fn submit_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError>;

    /// User send variant that records the owning execution identity on a
    /// newly-created marker. The identity is deliberately not applied to an
    /// idempotency replay: a replay must remain owned by the execution that
    /// originally created the marker.
    ///
    async fn submit_user_run_for_execution(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        execution_id: &str,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError>;

    /// Atomically settle a background operation against the run that launched
    /// it (the agena analog of Claude Code's `<task-notification>` arriving on
    /// the launching turn). In one transaction the method:
    ///
    /// 1. refreshes the lease — a stale lease (any owner) is re-heartbeated so
    ///    the transaction may write. Other in-flight runs are deliberately
    ///    **not** aborted: the settle targets one specific launching run and
    ///    must never destroy a *different* run that a live execution is still
    ///    driving (aborting unrelated in-flight runs is `try_acquire_lease`'s
    ///    job when a new execution genuinely takes over);
    /// 2. transitions the launching tool part (InProgress for the atomic
    ///    launch checkpoint, terminal when the operation settles);
    /// 3. appends the companion parts (`new_parts`, preserving their supplied
    ///    roles) under the launching run — **no new run marker**;
    /// 4. terminalizes the launching run marker (Completed) once no in-flight
    ///    child remains, so the session returns to Ready instead of lingering
    ///    in Interrupted.
    ///
    /// `tool_part` is `Some((part_id, next_state, content))` when the launching
    /// tool part must be checkpointed or terminalized. Returns the created
    /// parts (a launch guard or settled notification rows).
    async fn settle_background_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        tool_part: Option<(i64, PartState, serde_json::Value)>,
        new_parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError>;

    /// Append content parts to an existing run (streaming).
    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError>;

    /// Apply a streaming delta to one part (revision++, updated_at bump).
    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
        now_ms: i64,
    ) -> Result<Part, StoreError>;

    /// Finish a run marker with the given outcome.
    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
        now_ms: i64,
    ) -> Result<Part, StoreError>;

    /// Start a fresh run without user input (`continue`, `compaction`,
    /// `background`, `steer`). Creates a run marker with the given `run_kind`.
    async fn start_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_kind: &str,
        content: serde_json::Value,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError>;

    /// Cancel a run marker and its non-terminal child parts (17.5 user cancel),
    /// returning every committed row that changed.
    async fn cancel_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError>;

    /// Withdraw a newly submitted user run from one session projection. The
    /// underlying part rows remain available for orphan GC and shared fork
    /// memberships; only this session's membership and idempotency claim are
    /// removed.
    async fn withdraw_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError>;

    // --- fork / rewind (eager edge copy, 7.3) ---

    async fn fork_session(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
        rewind: bool,
        now_ms: i64,
    ) -> Result<SessionMeta, StoreError>;

    // --- recovery / maintenance ---

    /// Reconcile a session whose in-flight run has no fresh lease (17.4 step
    /// 2c): mark in-flight run markers failed (`process_restart`) and their
    /// non-terminal children cancelled. Idempotent.
    async fn reconcile(&self, session_id: i64, now_ms: i64)
    -> Result<ReconcileOutcome, StoreError>;

    /// Reap stale leases and GC orphan parts (7.6). Refcount-guarded: a part
    /// is deleted only when it has zero membership AND (no run reference OR its
    /// run is terminal).
    async fn maintenance(&self, now_ms: i64) -> Result<MaintenanceOutcome, StoreError>;

    // --- usage ---

    /// Append one provider-call usage record (append-only, never updated).
    async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError>;

    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError>;

    // --- export / import ---

    /// Serialize the full session (meta + ordered parts) as JSONL: one JSON
    /// object per line, `{"meta": ...}` first, then `{"part": ...}` per part.
    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError>;

    /// Import a JSONL bundle (as exported) into a fresh session under
    /// `workspace_id`, returning the new session id.
    async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
        now_ms: i64,
    ) -> Result<i64, StoreError>;
}
