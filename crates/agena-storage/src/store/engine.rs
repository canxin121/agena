//! The persistence engine contract — the internal data boundary behind the
//! sealed `SessionStore` facade.
//!
//! Two backends implement this trait behind one engine type: `SqliteEngine`
//! (production, sole owner of the `DatabaseConnection` and every raw SQL
//! statement) and `InMemoryEngine` (tests / small deployments). Callers cannot
//! distinguish memory from DB. See `crates/agena-storage-sqlite/src/engine.rs`
//! and `crates/agena-storage/src/store/in_memory.rs`.

use async_trait::async_trait;

use super::{
    LeaseAcquire, NewPart, NewSession, Part, PartDelta, ReconcileOutcome, RunOutcome,
    SessionListQuery, SessionMeta, SessionSummary, SessionView, StoreError, SubmitOutcome,
    UsageQuery, UsageRecord, UsageStats,
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

    /// Load a session's metadata plus its parts ordered by
    /// `(created_at_ms, part_id)` — one membership JOIN.
    async fn load_session(&self, session_id: i64) -> Result<SessionView, StoreError>;

    /// The newest member part's `(created_at_ms, part_id)` cursor, if the
    /// session has any parts. Used by the facade's memory layer for
    /// cross-process catch-up: `sessions.version` catches session-meta writes,
    /// and this cursor catches part additions (14.4). Both are compared on
    /// cache hit; a change in either invalidates the cached view.
    async fn newest_member_cursor(&self, session_id: i64)
        -> Result<Option<(i64, i64)>, StoreError>;

    /// Rename a session (bumps `version`).
    async fn rename_session(
        &self,
        session_id: i64,
        title: String,
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

    /// List session summaries, newest first (section 14.1).
    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError>;

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
    async fn current_lease(&self, session_id: i64) -> Result<Option<super::LeaseState>, StoreError>;

    /// Delete leases whose heartbeat is stale and return their session ids.
    async fn reap_stale_leases(&self, stale_before_ms: i64) -> Result<Vec<i64>, StoreError>;

    // --- writes (all require the session lease) ---

    /// User send (7.1): create the run marker + content parts + membership
    /// edges + optional idempotency row in one transaction. The marker's
    /// `run_kind` is taken from `content.run_kind`.
    async fn submit_user_message(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError>;

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

    /// Cancel a run marker and its non-terminal child parts (17.5 user cancel).
    async fn cancel_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        now_ms: i64,
    ) -> Result<(), StoreError>;

    /// Answer a pending interaction: mark it completed and append a user reply
    /// part bound to the interaction (17.5).
    async fn answer_interaction(
        &self,
        session_id: i64,
        owner_id: &str,
        interaction_part_id: i64,
        reply: NewPart,
        now_ms: i64,
    ) -> Result<Part, StoreError>;

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
    async fn reconcile(
        &self,
        session_id: i64,
        now_ms: i64,
    ) -> Result<ReconcileOutcome, StoreError>;

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
