//! The parts-first data boundary for the session manager.
//!
//! The sealed [`agena_storage::SessionStore`] facade owns ids, leases, and
//! transactions; parts are the only chat entity; there is no event log and no
//! live `EventKind` plumbing.
//!
//! This module is the manager's thin adapter over that facade. It:
//!
//! - converts a [`SessionView`] (metadata + parts) back into the
//!   [`Session`] aggregate the execution engine operates on
//!   (parts are grouped into runs by their `run` marker);
//! - converts [`TypedContent`] payloads to and from the JSON stored on
//!   `parts.content`;
//! - translates every manager write (submit, append, update, run lifecycle,
//!   interaction, fork, rewind, compaction) into facade calls, remapping
//!   engine-allocated part ids onto the in-memory aggregate.
//!
//! There is deliberately no database handle, no raw SQL, and no event
//! concept in this file — the facade is the only data surface (design 14.3,
//! 15.2).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use agena_domain::{ExecutionAccess, ExecutionSelection, ExecutionStatus, ReasoningPart, Role};
use agena_plugin_sdk::attachment::{AttachmentPart, AttachmentSource};
use agena_runtime_contracts::part_content;
use agena_runtime_contracts::part_content::TypedContent;
use agena_storage::store::{
    BackgroundDelivery, BackgroundEventRequest, BackgroundOperation, BackgroundOperationKind,
    BackgroundOperationTransition, BackgroundSettleOutcome, NewBackgroundOperation, NewPart, Part,
    PartDelta, PartRole, PartState, PartVisibility, SessionMeta, SessionStore, SessionView,
    StoreError, SubmitOutcome, UsageQuery,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;
use crate::part::{OperationPart, SkillReferencePart};
use crate::session::Session;

/// The facade-backed store adapter used by [`crate::SessionManager`].
///
/// `owner_id` is the same process-wide execution identity passed to
/// [`agena_storage::SessionFacade::new`]; every write routes through the
/// facade's lease validation so the manager never touches leases itself.
///
/// The engine is the only id source, so freshly built in-memory parts carry a
/// negative placeholder until the facade returns the real id. The adapter
/// remaps placeholders to engine ids on every write and rewrites the
/// in-memory aggregate so part references stay consistent.
#[derive(Clone)]
pub(crate) struct StoreAdapter {
    pub(crate) facade: Arc<dyn SessionStore>,
    pub(crate) owner_id: String,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl StoreAdapter {
    pub(crate) fn new(
        facade: Arc<dyn SessionStore>,
        owner_id: String,
        now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            facade,
            owner_id,
            now_ms,
        }
    }

    pub(crate) fn now_ms(&self) -> i64 {
        (self.now_ms)()
    }

    pub(crate) fn background_owner_id(&self) -> &str {
        &self.owner_id
    }

    /// Load a session's transcript and metadata and rebuild the in-memory
    /// aggregate the execution engine operates on.
    pub(crate) async fn load_session(&self, session_id: i64) -> Result<Session, AppError> {
        let view = self.facade.load(session_id).await.map_err(store_error)?;
        session_from_view(view)
    }

    /// Create a new session row and return the rebuilt aggregate.
    pub(crate) async fn create_session(
        &self,
        workspace_id: i64,
        parent_id: Option<i64>,
        relation_kind: agena_domain::SessionRelationKind,
        cutoff_part_id: Option<i64>,
        title: String,
        task_id: Option<String>,
        config_json: Option<Value>,
    ) -> Result<Session, AppError> {
        let meta = self
            .facade
            .create_session(agena_storage::store::NewSession {
                workspace_id,
                parent_id,
                relation_kind,
                cutoff_part_id,
                title,
                task_id,
                config_json,
                provider_anchors_json: None,
            })
            .await
            .map_err(store_error)?;
        self.session_from_meta(meta)
    }

    pub(crate) async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<i64>, AppError> {
        Ok(self
            .facade
            .find_subagent_by_task_id(parent_session_id, task_id)
            .await
            .map_err(store_error)?
            .map(|meta| meta.id))
    }

    pub(crate) async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
    ) -> Result<i64, AppError> {
        self.facade
            .create_subagent_session(parent_session_id, task_id, title)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn update_subtask_state(
        &self,
        mut session: Session,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<Value>,
    ) -> Result<Session, AppError> {
        let session_id = session.id;
        let meta = self
            .facade
            .update_subtask_state(session_id, status, started_at_ms, finished_at_ms, failure)
            .await
            .map_err(store_error)?;
        session.version = meta.version;
        session.updated_at = timestamp_millis_to_utc(meta.updated_at_ms)?;
        session.runtime.subtask = crate::session::SubtaskRuntimeState {
            status: meta
                .subtask_status
                .as_deref()
                .and_then(agena_domain::SubtaskStatus::parse)
                .unwrap_or_default(),
            started_at_ms: meta.subtask_started_at_ms,
            finished_at_ms: meta.subtask_finished_at_ms,
            failure: subtask_failure_from_value(meta.subtask_failure.as_ref())?,
        };
        session.refresh_derived();
        Ok(session)
    }

    /// Persist the D5 execution configuration (selection/access defaults,
    /// permission ceiling, capability denials, workspace root override) to
    /// `sessions.config_json`. The in-memory aggregate is returned unchanged
    /// except for the version/updated_at the write bumped. Empty configs are
    /// written as `NULL` so a session with no overrides stays a blank slate.
    pub(crate) async fn persist_execution_config(
        &self,
        mut session: Session,
    ) -> Result<Session, AppError> {
        let config: PersistedExecutionConfig =
            PersistedExecutionConfig::from(&session.runtime.execution);
        let value = (!config.is_empty())
            .then(|| serde_json::to_value(config))
            .transpose()
            .map_err(|error| AppError::Internal(format!("serialize execution config: {error}")))?;
        let meta = self
            .facade
            .set_config_json(session.id, value)
            .await
            .map_err(store_error)?;
        session.version = meta.version;
        session.updated_at = timestamp_millis_to_utc(meta.updated_at_ms)?;
        Ok(session)
    }

    pub(crate) async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<Session, AppError> {
        self.facade
            .rename(session_id, title)
            .await
            .map_err(store_error)?;
        self.load_session(session_id).await
    }

    /// Submit a user message: creates the `run` marker and its content parts
    /// in one transaction. The facade returns the full outcome — the run
    /// marker plus the committed, engine-id'd parts — so callers remap
    /// placeholders without reloading.
    pub(crate) async fn submit_user_run(
        &self,
        session_id: i64,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, AppError> {
        self.facade
            .submit_user_run(session_id, &self.owner_id, parts, idempotency_key)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn submit_user_run_for_execution(
        &self,
        session_id: i64,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        execution_id: &str,
    ) -> Result<SubmitOutcome, AppError> {
        self.facade
            .submit_user_run_for_execution(
                session_id,
                &self.owner_id,
                parts,
                idempotency_key,
                execution_id,
            )
            .await
            .map_err(store_error)
    }

    pub(crate) async fn create_background_operation(
        &self,
        operation: NewBackgroundOperation,
    ) -> Result<BackgroundOperation, AppError> {
        self.facade
            .create_background_operation(operation)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn background_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<BackgroundOperation>, AppError> {
        self.facade
            .background_operation(operation_id)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn background_operation_by_external_id(
        &self,
        kind: BackgroundOperationKind,
        external_id: &str,
    ) -> Result<Option<BackgroundOperation>, AppError> {
        self.facade
            .background_operation_by_external_id(kind, external_id)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn active_background_operations(
        &self,
        kind: Option<BackgroundOperationKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundOperation>, AppError> {
        self.facade
            .active_background_operations(kind, limit)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn transition_background_operation(
        &self,
        transition: BackgroundOperationTransition,
    ) -> Result<BackgroundOperation, AppError> {
        self.facade
            .transition_background_operation(transition)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn record_background_event(
        &self,
        request: BackgroundEventRequest,
    ) -> Result<BackgroundSettleOutcome, AppError> {
        self.facade
            .record_background_event(request)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn claim_background_delivery(
        &self,
        delivery_id: &str,
        claim_until_ms: i64,
    ) -> Result<Option<BackgroundDelivery>, AppError> {
        self.facade
            .claim_background_delivery(delivery_id, &self.owner_id, claim_until_ms)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn consume_background_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<BackgroundDelivery, AppError> {
        self.facade
            .consume_background_delivery(delivery_id, &self.owner_id)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn retry_background_delivery(
        &self,
        delivery_id: &str,
        error: Value,
        next_attempt_at_ms: i64,
    ) -> Result<BackgroundDelivery, AppError> {
        self.facade
            .retry_background_delivery(delivery_id, &self.owner_id, error, next_attempt_at_ms)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn fail_background_delivery(
        &self,
        delivery_id: &str,
        error: Value,
    ) -> Result<BackgroundDelivery, AppError> {
        self.facade
            .fail_background_delivery(delivery_id, &self.owner_id, error)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn fail_pending_background_deliveries(
        &self,
        session_id: i64,
        error: Value,
    ) -> Result<usize, AppError> {
        self.facade
            .fail_pending_background_deliveries(session_id, error)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn pending_background_deliveries(
        &self,
        limit: usize,
    ) -> Result<Vec<BackgroundDelivery>, AppError> {
        self.facade
            .pending_background_deliveries(limit)
            .await
            .map_err(store_error)
    }

    /// Atomically checkpoint or settle a background operation against its
    /// launching run. Launch commits the InProgress tool part (including its
    /// durable correlation marker) together with the invisible guard result;
    /// settle terminalizes that part and appends notification parts. Both
    /// phases use the same transaction so a crash cannot leave only one half.
    pub(crate) async fn settle_background_run(
        &self,
        session_id: i64,
        run_id: i64,
        tool_part: Option<(i64, PartState, Value)>,
        parts: Vec<NewPart>,
    ) -> Result<Vec<Part>, AppError> {
        self.facade
            .settle_background_run(session_id, &self.owner_id, run_id, tool_part, parts)
            .await
            .map_err(store_error)
    }

    /// Start a non-user run (continue / compaction / subtask) and return the
    /// new run marker part id.
    pub(crate) async fn start_run(
        &self,
        session_id: i64,
        run_kind: &str,
        content: Value,
    ) -> Result<i64, AppError> {
        let outcome = self
            .facade
            .start_run(session_id, &self.owner_id, run_kind, content, None)
            .await
            .map_err(store_error)?;
        Ok(outcome.run_id)
    }

    /// Append content parts under an existing run marker. Returns the created
    /// parts (engine ids) so callers can rebuild in-memory messages.
    pub(crate) async fn append_parts(
        &self,
        session_id: i64,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<Vec<Part>, AppError> {
        self.facade
            .append_parts(session_id, &self.owner_id, run_id, parts)
            .await
            .map_err(store_error)
    }

    /// Append the safe, durable user projection of an execution failure below
    /// an in-flight run marker. The diagnostic source stays in tracing; this
    /// part carries only [`agena_failure::UserProblem`], which is suitable for
    /// transcript rendering and expansion.
    pub(crate) async fn append_failure_part(
        &self,
        session_id: i64,
        run_id: i64,
        failure: &agena_failure::Failure,
    ) -> Result<Part, AppError> {
        let problem = agena_failure::UserProblem::from(failure);
        let category = serde_json::to_value(problem.category)
            .expect("failure category is always JSON serializable")
            .as_str()
            .map(ToOwned::to_owned);
        let message = problem.user.fallback.clone();
        let mut extra = BTreeMap::new();
        extra.insert(
            "problem".to_owned(),
            serde_json::to_value(&problem).expect("user problem is always JSON serializable"),
        );
        let content = TypedContent::Error(part_content::ErrorContent {
            category,
            message,
            detail: None,
            extra,
        });
        let new_part =
            new_part_from_content("error", PartRole::Assistant, &content, PartState::Failed)?;
        self.append_parts(session_id, run_id, vec![new_part])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "append_parts returned no failure part for run {run_id}"
                ))
            })
    }

    /// Apply a streaming delta to one part and return the updated part.
    pub(crate) async fn update_part(
        &self,
        session_id: i64,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<Part, AppError> {
        self.facade
            .update_part(session_id, &self.owner_id, part_id, delta)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn complete_run(
        &self,
        session_id: i64,
        run_id: i64,
        outcome: agena_storage::store::RunOutcome,
    ) -> Result<(), AppError> {
        self.facade
            .complete_run(session_id, &self.owner_id, run_id, outcome)
            .await
            .map(|_| ())
            .map_err(store_error)
    }

    pub(crate) async fn cancel_run(&self, session_id: i64, run_id: i64) -> Result<(), AppError> {
        self.facade
            .cancel_run(session_id, &self.owner_id, run_id)
            .await
            .map(|_| ())
            .map_err(store_error)
    }

    pub(crate) async fn withdraw_user_run(
        &self,
        session_id: i64,
        run_id: i64,
    ) -> Result<Vec<agena_storage::store::Part>, AppError> {
        self.facade
            .withdraw_user_run(session_id, &self.owner_id, run_id)
            .await
            .map_err(store_error)
    }

    /// Reconcile a session whose in-flight run lost its lease: mark stale run
    /// markers failed and their non-terminal children cancelled (17.4).
    pub(crate) async fn reconcile(&self, session_id: i64) -> Result<(), AppError> {
        self.facade.reconcile(session_id).await.map_err(store_error)
    }

    /// Extend the session lease without touching any part. Long stable runs
    /// (a slow reasoning stream, a multi-second tool execution) can exceed the
    /// lease staleness window between commits; the stable-run loop's heartbeat
    /// task calls this so the run's ownership stays fresh. A `false` return
    /// means the lease is gone (stolen/released) — the next commit surfaces it
    /// authoritatively, so this is best-effort.
    pub(crate) async fn heartbeat_lease(&self, session_id: i64) -> bool {
        self.facade
            .heartbeat_lease(session_id, &self.owner_id)
            .await
            .unwrap_or(false)
    }

    pub(crate) async fn fork(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, AppError> {
        self.facade
            .fork(session_id, at_part_id, title)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn rewind(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, AppError> {
        self.facade
            .rewind(session_id, at_part_id, title)
            .await
            .map_err(store_error)
    }

    /// Start a compaction run; returns the compaction run marker part id. The
    /// checkpoint `summary` (and optional `window` description) are recorded
    /// on the marker's content (4.1.1 `CompactionContent`).
    pub(crate) async fn compact_session(
        &self,
        session_id: i64,
        summary: Option<String>,
        window: Option<String>,
    ) -> Result<i64, AppError> {
        self.facade
            .compact_session(session_id, &self.owner_id, summary, window)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.facade
            .export_session_jsonl(session_id)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
    ) -> Result<i64, AppError> {
        self.facade
            .import_session_jsonl(workspace_id, bundle)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn usage_stats(
        &self,
        workspace_id: i64,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, AppError> {
        let generated_at = Utc::now();
        // The facade query is scalar/exact (16.3); the domain query carries
        // multi-value filters that this bridge collapses to the first value.
        let storage_query = UsageQuery {
            workspace_id: Some(workspace_id),
            session_id: query.session_ids.first().copied(),
            provider_id: query.provider_ids.first().cloned(),
            model_id: query.model_ids.first().cloned(),
            after_ms: query.from.map(|value| value.timestamp_millis()),
            before_ms: query.to.map(|value| value.timestamp_millis()),
        };
        let stats = self
            .facade
            .usage_stats(storage_query)
            .await
            .map_err(store_error)?;
        Ok(domain_usage_stats_from_storage(stats, &query, generated_at))
    }

    /// Engine-owned maintenance through the sealed facade (14.2): reap stale
    /// leases and GC orphan parts. Idempotent, safe from any process.
    pub(crate) async fn maintenance(
        &self,
    ) -> Result<agena_storage::store::MaintenanceOutcome, AppError> {
        self.facade
            .maintenance(self.now_ms())
            .await
            .map_err(store_error)
    }

    /// List session rows for the workspace as the shared domain DTO. The
    /// facade pages by `(updated_at_ms, id)` cursor; the
    /// `SessionListRequest.offset` paging is emulated by skipping the first
    /// `offset` rows, and `include_subagents = false` drops subtask rows.
    pub(crate) async fn list_session_summaries(
        &self,
        workspace_id: i64,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<agena_domain::SessionSummary>, AppError> {
        let fetch_limit = request
            .limit
            .map(|limit| limit as i64 + request.offset as i64);
        let summaries = self
            .facade
            .list_session_summaries(agena_storage::store::SessionListQuery {
                workspace_id: Some(workspace_id),
                parent_id: request.parent_id,
                roots_only: request.roots_only,
                exclude_subagents: !request.include_subagents,
                search: request.search,
                limit: fetch_limit,
                before: None,
            })
            .await
            .map_err(store_error)?;
        summaries
            .into_iter()
            .skip(request.offset as usize)
            .filter(|summary| request.include_subagents || !summary.relation_kind.is_subagent())
            .map(domain_summary_from_storage)
            .collect()
    }

    /// Fetch one session's summary row as the shared domain DTO, or `None`.
    pub(crate) async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<agena_domain::SessionSummary>, AppError> {
        self.facade
            .get_session_summary(session_id)
            .await
            .map_err(store_error)?
            .map(domain_summary_from_storage)
            .transpose()
    }

    /// Session counts per workspace (13.5 `workspace_counts`).
    pub(crate) async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, AppError> {
        self.facade
            .session_counts_by_workspace(workspace_ids)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<agena_storage::store::SessionSummary>, AppError> {
        self.facade
            .list_session_tree(root_id)
            .await
            .map_err(store_error)
    }

    pub(crate) async fn session_state(
        &self,
        session_id: i64,
    ) -> Result<agena_storage::store::SessionPresentation, AppError> {
        self.facade
            .session_state(session_id)
            .await
            .map_err(store_error)
    }

    fn session_from_meta(&self, meta: SessionMeta) -> Result<Session, AppError> {
        let view = SessionView {
            meta,
            parts: Vec::new(),
        };
        session_from_view(view)
    }
}

/// Process-local, monotonic source of negative placeholder ids.
fn next_placeholder_id() -> i64 {
    use portable_atomic::AtomicI64;
    use std::sync::atomic::Ordering;
    static NEXT: AtomicI64 = AtomicI64::new(-1);
    NEXT.fetch_sub(1, Ordering::Relaxed)
}

/// Part-id allocator handed to the processor for a model run. The processor
/// emits parts with placeholder ids while streaming; the adapter remaps them
/// to engine ids when the parts are appended to the run.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessorPartIdAllocator;

impl ProcessorPartIdAllocator {
    pub(crate) async fn reserve(&self) -> Result<i64, AppError> {
        Ok(next_placeholder_id())
    }
}

fn store_error(error: StoreError) -> AppError {
    match error {
        StoreError::NotFound(message) => AppError::Internal(message),
        StoreError::LeaseNotHeld { session_id } => {
            AppError::Internal(format!("lease not held for session {session_id}"))
        }
        StoreError::LeaseHeldByOther {
            session_id,
            owner_id,
            ..
        } => AppError::Internal(format!(
            "session {session_id} lease held by another owner {owner_id}"
        )),
        StoreError::InvalidState(message)
        | StoreError::Constraint(message)
        | StoreError::Conflict(message)
        | StoreError::Serialization(message)
        | StoreError::Io(message)
        | StoreError::Database(message) => AppError::Internal(message),
        StoreError::Busy => AppError::Internal("database busy, retry".to_string()),
    }
}

/// Rebuild the execution-engine [`Session`] aggregate from a [`SessionView`]
/// (metadata + ordered parts).
///
/// The engine operates on the flat parts projection ([`Session::parts`],
/// design 14-15): this installs the ordered part list and recomputes the
/// derived state (pending operations, workflow, approx bytes) from it.
/// Consumers rebuild logical runs on demand through [`parts_into_runs`] and
/// the provider projectors.
pub(crate) fn session_from_view(view: SessionView) -> Result<Session, AppError> {
    let SessionView { meta, parts } = view;
    let mut session = Session::new(
        meta.id,
        meta.workspace_id,
        meta.title.clone(),
        timestamp_millis_to_utc(meta.created_at_ms)?,
    );
    session.parent_id = meta.parent_id;
    session.depth = meta.depth;
    session.root_id = meta.root_id;
    session.version = meta.version;
    session.relation_kind = meta.relation_kind;
    session.lifecycle_state = meta.lifecycle_state;
    session.source_cutoff_seq_global = meta.cutoff_part_id;
    session.task_id = meta.task_id.clone();
    session.updated_at = timestamp_millis_to_utc(meta.updated_at_ms)?;
    apply_meta_runtime(&mut session.runtime, &meta)?;
    session.bind_runtime_scope();
    session.install_projected_parts(parts);
    Ok(session)
}

/// Group a flat ordered parts slice into logical messages: each run marker
/// part starts a group that collects its content parts (in `(created_at_ms,
/// part_id)` order); bare content parts (no run) form singleton groups. Runs
/// are emitted in the order their markers appear in the (already ordered)
/// parts, with singletons appended after. The provider projectors
/// (`project_completion_input`, `project_persisted`) consume one group at a
/// time.
pub(crate) fn parts_into_runs(parts: &[Part]) -> Vec<Vec<Part>> {
    let mut run_ids: Vec<i64> = Vec::new();
    let mut marker_by_run: BTreeMap<i64, &Part> = BTreeMap::new();
    let mut content_by_run: BTreeMap<i64, Vec<Part>> = BTreeMap::new();
    let mut singleton: Vec<Part> = Vec::new();

    for part in parts {
        if part.is_run_marker() {
            if !marker_by_run.contains_key(&part.part_id) {
                run_ids.push(part.part_id);
            }
            marker_by_run.insert(part.part_id, part);
        } else if let Some(run_id) = part.run_id {
            content_by_run.entry(run_id).or_default().push(part.clone());
        } else {
            singleton.push(part.clone());
        }
    }

    let mut runs = Vec::with_capacity(run_ids.len().saturating_add(singleton.len()));
    for run_id in run_ids {
        let marker = marker_by_run
            .get(&run_id)
            .copied()
            .cloned()
            .expect("run marker registered before content grouping");
        let mut content = content_by_run.remove(&run_id).unwrap_or_default();
        content.sort_by_key(|part| (part.created_at_ms, part.part_id));
        let mut group = Vec::with_capacity(content.len().saturating_add(1));
        group.push(marker);
        group.extend(content);
        runs.push(group);
    }
    // A late Assistant-owned hook may be appended to its launching run after a
    // compaction checkpoint. The active-window slice then contains the new
    // child but not the old marker. Preserve such orphaned children as
    // chronological singleton inputs so the provider sees the notification;
    // the full transcript still has the marker and groups the same part under
    // the original assistant turn for presentation.
    for mut orphaned in content_by_run.into_values() {
        singleton.append(&mut orphaned);
    }
    singleton.sort_by_key(|part| (part.created_at_ms, part.part_id));
    runs.extend(singleton.into_iter().map(|part| vec![part]));
    runs
}

/// The key under which the adapter persists a part's provider `operation_id`
/// (the provider tool-call id used to correlate an invocation with the
/// ephemeral provider result emitted from that same `tool_call` part). The
/// parts schema has no column for it (design 4.1), so it
/// rides inside the rich `OperationPart.metadata` map — a reserved key the
/// engine never treats as its own. This is the adapter's private contract and
/// is invisible to everything that reads `parts.content` as the canonical
/// payload.
pub(crate) const OPERATION_ID_METADATA_KEY: &str = "agena.operation_id";

/// Serialize a decoded [`TypedContent`] into the canonical JSON payload stored
/// on `parts.content` (design 4.1.1). Every typed content shape serializes
/// itself via its `as_value` projection, so the bytes written here are
/// identical to what the typed model produces on every other write path.
pub(crate) fn typed_content_to_value(content: &TypedContent) -> Result<Value, AppError> {
    let value = match content {
        TypedContent::Run(part) => part.as_value(),
        TypedContent::Text(part) => part.as_value(),
        TypedContent::Think(part) => part.as_value(),
        TypedContent::ToolCall(part) => part.as_value(),
        TypedContent::FileRef(part) => part.as_value(),
        TypedContent::PasteRef(part) => {
            serde_json::to_value(part).expect("paste ref content is always JSON serializable")
        }
        TypedContent::SkillRef(part) => part.as_value(),
        TypedContent::Notice(part) => part.as_value(),
        TypedContent::Hook(part) => part.as_value(),
        TypedContent::SystemNotification(part) => part.as_value(),
        TypedContent::Compaction(part) => {
            serde_json::to_value(part).expect("compaction content is always JSON serializable")
        }
        TypedContent::Error(part) => part.as_value(),
    };
    Ok(value)
}

/// Build a [`NewPart`] from a decoded typed part payload.
///
/// `state` defaults to `pending`; content is the canonical JSON of the
/// [`TypedContent`]. `kind` defaults to the derived part kind when not given.
pub(crate) fn new_part_from_content(
    kind: impl Into<String>,
    role: PartRole,
    content: &TypedContent,
    state: PartState,
) -> Result<NewPart, AppError> {
    let value = typed_content_to_value(content)?;
    Ok(NewPart {
        kind: kind.into(),
        role,
        content: value,
        summary: part_summary(content),
        visibility: PartVisibility::Both,
        parent_part_id: None,
        state,
    })
}

fn part_summary(content: &TypedContent) -> Option<String> {
    match content {
        TypedContent::Text(text) => truncate(&text.text),
        TypedContent::Think(think) => truncate(&reasoning_from_think(think).preferred_text()),
        // Tool failures already live in `content.error`. Copying them into the
        // generic summary column would create a second durable representation
        // of the same fact.
        TypedContent::ToolCall(_) => None,
        TypedContent::Error(error) => {
            truncate(&part_content::user_problem_from_error(error).user.fallback)
        }
        TypedContent::SkillRef(reference) => {
            truncate(&part_content::skill_reference_from_skill_ref(reference).summary())
        }
        TypedContent::Hook(hook) => truncate(&hook.summary),
        TypedContent::Notice(notice) => truncate(&notice.summary),
        TypedContent::SystemNotification(notification) => truncate(&notification.summary),
        TypedContent::FileRef(attachment) => {
            let attachment = part_content::attachment_from_file_ref(attachment);
            if attachment.attachments.is_empty() {
                Some("0 attachment(s)".to_string())
            } else {
                truncate(&format!("{} attachment(s)", attachment.attachments.len()))
            }
        }
        TypedContent::Run(_) => None,
        TypedContent::PasteRef(paste) => truncate(&paste.text),
        TypedContent::Compaction(compaction) => {
            truncate(compaction.summary.as_deref().unwrap_or_default())
        }
    }
}

fn truncate(value: &str) -> Option<String> {
    const LIMIT: usize = 240;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    for ch in trimmed.chars().take(LIMIT) {
        out.push(ch);
    }
    if trimmed.chars().nth(LIMIT).is_some() {
        out.push('…');
    }
    Some(out)
}

/// Decode the canonical JSON payload stored on a part into its typed content
/// shape, dispatching on the part's `kind` column through the typed content
/// layer in `agena-runtime-contracts` ([`agena_runtime_contracts::part_content::decode`]).
/// Tool calls are strict: removed result/presentation keys and the removed
/// standalone `tool_result` kind fail instead of being migrated.
pub(crate) fn typed_content_from_value(
    kind: &str,
    value: &Value,
) -> Result<TypedContent, AppError> {
    agena_runtime_contracts::part_content::decode(kind, value)
        .map_err(|error| AppError::Internal(format!("decode part content from store: {error}")))
}

/// Project an [`OperationPart`] onto the strict canonical `tool_call` shape:
/// invocation/control facts plus one optional raw output. No model or human
/// presentation is serialized.
pub(crate) fn tool_call_from_operation(operation: &OperationPart) -> part_content::ToolCallContent {
    part_content::tool_call_from_operation(operation)
}

/// Project an [`AttachmentPart`] onto the canonical `file_ref` shape: the
/// first item's identity as named keys, the item's extended keys (`kind`,
/// `source`, media dimensions), and the full attachment list losslessly under
/// `extra["attachments"]` (covers multi-attachment parts).
pub(crate) fn file_ref_from_attachment(part: &AttachmentPart) -> part_content::FileRefContent {
    let mut extra = BTreeMap::new();
    if !part.attachments.is_empty() {
        extra.insert(
            "attachments".to_owned(),
            serde_json::to_value(&part.attachments)
                .expect("attachment list is always JSON serializable"),
        );
    }
    if let Some(first) = part.attachments.first() {
        extra.insert(
            "kind".to_owned(),
            Value::String(first.kind.as_ref().to_owned()),
        );
        extra.insert(
            "source".to_owned(),
            serde_json::to_value(&first.source)
                .expect("attachment source is always JSON serializable"),
        );
        if let Some(title) = &first.title {
            extra.insert("title".to_owned(), Value::String(title.clone()));
        }
        if let Some(size) = first.size_bytes {
            extra.insert("size_bytes".to_owned(), Value::from(size));
        }
        if let Some(width) = first.width {
            extra.insert("width".to_owned(), Value::from(width));
        }
        if let Some(height) = first.height {
            extra.insert("height".to_owned(), Value::from(height));
        }
        if let Some(duration) = first.duration_ms {
            extra.insert("duration_ms".to_owned(), Value::from(duration));
        }
        if let Some(pages) = first.page_count {
            extra.insert("page_count".to_owned(), Value::from(pages));
        }
    }
    let first = part.attachments.first();
    part_content::FileRefContent {
        path: first.and_then(|item| match &item.source {
            AttachmentSource::LocalPath { path } => Some(path.clone()),
            _ => None,
        }),
        name: first.and_then(|item| item.filename.clone()),
        mime: first
            .map(|item| item.mime.clone())
            .filter(|mime| !mime.is_empty()),
        sha: first.and_then(|item| item.sha256.clone()),
        extra,
    }
}

/// Project a [`SkillReferencePart`] onto the canonical `skill_ref` shape: the
/// first skill name as the named key, and the full snapshot losslessly under
/// `extra["skills"]` so the typed content remains lossless.
pub(crate) fn skill_ref_from_reference(part: &SkillReferencePart) -> part_content::SkillRefContent {
    let mut extra = BTreeMap::new();
    if !part.skills.is_empty() {
        extra.insert(
            "skills".to_owned(),
            serde_json::to_value(&part.skills).expect("skill snapshot is always JSON serializable"),
        );
    }
    part_content::SkillRefContent {
        skill: part.skills.first().map(|skill| skill.name.clone()),
        args: None,
        extra,
    }
}

/// Build the canonical `text` typed content for a plain text payload.
pub(crate) fn text_content(text: impl Into<String>) -> part_content::TextContent {
    part_content::TextContent {
        text: text.into(),
        synthetic: false,
        extra: BTreeMap::new(),
    }
}

/// The coarse [`agena_domain::PartKind`] of a typed payload: text is
/// `Text`, every other kind is `Activity`.
/// The plain text of a `TypedContent::Text` payload, if any.
pub(crate) fn typed_text(content: &TypedContent) -> Option<&str> {
    match content {
        TypedContent::Text(text) => Some(text.text.as_str()),
        _ => None,
    }
}

// ─── Typed-content projections ───────────────────────────────────────────────
//
// Runtime-facing domain values (`OperationPart`, `AttachmentPart`,
// `SkillReferencePart`, `ReasoningPart`, `UserProblem`) are reconstructed from
// canonical typed content through the extractor helpers in
// `agena_runtime_contracts::part_content`.

/// Rebuild a [`ReasoningPart`] from the canonical `think` shape.
pub(crate) fn reasoning_from_think(part: &part_content::ThinkContent) -> ReasoningPart {
    ReasoningPart {
        summary: part.summary.clone(),
        raw_content: part.raw.clone(),
        encrypted_content: part.encrypted_content.clone(),
    }
}

pub(crate) fn role_from_part_role(role: PartRole) -> Role {
    match role {
        PartRole::User => Role::User,
        PartRole::Assistant => Role::Assistant,
        PartRole::System => Role::System,
        PartRole::Tool => Role::Tool,
        PartRole::Runtime => Role::System,
    }
}

pub(crate) fn execution_status_from_part_state(state: PartState) -> ExecutionStatus {
    match state {
        PartState::Pending => ExecutionStatus::Pending,
        PartState::InProgress => ExecutionStatus::InProgress,
        PartState::Completed => ExecutionStatus::Completed,
        PartState::Failed => ExecutionStatus::Failed,
        PartState::Cancelled => ExecutionStatus::Cancelled,
    }
}

pub(crate) fn part_state_from_execution_status(status: ExecutionStatus) -> PartState {
    match status {
        ExecutionStatus::Pending => PartState::Pending,
        ExecutionStatus::InProgress => PartState::InProgress,
        ExecutionStatus::Completed => PartState::Completed,
        // The persisted part state vocabulary is coarse (schema CHECK); the
        // precise non-execution reason rides in the part content, matching the
        // transcript's coarse mapping (history.rs `activity_state_from_execution`).
        ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => PartState::Completed,
        ExecutionStatus::Failed => PartState::Failed,
        ExecutionStatus::Cancelled => PartState::Cancelled,
    }
}

/// Project the facade's normalized usage aggregates (16.3) onto the shared
/// domain DTO. The facade returns per-provider×model groups; the domain
/// breakdowns for day/session are derived from the same scalar columns. P5
/// enriches `by_day` / `by_session` from the per-day/per-session queries
/// (16.3); totals and provider/model breakdowns are exact here.
fn domain_usage_stats_from_storage(
    stats: agena_storage::store::UsageStats,
    query: &agena_domain::UsageStatsQuery,
    generated_at: DateTime<Utc>,
) -> agena_domain::UsageStats {
    let micros_per_usd = 1_000_000_f64;
    let runs = stats.total_calls.max(0) as u64;
    let input_tokens = stats.total_input_tokens.max(0) as u64;
    let output_tokens = stats.total_output_tokens.max(0) as u64;
    let total_cost_usd = stats.total_cost_micros.max(0) as f64 / micros_per_usd;
    let totals = agena_domain::UsageTotals {
        runs,
        sessions: 0,
        input_tokens,
        output_tokens,
        reasoning_tokens: 0,
        cache_write_tokens: 0,
        cache_write_5m_tokens: 0,
        cache_write_1h_tokens: 0,
        cache_read_tokens: 0,
        tool_use_tokens: 0,
        other_tokens: 0,
        total_tokens: input_tokens + output_tokens,
        cache_input_tokens: 0,
        cache_hit_rate: 0.0,
        total_cost_usd,
        recorded_cost_usd: 0.0,
        estimated_cost_usd: total_cost_usd,
        unpriced_runs: 0,
        billable_units: Vec::new(),
    };
    let by_provider = stats.groups.iter().fold(
        std::collections::BTreeMap::<String, agena_domain::UsageTotals>::new(),
        |mut map, group| {
            let provider = map.entry(group.provider_id.clone()).or_default();
            provider.runs += group.calls.max(0) as u64;
            provider.input_tokens += group.input_tokens.max(0) as u64;
            provider.output_tokens += group.output_tokens.max(0) as u64;
            provider.total_tokens +=
                (group.input_tokens.max(0) + group.output_tokens.max(0)) as u64;
            provider.estimated_cost_usd += group.total_cost_micros.max(0) as f64 / micros_per_usd;
            map
        },
    );
    let by_provider = by_provider
        .into_iter()
        .map(
            |(provider_id, totals)| agena_domain::ProviderUsageBreakdown {
                provider_id,
                totals,
            },
        )
        .collect();
    let by_model = stats
        .groups
        .into_iter()
        .map(|group| {
            let totals = agena_domain::UsageTotals {
                runs: group.calls.max(0) as u64,
                input_tokens: group.input_tokens.max(0) as u64,
                output_tokens: group.output_tokens.max(0) as u64,
                total_tokens: (group.input_tokens.max(0) + group.output_tokens.max(0)) as u64,
                reasoning_tokens: group.reasoning_tokens.max(0) as u64,
                cache_write_tokens: group.cache_write_tokens.max(0) as u64,
                cache_read_tokens: group.cache_read_tokens.max(0) as u64,
                estimated_cost_usd: group.total_cost_micros.max(0) as f64 / micros_per_usd,
                ..Default::default()
            };
            agena_domain::ModelUsageBreakdown {
                provider_id: group.provider_id,
                model_id: group.model_id,
                totals,
            }
        })
        .collect();
    agena_domain::UsageStats {
        generated_at,
        period: query.period,
        period_label: query.period.label().to_string(),
        from: query.from.to_owned(),
        to: query.to.to_owned(),
        timezone_offset_minutes: query.timezone_offset_minutes,
        totals,
        active_days: 0,
        average_cost_per_run_usd: if runs == 0 {
            0.0
        } else {
            total_cost_usd / runs as f64
        },
        average_tokens_per_run: if runs == 0 {
            0.0
        } else {
            (input_tokens + output_tokens) as f64 / runs as f64
        },
        average_cost_per_active_day_usd: 0.0,
        average_tokens_per_active_day: 0.0,
        peak_cost_date: None,
        peak_cost_usd: 0.0,
        peak_tokens_date: None,
        peak_tokens: 0,
        by_day: Vec::new(),
        by_provider,
        by_model,
        by_session: Vec::new(),
    }
}

/// Decode the subtask failure JSON column without treating corrupt persisted
/// state as an absent failure.
fn subtask_failure_from_value(
    value: Option<&Value>,
) -> Result<Option<agena_failure::Failure>, AppError> {
    value
        .map(|value| {
            serde_json::from_value::<agena_failure::Failure>(value.clone()).map_err(|error| {
                AppError::Internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "decode persisted subtask failure",
                    &error,
                ))
            })
        })
        .transpose()
}

/// Apply session-row metadata (provider anchors, subtask, execution config)
/// to a freshly built runtime state.
fn apply_meta_runtime(
    runtime: &mut crate::session::SessionRuntimeState,
    meta: &SessionMeta,
) -> Result<(), AppError> {
    if let Some(value) = meta.provider_anchors_json.as_ref() {
        runtime.provider_anchors = serde_json::from_value::<
            BTreeMap<String, crate::model::ProviderPromptAnchor>,
        >(value.clone())
        .map_err(|error| {
            AppError::Internal(agena_failure::diagnostic::format_error_chain_with_context(
                format!("decode provider anchors for session {}", meta.id),
                &error,
            ))
        })?;
    }
    runtime.subtask = crate::session::SubtaskRuntimeState {
        status: meta
            .subtask_status
            .as_deref()
            .and_then(agena_domain::SubtaskStatus::parse)
            .unwrap_or_default(),
        started_at_ms: meta.subtask_started_at_ms,
        finished_at_ms: meta.subtask_finished_at_ms,
        failure: subtask_failure_from_value(meta.subtask_failure.as_ref())?,
    };
    if let Some(value) = meta.config_json.as_ref() {
        let config =
            serde_json::from_value::<PersistedExecutionConfig>(value.clone()).map_err(|error| {
                AppError::Internal(agena_failure::diagnostic::format_error_chain_with_context(
                    format!("decode persisted execution config for session {}", meta.id),
                    &error,
                ))
            })?;
        runtime.execution.selection = config.selection;
        runtime.execution.access = config.access;
        runtime.execution.permission_ceiling = config.permission_ceiling;
        runtime.execution.capability_denied_tool_names = config.capability_denied_tool_names;
        runtime.execution.effective_workspace_root = config.effective_workspace_root;
    }
    Ok(())
}

/// The D5 slice of `sessions.config_json`: execution configuration only, never
/// derived workflow state. `effective_permission` is deliberately excluded —
/// it is re-derived from the permission policy at run start (refresh_execution_policy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PersistedExecutionConfig {
    #[serde(flatten)]
    pub selection: ExecutionSelection,
    #[serde(default, skip_serializing_if = "ExecutionAccess::is_inherit")]
    pub access: ExecutionAccess,
    #[serde(
        default,
        skip_serializing_if = "crate::authorization::PermissionConfig::is_empty"
    )]
    pub permission_ceiling: crate::authorization::PermissionConfig,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capability_denied_tool_names: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<PathBuf>,
}

impl PersistedExecutionConfig {
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.access.is_inherit()
            && self.permission_ceiling.is_empty()
            && self.capability_denied_tool_names.is_empty()
            && self.effective_workspace_root.is_none()
    }
}

impl From<&crate::session::model::SessionExecutionContext> for PersistedExecutionConfig {
    fn from(execution: &crate::session::model::SessionExecutionContext) -> Self {
        Self {
            selection: execution.selection.clone(),
            access: execution.access,
            permission_ceiling: execution.permission_ceiling.clone(),
            capability_denied_tool_names: execution.capability_denied_tool_names.clone(),
            effective_workspace_root: execution.effective_workspace_root.clone(),
        }
    }
}

pub(crate) fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    let seconds = timestamp_ms.div_euclid(1000);
    let nanos = (timestamp_ms.rem_euclid(1000) * 1_000_000) as u32;
    DateTime::from_timestamp(seconds, nanos)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp {timestamp_ms}ms")))
}

/// Convert a facade summary row into the shared domain DTO. The current facade
/// does not populate the optional source-watermark or per-summary subtask
/// fields, so those values remain `None` in this projection.
pub(crate) fn domain_summary_from_storage(
    summary: agena_storage::store::SessionSummary,
) -> Result<agena_domain::SessionSummary, AppError> {
    let last_message_at = summary
        .last_message_at_ms
        .map(timestamp_millis_to_utc)
        .transpose()?;
    Ok(agena_domain::SessionSummary {
        id: summary.id,
        parent_id: summary.parent_id,
        depth: summary.depth,
        root_id: summary.root_id,
        workspace_id: summary.workspace_id,
        title: summary.title,
        favorite: summary.favorite,
        pinned: summary.pinned,
        version: summary.version,
        relation_kind: summary.relation_kind,
        lifecycle_state: summary.lifecycle_state,
        source_cutoff_seq_global: None,
        source_message_id: None,
        task_id: summary.task_id,
        subtask_access: None,
        subtask_status: summary
            .subtask_status
            .as_deref()
            .and_then(agena_domain::SubtaskStatus::parse),
        created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
        message_count: u64::try_from(summary.message_count).map_err(|_| {
            AppError::Internal(format!(
                "invalid negative message count for session {}",
                summary.id
            ))
        })?,
        child_session_count: u64::try_from(summary.child_session_count).map_err(|_| {
            AppError::Internal(format!(
                "invalid negative child count for session {}",
                summary.id
            ))
        })?,
        last_message_at,
    })
}

/// A run marker's content payload used by the manager when starting a run.
///
/// `kind = "run"` parts carry their run kind in `content.run_kind` (design
/// 4.1), may carry model identity for prompt assembly, and persist the
/// canonical conversation identity (design 19.5) so reply wake-up and
/// reply-command matching survive a reload.
pub(crate) fn run_marker_content(
    run_kind: &str,
    model_provider_id: Option<&str>,
    model_id: Option<&str>,
    conversation_turn_id: Option<agena_domain::TurnId>,
    conversation_reply_id: Option<agena_domain::AssistantReplyId>,
) -> Value {
    let mut content = serde_json::json!({ "run_kind": run_kind });
    if let Some(provider_id) = model_provider_id {
        content["provider_id"] = Value::String(provider_id.to_owned());
    }
    if let Some(model_id) = model_id {
        content["model_id"] = Value::String(model_id.to_owned());
    }
    if let Some(turn_id) = conversation_turn_id {
        content["turn_id"] = Value::String(turn_id.to_string());
    }
    if let Some(reply_id) = conversation_reply_id {
        content["reply_id"] = Value::String(reply_id.to_string());
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{
        SessionLifecycleState, SessionRelationKind, StructuredObject, TimeRange, ToolInvocation,
    };
    use agena_storage::store::{PartVisibility, SessionMeta, SessionView};

    fn meta(id: i64) -> SessionMeta {
        SessionMeta {
            id,
            parent_id: None,
            depth: 0,
            root_id: id,
            workspace_id: 1,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "t".to_owned(),
            favorite: false,
            pinned: false,
            version: 1,
            lifecycle_state: SessionLifecycleState::Ready,
            creation_failure: None,
            task_id: None,
            subtask_status: None,
            subtask_started_at_ms: None,
            subtask_finished_at_ms: None,
            subtask_failure: None,
            config_json: None,
            provider_anchors_json: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    fn part(
        part_id: i64,
        kind: &str,
        role: PartRole,
        state: PartState,
        run_id: Option<i64>,
        content: Value,
        created_at_ms: i64,
    ) -> Part {
        Part {
            part_id,
            kind: kind.to_owned(),
            role,
            state,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id,
            origin_session_id: 1,
            revision: 1,
            started_at_ms: created_at_ms,
            finished_at_ms: Some(created_at_ms + 1),
            created_at_ms,
            updated_at_ms: created_at_ms,
            provider_state: None,
        }
    }

    #[test]
    fn session_from_view_groups_parts_by_run_marker() {
        let user_marker = part(
            10,
            "run",
            PartRole::User,
            PartState::Completed,
            None,
            serde_json::json!({"run_kind": "user"}),
            1000,
        );
        let user_text = part(
            11,
            "text",
            PartRole::User,
            PartState::Completed,
            Some(10),
            text_content_value("hello"),
            1010,
        );
        let assistant_marker = part(
            20,
            "run",
            PartRole::Assistant,
            PartState::Completed,
            None,
            serde_json::json!({"run_kind": "execution"}),
            2000,
        );
        let assistant_text = part(
            21,
            "text",
            PartRole::Assistant,
            PartState::Completed,
            Some(20),
            text_content_value("hi"),
            2010,
        );
        let view = SessionView {
            meta: meta(1),
            parts: vec![user_marker, user_text, assistant_marker, assistant_text],
        };
        let session = session_from_view(view).unwrap();
        assert_eq!(session.id, 1);
        let runs = parts_into_runs(session.parts());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0][0].role, PartRole::User);
        assert_eq!(runs[0][0].part_id, 10);
        assert_eq!(runs[0].len(), 2);
        assert_eq!(
            typed_text_from_value(&runs[0][1].content).as_deref(),
            Some("hello"),
            "part text round-trips through the canonical JSON payload"
        );
        assert_eq!(runs[1][0].role, PartRole::Assistant);
        assert_eq!(
            typed_text_from_value(&runs[1][1].content).as_deref(),
            Some("hi")
        );
    }

    #[test]
    fn late_assistant_hook_groups_with_launch_turn_but_survives_without_its_marker() {
        let launch_marker = part(
            10,
            "run",
            PartRole::Assistant,
            PartState::Completed,
            None,
            serde_json::json!({"run_kind": "continue"}),
            1000,
        );
        let later_marker = part(
            20,
            "run",
            PartRole::Assistant,
            PartState::Completed,
            None,
            serde_json::json!({"run_kind": "continue"}),
            2000,
        );
        let notification = part(
            30,
            "system_notification",
            PartRole::Assistant,
            PartState::Completed,
            Some(10),
            serde_json::json!({
                "operation_id": "proc_late",
                "operation_kind": "shell",
                "status": "completed",
                "summary": "done",
                "body": "done"
            }),
            3000,
        );

        let full = parts_into_runs(&[
            launch_marker.clone(),
            later_marker.clone(),
            notification.clone(),
        ]);
        assert_eq!(full.len(), 2);
        assert_eq!(full[0][0].part_id, launch_marker.part_id);
        assert_eq!(full[0][1].part_id, notification.part_id);
        assert_eq!(
            full[1][0].part_id, later_marker.part_id,
            "the full transcript presents the hook on its original assistant turn"
        );

        let markerless_active_window = parts_into_runs(&[later_marker, notification.clone()]);
        assert_eq!(markerless_active_window.len(), 2);
        assert_eq!(
            markerless_active_window[1],
            vec![notification],
            "a post-compaction active window still delivers the late hook to the model"
        );
    }

    #[test]
    fn part_content_round_trips_through_json() {
        let content = TypedContent::Text(part_content::TextContent {
            text: "round trip".to_owned(),
            synthetic: false,
            extra: BTreeMap::new(),
        });
        let value = typed_content_to_value(&content).unwrap();
        let back = typed_content_from_value("text", &value).unwrap();
        assert_eq!(back, content);
        assert_eq!(typed_text_from_value(&value).as_deref(), Some("round trip"));
    }

    #[test]
    fn new_part_serializes_content_and_role() {
        let content = TypedContent::Text(part_content::TextContent {
            text: "payload".to_owned(),
            synthetic: false,
            extra: BTreeMap::new(),
        });
        let new_part =
            new_part_from_content("text", PartRole::User, &content, PartState::Completed).unwrap();
        assert_eq!(new_part.kind, "text");
        assert_eq!(new_part.role, PartRole::User);
        assert_eq!(new_part.state, PartState::Completed);
        assert_eq!(
            typed_content_from_value("text", &new_part.content).unwrap(),
            content
        );
    }

    /// Canonical `text` payload helper used by the storage fixtures below.
    fn text_content_value(text: &str) -> Value {
        part_content::TextContent {
            text: text.to_owned(),
            synthetic: false,
            extra: BTreeMap::new(),
        }
        .as_value()
    }

    /// Recover the text of a canonical `text` payload (or `None` when the
    /// payload is not a text part).
    fn typed_text_from_value(value: &Value) -> Option<String> {
        match typed_content_from_value("text", value).ok()? {
            TypedContent::Text(part) => Some(part.text),
            _ => None,
        }
    }

    #[test]
    fn rich_operation_round_trips_losslessly_through_canonical_tool_call() {
        // A completed tool operation carries a result envelope, details and a
        // stashed provider operation id; the canonical tool_call payload must
        // preserve all of it (design 4.1.1 + 19.4 extended keys).
        let mut operation = OperationPart::pending(
            7,
            ToolInvocation::plugin_named(
                "fs.read",
                "builtin",
                StructuredObject::try_from(serde_json::json!({"file_path": "/tmp/x.txt"})).unwrap(),
            ),
            TimeRange {
                start_ms: 1000,
                end_ms: Some(2000),
            },
        );
        operation.metadata.insert(
            OPERATION_ID_METADATA_KEY.to_owned(),
            Value::String("op-42".to_owned()),
        );
        operation.output = Some(agena_domain::RawOutput {
            payload: Some(serde_json::json!({"lines": 3})),
            ..Default::default()
        });
        operation.state = agena_domain::ToolResultState::Completed;
        let content = TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation)));

        // Serialize via the typed canonical shape, then rebuild.
        let value = typed_content_to_value(&content).unwrap();

        // Canonical named keys are present; the single payload is the only
        // output fact (no operation bucket, no result envelope).
        assert_eq!(value["name"], serde_json::json!("fs.read"));
        assert_eq!(value["plugin"], serde_json::json!("builtin"));
        assert_eq!(value["input"]["file_path"], serde_json::json!("/tmp/x.txt"));
        assert_eq!(value["output"]["payload"]["lines"], serde_json::json!(3));
        assert!(value.get("operation").is_none());
        assert!(value.get("result").is_none());

        let back = typed_content_from_value("tool_call", &value).unwrap();
        let TypedContent::ToolCall(tool_call) = back else {
            panic!("tool_call must rebuild as a typed tool call");
        };
        let rebuilt = part_content::operation_from_tool_call(&tool_call);
        assert_eq!(
            rebuilt, operation,
            "rich operation must survive the canonical round trip"
        );
        assert_eq!(
            rebuilt
                .metadata
                .get(OPERATION_ID_METADATA_KEY)
                .and_then(Value::as_str),
            Some("op-42")
        );
    }

    #[test]
    fn tool_part_does_not_duplicate_failure_in_summary_column() {
        let operation = OperationPart::failed(
            9,
            ToolInvocation::plugin_named("fs.read", "builtin", StructuredObject::default()),
            agena_failure::Failure::new(
                agena_failure::FailureCode::new("tool.internal"),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::UseAlternative,
                agena_failure::RecoveryDirective::ChooseAlternative,
                agena_failure::FailureImpact::OperationFailed,
                agena_failure::UserPresentation::new(
                    "tool-internal-failure",
                    "the original tool failure",
                ),
            ),
            agena_domain::RawOutput::text("the original raw result"),
            TimeRange::default(),
        );
        let content = TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation)));

        let part = new_part_from_content(
            "tool_call",
            PartRole::Assistant,
            &content,
            PartState::Failed,
        )
        .expect("canonical tool part");

        assert!(part.summary.is_none());
        assert_eq!(
            part.content["error"]["failure"]["user"]["fallback"],
            "the original tool failure"
        );
    }
}
