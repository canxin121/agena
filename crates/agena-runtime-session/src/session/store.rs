//! The v2 data boundary for the session manager.
//!
//! v1 kept a session store that owned the database, projected an event log
//! into model messages, reserved ids, and ran leases. All of that is gone in
//! v2 (design 14-15): the sealed [`agena_storage::SessionStore`] facade owns
//! ids, leases, and transactions; parts are the only chat entity; there is no
//! event log and no live `EventKind` plumbing.
//!
//! This module is the manager's thin adapter over that facade. It:
//!
//! - converts a [`SessionView`] (metadata + parts) back into the v1
//!   [`Session`] aggregate the execution engine still operates on
//!   (parts are grouped into [`Message`]s by their `run` marker);
//! - converts [`PartContent`] payloads to and from the JSON stored on
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

use agena_domain::{ExecutionAccess, ExecutionSelection, ExecutionStatus, Role};
use agena_storage::store::{
    NewPart, Part, PartDelta, PartRole, PartState, PartVisibility, SessionMeta, SessionStore,
    SessionView, StoreError, SubmitOutcome, UsageQuery,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;
use crate::message::{
    Message, MessageMetadata, MessagePart, MessageProviderState, PartContent, RequestPart,
    RuntimeActivity,
};
use crate::session::Session;

/// The facade-backed store adapter used by [`crate::SessionManager`].
///
/// `owner_id` is the same process-wide execution identity passed to
/// [`agena_storage::SessionFacade::new`]; every write routes through the
/// facade's lease validation so the manager never touches leases itself.
///
/// v1 reserved database ids up front and built messages with them. v2 makes
/// the engine the only id source (design 14.2), so freshly built in-memory
/// parts carry a negative placeholder until the facade returns the real id.
/// The adapter remaps placeholders to engine ids on every write and rewrites
/// the in-memory aggregate so message/part references stay consistent.
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
            failure: subtask_failure_from_value(meta.subtask_failure.as_ref()),
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
    /// in one transaction, returns the run id and the engine-id'd user
    /// [`Message`] (plus all created parts for callers that need them).
    pub(crate) async fn submit_user_message(
        &self,
        session_id: i64,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, AppError> {
        // The facade returns only the run marker id (7.1); the created parts
        // are reloaded from the view so callers can remap placeholders. The
        // view is ordered by `(created_at_ms, part_id)` so the marker leads.
        let run_id = self
            .facade
            .submit_user_message(session_id, &self.owner_id, parts, idempotency_key)
            .await
            .map_err(store_error)?;
        let view = self.facade.load(session_id).await.map_err(store_error)?;
        let created: Vec<Part> = view
            .parts
            .iter()
            .filter(|part| part.part_id == run_id || part.run_id == Some(run_id))
            .cloned()
            .collect();
        Ok(SubmitOutcome {
            run_id,
            created: true,
            parts: created,
        })
    }

    /// Start a non-user run (continue / compaction / subtask) and return the
    /// new run marker part id.
    pub(crate) async fn start_run(
        &self,
        session_id: i64,
        run_kind: &str,
        content: Value,
    ) -> Result<i64, AppError> {
        self.facade
            .start_run(session_id, &self.owner_id, run_kind, content, None)
            .await
            .map_err(store_error)
    }

    /// Append content parts under an existing run marker. Returns the created
    /// parts (engine ids) so callers can rebuild in-memory messages.
    pub(crate) async fn append_parts(
        &self,
        session_id: i64,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<Vec<Part>, AppError> {
        // The facade returns only `()` (14.1); the created parts are reloaded
        // from the view so callers can remap placeholders. The view is ordered
        // by `(created_at_ms, part_id)`, so the run's newest members — the
        // last `part_count` entries of the run's parts — are exactly the
        // appended ones (same transaction, largest ids).
        let part_count = parts.len();
        self.facade
            .append_parts(session_id, &self.owner_id, run_id, parts)
            .await
            .map_err(store_error)?;
        let view = self.facade.load(session_id).await.map_err(store_error)?;
        let run_parts: Vec<Part> = view
            .parts
            .into_iter()
            .filter(|part| part.run_id == Some(run_id))
            .collect();
        let split = run_parts.len().saturating_sub(part_count);
        Ok(run_parts.into_iter().skip(split).collect())
    }

    /// Apply a streaming delta to one part and return the updated part.
    pub(crate) async fn update_part(
        &self,
        session_id: i64,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<Part, AppError> {
        // The facade returns `()` (14.1); the authoritative part is reloaded
        // so the caller applies the engine's values back onto its aggregate.
        self.facade
            .update_part(session_id, &self.owner_id, part_id, delta)
            .await
            .map_err(store_error)?;
        let view = self.facade.load(session_id).await.map_err(store_error)?;
        view.parts
            .into_iter()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!("updated part {part_id} not found after update"))
            })
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
            .map_err(store_error)
    }

    pub(crate) async fn cancel_run(&self, session_id: i64, run_id: i64) -> Result<(), AppError> {
        self.facade
            .cancel_run(session_id, &self.owner_id, run_id)
            .await
            .map_err(store_error)
    }

    /// Reconcile a session whose in-flight run lost its lease: mark stale run
    /// markers failed and their non-terminal children cancelled (17.4).
    pub(crate) async fn reconcile(&self, session_id: i64) -> Result<(), AppError> {
        self.facade.reconcile(session_id).await.map_err(store_error)
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

    /// Start a compaction run; returns the compaction run marker part id.
    pub(crate) async fn compact_session(&self, session_id: i64) -> Result<i64, AppError> {
        self.facade
            .compact_session(session_id, &self.owner_id)
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
    /// facade pages by `(updated_at_ms, id)` cursor; the legacy
    /// `SessionListRequest.offset` paging is emulated by skipping the first
    /// `offset` rows, and `include_subagents = false` drops subtask rows
    /// (13.11 keeps the DTO shape; the watermark fields v2 dissolved — source
    /// cutoff / message id / subtask access — are `None`).
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

    /// Reserve in-memory identity for a brand-new message. v2 makes the
    /// engine the only durable id source (14.2), so these are negative
    /// placeholders the adapter remaps to engine ids on the next persist.
    pub(crate) async fn reserve_message_ids(
        &self,
        part_count: usize,
    ) -> Result<ReservedMessageIds, AppError> {
        Ok(ReservedMessageIds::unpersisted(part_count))
    }

    /// Reserve identity for a processor model run: a placeholder assistant
    /// message id plus a placeholder part allocator. The run marker is
    /// created by `start_run` on persist; its part id becomes the message id.
    pub(crate) async fn reserve_processor_ids(&self) -> Result<ProcessorIds, AppError> {
        Ok(ProcessorIds {
            message_id: next_placeholder_id(),
            part_ids: ProcessorPartIdAllocator,
        })
    }

    pub(crate) async fn reserve_part_id(&self) -> Result<i64, AppError> {
        Ok(next_placeholder_id())
    }

    fn session_from_meta(&self, meta: SessionMeta) -> Result<Session, AppError> {
        let view = SessionView {
            meta,
            parts: Vec::new(),
        };
        session_from_view(view)
    }
}

/// In-memory identity for one processor model run (see
/// [`StoreAdapter::reserve_processor_ids`]).
#[derive(Debug, Clone)]
pub(crate) struct ProcessorIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: ProcessorPartIdAllocator,
}

/// Explicit delta for durable model-message projection.
///
/// A persist names the exact parts whose value or status changed. This
/// prevents an update to one streamed Operation from checkpointing every
/// older sibling in the same assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageCheckpoint {
    pub(crate) message_id: i64,
    pub(crate) part_ids: Vec<i64>,
}

impl MessageCheckpoint {
    pub(crate) fn all(message: &Message) -> Self {
        Self::parts(message.id, message.parts.iter().map(|part| part.id))
    }

    pub(crate) fn part(message_id: i64, part_id: i64) -> Self {
        Self {
            message_id,
            part_ids: vec![part_id],
        }
    }

    pub(crate) fn parts(message_id: i64, part_ids: impl IntoIterator<Item = i64>) -> Self {
        let mut part_ids = part_ids.into_iter().collect::<Vec<_>>();
        part_ids.sort_unstable();
        part_ids.dedup();
        Self {
            message_id,
            part_ids,
        }
    }
}

/// In-memory message/part identity before the engine assigns durable ids.
///
/// v2 makes the persistence engine the only id source (design 14.2), so the
/// manager builds fresh in-memory messages with negative placeholder ids and
/// the adapter remaps them to engine ids on every persist. `reserve_message_ids`
/// in v1 pre-allocated real ids; v2 has no such allocator.
#[derive(Debug, Clone)]
pub(crate) struct ReservedMessageIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: Vec<i64>,
}

impl ReservedMessageIds {
    /// A message id plus `part_count` part ids, all process-unique placeholders
    /// (negative) that `persist_session` replaces with engine ids.
    pub(crate) fn unpersisted(part_count: usize) -> Self {
        let message_id = next_placeholder_id();
        let part_ids = (0..part_count)
            .map(|_| next_placeholder_id())
            .collect::<Vec<_>>();
        Self {
            message_id,
            part_ids,
        }
    }
}

/// Process-local, monotonic source of negative placeholder ids.
fn next_placeholder_id() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
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

impl StoreAdapter {
    /// Persist the session's in-memory deltas through the facade and return
    /// the remapped aggregate (engine ids on every new message and part).
    ///
    /// This is the v2 replacement for the v1 `SessionCommit`/`persist` write
    /// path. The manager mutates a `Session` it loaded (or built with
    /// placeholder ids) and calls this with the [`MessageCheckpoint`]s naming
    /// the parts whose value/status changed. The adapter:
    ///
    /// - submits brand-new user runs via `submit_user_message` (marker +
    ///   content parts in one transaction, 7.1);
    /// - starts brand-new non-user runs via `start_run` (the manager normally
    ///   starts the run first so the marker is durable before the provider
    ///   call, but a placeholder message here is started and appended);
    /// - appends new parts under existing runs (`append_parts`, D10);
    /// - pushes checkpointed part deltas (`update_part`);
    /// - terminalizes completed/failed/cancelled runs (`complete_run`).
    ///
    /// Every created/updated engine id is written back onto the in-memory
    /// aggregate and derived state is rebuilt, so the returned `Session` is
    /// fully consistent for the next execution step.
    pub(crate) async fn persist_session(
        &self,
        mut session: Session,
        checkpoints: &[MessageCheckpoint],
    ) -> Result<Session, AppError> {
        let session_id = session.id;
        let mut remapped = Vec::with_capacity(session.messages.len());
        for mut message in std::mem::take(&mut session.messages) {
            message = self
                .persist_message(session_id, message, checkpoints)
                .await?;
            remapped.push(message);
        }
        session.messages = remapped;
        session.install_projected_messages(session.messages.clone());
        Ok(session)
    }

    /// Persist one message's deltas under its run. `message.id` is the run
    /// marker part id (after remap) or a placeholder for a brand-new run.
    async fn persist_message(
        &self,
        session_id: i64,
        mut message: Message,
        checkpoints: &[MessageCheckpoint],
    ) -> Result<Message, AppError> {
        let is_new_run = message.id < 0;
        if is_new_run {
            return self.persist_new_message(session_id, message).await;
        }

        // Existing run: append new parts, then push checkpointed deltas, then
        // terminalize when the message state is final and this persist names it.
        let run_id = message.id;
        let new_parts: Vec<_> = message.parts.iter().filter(|part| part.id < 0).collect();
        if !new_parts.is_empty() {
            let created = self
                .append_parts(
                    session_id,
                    run_id,
                    new_parts
                        .iter()
                        .map(|part| new_part_from_message_part(part, message.role))
                        .collect::<Result<Vec<_>, _>>()?,
                )
                .await?;
            remap_new_parts(&mut message, &created);
        }

        let checkpointed = checkpoints.iter().find(|c| c.message_id == message.id);
        if let Some(checkpoint) = checkpointed {
            for part in &mut message.parts {
                if part.id >= 0 && checkpoint.part_ids.contains(&part.id) {
                    let delta = delta_for_part(part, self.now_ms())?;
                    let updated = self.update_part(session_id, part.id, delta).await?;
                    apply_part_to_message_part(part, &updated);
                }
            }
        }

        // Terminalize the run marker only when the whole run is terminal:
        // every part under the marker must be terminal too. A successful
        // model turn with pending tool calls keeps the marker in-flight so
        // the session stays Running (design 17.3/17.5) while the tools
        // execute; the marker completes on the persist that makes the last
        // part terminal. `message.state` is already the terminal state the
        // processor/manager assigned, so no field needs rewriting.
        if message.state.is_terminal()
            && checkpointed.is_some()
            && message.parts.iter().all(|part| part.status.is_terminal())
        {
            let outcome = run_outcome_for(&message);
            if matches!(message.state, ExecutionStatus::Cancelled) {
                self.cancel_run(session_id, run_id).await?;
            } else {
                self.complete_run(session_id, run_id, outcome).await?;
            }
        }

        Ok(message)
    }

    /// Persist a brand-new run: user messages submit (7.1); non-user messages
    /// start a run marker then append their parts.
    async fn persist_new_message(
        &self,
        session_id: i64,
        mut message: Message,
    ) -> Result<Message, AppError> {
        if message.role == agena_domain::Role::User {
            let new_parts = message
                .parts
                .iter()
                .map(|part| new_part_from_message_part(part, message.role))
                .collect::<Result<Vec<_>, _>>()?;
            let idempotency_key = message.metadata.idempotency_key.clone();
            let outcome = self
                .submit_user_message(session_id, new_parts, idempotency_key)
                .await?;
            let run_id = outcome.run_id;
            message.id = run_id;
            message.metadata.model_turn_id = Some(run_id);
            for (part, created) in message.parts.iter_mut().zip(outcome.parts.iter().skip(1)) {
                part.id = created.part_id;
                part.message_id = run_id;
            }
            self.terminalize_new_marker(session_id, &mut message, run_id)
                .await?;
            return Ok(message);
        }

        // Non-user run: start a marker, then append the parts.
        let run_kind = "execution";
        let run_id = self
            .start_run(
                session_id,
                run_kind,
                run_marker_content(
                    run_kind,
                    Some(message.metadata.model_provider_id.as_str()),
                    Some(message.metadata.model_id.as_str()),
                    message.metadata.conversation_turn_id,
                    message.metadata.conversation_reply_id,
                ),
            )
            .await?;
        message.id = run_id;
        message.metadata.model_turn_id = Some(run_id);
        if !message.parts.is_empty() {
            let new_parts = message
                .parts
                .iter()
                .map(|part| new_part_from_message_part(part, message.role))
                .collect::<Result<Vec<_>, _>>()?;
            let created = self.append_parts(session_id, run_id, new_parts).await?;
            remap_new_parts(&mut message, &created);
        }
        self.terminalize_new_marker(session_id, &mut message, run_id)
            .await?;
        Ok(message)
    }

    /// The engine creates a run marker in `pending`; a manager-built message
    /// always carries its terminal state on first persist, so the marker must
    /// be terminalized here or the derived session state (17.3) would leave
    /// the session permanently in-flight. Mirrors the terminal block of
    /// [`Self::persist_message`] for the brand-new-run case.
    async fn terminalize_new_marker(
        &self,
        session_id: i64,
        message: &mut Message,
        run_id: i64,
    ) -> Result<(), AppError> {
        if !message.state.is_terminal() {
            return Ok(());
        }
        // Same all-terminal rule as [`Self::persist_message`]: a brand-new
        // message with a non-terminal part (e.g. a tool placeholder awaiting
        // execution) must not complete its marker.
        if message.parts.iter().any(|part| !part.status.is_terminal()) {
            return Ok(());
        }
        if matches!(message.state, ExecutionStatus::Cancelled) {
            self.cancel_run(session_id, run_id).await?;
        } else {
            let outcome = run_outcome_for(message);
            self.complete_run(session_id, run_id, outcome).await?;
        }
        Ok(())
    }
}

/// Convert an in-memory part payload into a [`NewPart`] for facade writes,
/// using the part's current status as the initial state.
fn new_part_from_message_part(part: &MessagePart, role: Role) -> Result<NewPart, AppError> {
    let content = part.content.as_ref().ok_or_else(|| {
        AppError::Internal("part with no content cannot be persisted".to_string())
    })?;
    new_part_from_content(
        part_kind_for(part),
        part_role_from_role(role),
        content,
        part_state_from_execution_status(part.status),
    )
}

/// Map an in-memory part kind (the engine's content vocabulary) to the v2
/// `parts.kind` column. The engine's open kind set covers the rich activity
/// payloads; text and think use their literal kinds.
fn part_kind_for(part: &MessagePart) -> &'static str {
    match part.content.as_ref() {
        Some(PartContent::Text(_)) => "text",
        Some(PartContent::Activity(RuntimeActivity::Reasoning(_))) => "think",
        Some(PartContent::Activity(RuntimeActivity::Operation(_))) => "tool_call",
        Some(PartContent::Activity(RuntimeActivity::Interaction(_))) => "interaction",
        Some(PartContent::Activity(RuntimeActivity::Hook(_))) => "hook",
        Some(PartContent::Activity(RuntimeActivity::Notice(_))) => "notice",
        Some(PartContent::Activity(RuntimeActivity::Error(_))) => "error",
        Some(PartContent::Activity(RuntimeActivity::Resource(_))) => "file_ref",
        Some(PartContent::Activity(RuntimeActivity::SkillReference(_))) => "skill_ref",
        None => "text",
    }
}

/// Build a streaming [`PartDelta`] for one part's current value/status.
fn delta_for_part(part: &MessagePart, now_ms: i64) -> Result<PartDelta, AppError> {
    let content = serialize_part_content(part)?;
    Ok(PartDelta {
        state: Some(part_state_from_execution_status(part.status)),
        content: Some(content),
        content_text_delta: None,
        summary: part.summary.clone(),
        rendered_markdown: None,
        provider_state: None,
        finished_at_ms: part.status.is_terminal().then_some(now_ms),
    })
}

/// Apply the engine's authoritative part values back onto an in-memory part
/// after an `update_part` (state/content/summary round-trip).
fn apply_part_to_message_part(part: &mut MessagePart, updated: &Part) {
    part.status = execution_status_from_part_state(updated.state);
    if let Ok(content) = part_content_from_value(&updated.content) {
        part.content = Some(content);
    }
    if let Some(summary) = updated.summary.as_deref() {
        part.summary = Some(summary.to_owned());
    }
}

/// Reassign in-memory placeholder part ids from the facade's created parts.
/// The created list matches the new parts in order; the run marker (if any)
/// is the first element and is skipped.
fn remap_new_parts(message: &mut Message, created: &[Part]) {
    let mut created_iter = created.iter();
    for part in &mut message.parts {
        if part.id < 0
            && let Some(c) = created_iter.next()
        {
            part.id = c.part_id;
            part.message_id = message.id;
        }
    }
}

/// The terminal [`RunOutcome`] for a message whose state is final. The
/// message's provider state (13.2) rides onto the run marker's
/// `provider_state` column; only `MessageProviderState` persists this way.
fn run_outcome_for(message: &Message) -> agena_storage::store::RunOutcome {
    let status = part_state_from_execution_status(message.state);
    let abort_reason = match message.state {
        ExecutionStatus::Failed => Some("provider_error".to_string()),
        ExecutionStatus::Cancelled => Some("user_cancelled".to_string()),
        _ => None,
    };
    let provider_state = message.provider_state.as_ref().and_then(|state| {
        serde_json::to_value(state)
            .map_err(|error| {
                tracing::warn!(
                    target: "agena::session::store",
                    message_id = message.id,
                    "failed to serialize run provider state: {error}"
                );
            })
            .ok()
    });
    agena_storage::store::RunOutcome {
        status,
        abort_reason,
        content: None,
        provider_state,
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

/// Rebuild the execution-engine [`Session`] aggregate from a v2
/// [`SessionView`] (metadata + ordered parts).
///
/// Parts are grouped into [`Message`]s by their `run` marker (design 7.4):
/// each `run` marker part produces one message whose `MessagePart`s are the
/// marker's content parts in `(created_at_ms, part_id)` order. The message
/// role, state, and runtime metadata are derived from the marker and its
/// parts; provider anchors and execution selection come from the session row.
pub(crate) fn session_from_view(view: SessionView) -> Result<Session, AppError> {
    let SessionView { meta, parts } = view;
    let mut messages = Vec::new();

    // Runs in the order their markers appear in the (already ordered) parts.
    let mut by_run: BTreeMap<i64, Vec<&Part>> = BTreeMap::new();
    let mut marker_by_run: BTreeMap<i64, &Part> = BTreeMap::new();
    let mut singleton: Vec<&Part> = Vec::new();

    for part in &parts {
        if part.is_run_marker() {
            marker_by_run.insert(part.part_id, part);
            by_run.entry(part.part_id).or_default();
        } else if let Some(run_id) = part.run_id {
            by_run.entry(run_id).or_default().push(part);
        } else {
            // A content part with no run (should not happen for manager-owned
            // sessions, but import/foreign data may produce one).
            singleton.push(part);
        }
    }

    // Content parts are ordered by (created_at_ms, part_id) — stable sort by
    // id keeps the grouping deterministic per run.
    for run_id in by_run.keys() {
        let marker = marker_by_run
            .get(run_id)
            .copied()
            .ok_or_else(|| AppError::Internal(format!("run marker {run_id} has no marker part")))?;
        let mut run_parts = by_run[run_id].clone();
        run_parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
        messages.push(message_from_run(marker, run_parts)?);
    }
    for part in singleton {
        messages.push(message_from_singleton(part)?);
    }

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
    session.messages = messages;
    apply_meta_runtime(&mut session.runtime, &meta);
    session.install_projected_messages(session.messages.clone());
    Ok(session)
}

fn message_from_run(marker: &Part, run_parts: Vec<&Part>) -> Result<Message, AppError> {
    let role = role_from_part_role(marker.role);
    let state = execution_status_from_part_state(marker.state);
    let created_at = timestamp_millis_to_utc(marker.created_at_ms)?;
    let metadata = metadata_from_parts(marker, &run_parts);
    let provider_state = marker
        .provider_state
        .as_ref()
        .map(|value| serde_json::from_value::<MessageProviderState>(value.clone()))
        .transpose()
        .map_err(|error| AppError::Internal(format!("decode run provider state: {error}")))?;
    let mut parts = Vec::with_capacity(run_parts.len());
    for (index, part) in run_parts.into_iter().enumerate() {
        parts.push(part_to_message_part(part, marker.part_id, index as i32)?);
    }
    Ok(Message {
        id: marker.part_id,
        role,
        state,
        parts,
        created_at,
        metadata,
        provider_state,
        usage: None,
    })
}

fn message_from_singleton(part: &Part) -> Result<Message, AppError> {
    Ok(Message {
        id: part.part_id,
        role: role_from_part_role(part.role),
        state: execution_status_from_part_state(part.state),
        parts: vec![part_to_message_part(part, part.part_id, 0)?],
        created_at: timestamp_millis_to_utc(part.created_at_ms)?,
        metadata: MessageMetadata::default(),
        provider_state: None,
        usage: None,
    })
}

fn metadata_from_parts(marker: &Part, _parts: &[&Part]) -> MessageMetadata {
    let mut metadata = MessageMetadata {
        model_turn_id: Some(marker.part_id),
        ..Default::default()
    };
    // The canonical conversation identity (design 19.5) is persisted on the
    // run marker so it survives reload: reply wake-up and reply-command
    // matching resolve the same UUID pair the execution registered with.
    if let Some(turn_id) = marker.content.get("turn_id").and_then(Value::as_str)
        && let Ok(uuid) = uuid::Uuid::parse_str(turn_id)
    {
        metadata.conversation_turn_id = Some(agena_domain::TurnId(uuid));
    }
    if let Some(reply_id) = marker.content.get("reply_id").and_then(Value::as_str)
        && let Ok(uuid) = uuid::Uuid::parse_str(reply_id)
    {
        metadata.conversation_reply_id = Some(agena_domain::AssistantReplyId(uuid));
    }
    // Model identity, when recorded on the run marker content, is surfaced
    // here so prompt assembly can resolve the provider/model that produced it.
    if let Some(model_id) = marker.content.get("model_id").and_then(Value::as_str) {
        metadata.model_id = model_id.to_owned();
    }
    if let Some(provider_id) = marker.content.get("provider_id").and_then(Value::as_str) {
        metadata.model_provider_id = provider_id.to_owned();
    }
    if let Some(source) = marker.content.get("source").and_then(Value::as_str) {
        metadata.source = match source {
            "user" => agena_domain::MessageSource::User,
            "system" => agena_domain::MessageSource::System,
            "tool" => agena_domain::MessageSource::Assistant,
            _ => agena_domain::MessageSource::User,
        };
    }
    metadata
}

/// The key under which the adapter persists a part's provider `operation_id`
/// (the tool-call id used to correlate `tool_call` ↔ `tool_result` across a
/// transcript). The v2 parts schema has no column for it (design 4.1), so it
/// rides inside the rich `OperationPart.metadata` map — a reserved key the
/// engine never treats as its own. This is the adapter's private contract and
/// is invisible to everything that reads `parts.content` as the canonical
/// payload.
pub(crate) const OPERATION_ID_METADATA_KEY: &str = "agena.operation_id";

/// Serialize an in-memory part's content into the canonical JSON stored on
/// `parts.content`, stashing the provider `operation_id` (when any) into the
/// rich `OperationPart.metadata` map so a later reload can recover it (see
/// [`OPERATION_ID_METADATA_KEY`]). The in-memory aggregate is never mutated.
pub(crate) fn serialize_part_content(part: &MessagePart) -> Result<Value, AppError> {
    let Some(content) = part.content.as_ref() else {
        return Ok(Value::Null);
    };
    let mut content = content.clone();
    if let Some(operation_id) = part.operation_id.as_deref()
        && let PartContent::Activity(RuntimeActivity::Operation(operation)) = &mut content
    {
        operation.metadata.insert(
            OPERATION_ID_METADATA_KEY.to_owned(),
            Value::String(operation_id.to_owned()),
        );
    }
    serde_json::to_value(content)
        .map_err(|error| AppError::Internal(format!("serialize part content: {error}")))
}

/// Convert one persisted part into the execution-engine [`MessagePart`],
/// decoding the canonical JSON payload back into [`PartContent`].
fn part_to_message_part(
    part: &Part,
    message_id: i64,
    part_index: i32,
) -> Result<MessagePart, AppError> {
    let content = part_content_from_value(&part.content)?;
    // The coarse state column carries the lifecycle; the fine-grained status
    // (including denial outcomes) is reconstructed from the rich content.
    let status = match &content {
        PartContent::Activity(RuntimeActivity::Operation(operation)) => operation.status(),
        PartContent::Activity(RuntimeActivity::Interaction(RequestPart::UserInput(request))) => {
            request.status()
        }
        _ => execution_status_from_part_state(part.state),
    };
    let mut message_part = MessagePart::from_content_with_index(
        part.part_id,
        message_id,
        part_index,
        timestamp_millis_to_utc(part.created_at_ms)?,
        status,
        content,
    );
    if let Some(summary) = part.summary.as_deref() {
        message_part.summary = Some(summary.to_owned());
    }
    message_part.has_detail = part.content.is_object();
    // Recover the provider operation id stashed by `serialize_part_content` so
    // pending-tool correlation and prompt assembly survive a reload.
    if let Some(PartContent::Activity(RuntimeActivity::Operation(operation))) =
        message_part.content.as_ref()
    {
        if message_part.operation_id.is_none() {
            message_part.operation_id = operation
                .metadata
                .get(OPERATION_ID_METADATA_KEY)
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        message_part.activity_id = Some(agena_domain::ActivityId::new());
    }
    Ok(message_part)
}

/// Build a [`NewPart`] from an execution-engine part payload.
///
/// `state` defaults to `pending`; content is the canonical JSON of the
/// [`PartContent`]. `kind` defaults to the derived part kind when not given.
pub(crate) fn new_part_from_content(
    kind: impl Into<String>,
    role: PartRole,
    content: &PartContent,
    state: PartState,
) -> Result<NewPart, AppError> {
    let value = serde_json::to_value(content)
        .map_err(|error| AppError::Internal(format!("serialize part content: {error}")))?;
    Ok(NewPart {
        kind: kind.into(),
        role,
        content: value,
        summary: part_summary(content),
        visibility: PartVisibility::Both,
        rendered_markdown: None,
        parent_part_id: None,
        state,
    })
}

fn part_summary(content: &PartContent) -> Option<String> {
    match content {
        PartContent::Text(text) => truncate(&text.text),
        PartContent::Activity(RuntimeActivity::Reasoning(reasoning)) => {
            truncate(&reasoning.preferred_text())
        }
        PartContent::Activity(RuntimeActivity::Operation(operation)) => operation
            .error_message()
            .or_else(|| (!operation.summary.is_empty()).then_some(operation.summary.as_str()))
            .and_then(truncate),
        PartContent::Activity(RuntimeActivity::Error(error)) => {
            truncate(&error.problem.user.fallback)
        }
        PartContent::Activity(RuntimeActivity::SkillReference(reference)) => {
            truncate(&reference.summary())
        }
        PartContent::Activity(RuntimeActivity::Interaction(request)) => {
            truncate(&request.summary_text())
        }
        PartContent::Activity(RuntimeActivity::Hook(hook)) => truncate(&hook.summary),
        PartContent::Activity(RuntimeActivity::Notice(notice)) => truncate(&notice.summary),
        PartContent::Activity(RuntimeActivity::Resource(attachment)) => {
            if attachment.attachments.is_empty() {
                Some("0 attachment(s)".to_string())
            } else {
                truncate(&format!("{} attachment(s)", attachment.attachments.len()))
            }
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

/// Decode the canonical JSON payload stored on a part back into the
/// execution-engine [`PartContent`].
pub(crate) fn part_content_from_value(value: &Value) -> Result<PartContent, AppError> {
    serde_json::from_value(value.clone())
        .map_err(|error| AppError::Internal(format!("decode part content from store: {error}")))
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

pub(crate) fn part_role_from_role(role: Role) -> PartRole {
    match role {
        Role::User => PartRole::User,
        Role::Assistant => PartRole::Assistant,
        Role::System => PartRole::System,
        Role::Tool => PartRole::Tool,
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

/// Decode the subtask failure JSON column. A malformed value degrades to
/// `None` rather than failing the whole session load.
fn subtask_failure_from_value(value: Option<&Value>) -> Option<agena_failure::Failure> {
    value.and_then(|value| serde_json::from_value::<agena_failure::Failure>(value.clone()).ok())
}

/// Apply session-row metadata (provider anchors, subtask, execution config)
/// to a freshly built runtime state.
fn apply_meta_runtime(runtime: &mut crate::session::SessionRuntimeState, meta: &SessionMeta) {
    if let Some(value) = meta.provider_anchors_json.as_ref()
        && let Ok(anchors) = serde_json::from_value::<
            BTreeMap<String, crate::model::ProviderPromptAnchor>,
        >(value.clone())
    {
        runtime.provider_anchors = anchors;
    }
    runtime.subtask = crate::session::SubtaskRuntimeState {
        status: meta
            .subtask_status
            .as_deref()
            .and_then(agena_domain::SubtaskStatus::parse)
            .unwrap_or_default(),
        started_at_ms: meta.subtask_started_at_ms,
        finished_at_ms: meta.subtask_finished_at_ms,
        failure: subtask_failure_from_value(meta.subtask_failure.as_ref()),
    };
    if let Some(value) = meta.config_json.as_ref()
        && let Ok(config) = serde_json::from_value::<PersistedExecutionConfig>(value.clone())
    {
        runtime.execution.selection = config.selection;
        runtime.execution.access = config.access;
        runtime.execution.permission_ceiling = config.permission_ceiling;
        runtime.execution.capability_denied_tool_names = config.capability_denied_tool_names;
        runtime.execution.effective_workspace_root = config.effective_workspace_root;
    }
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

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    let seconds = timestamp_ms.div_euclid(1000);
    let nanos = (timestamp_ms.rem_euclid(1000) * 1_000_000) as u32;
    DateTime::from_timestamp(seconds, nanos)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp {timestamp_ms}ms")))
}

/// Convert a facade summary row into the shared domain DTO. v2 dissolved the
/// v1 event-watermark columns (`source_cutoff_seq_global`, `source_message_id`)
/// and per-summary `subtask_access` (13.2); the domain DTO keeps the fields for
/// wire compatibility and they are always `None` in v2.
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
    use agena_domain::{SessionLifecycleState, SessionRelationKind};
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
            rendered_markdown: None,
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
            serde_json::to_value(PartContent::text("hello")).unwrap(),
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
            serde_json::to_value(PartContent::text("hi")).unwrap(),
            2010,
        );
        let view = SessionView {
            meta: meta(1),
            parts: vec![user_marker, user_text, assistant_marker, assistant_text],
        };
        let session = session_from_view(view).unwrap();
        assert_eq!(session.id, 1);
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, Role::User);
        assert_eq!(session.messages[0].id, 10);
        assert_eq!(session.messages[0].parts.len(), 1);
        assert_eq!(
            session.messages[0].parts[0].text(),
            Some("hello"),
            "part text round-trips through the canonical JSON payload"
        );
        assert_eq!(session.messages[1].role, Role::Assistant);
        assert_eq!(session.messages[1].parts[0].text(), Some("hi"));
    }

    #[test]
    fn part_content_round_trips_through_json() {
        let content = PartContent::text("round trip");
        let value = serde_json::to_value(&content).unwrap();
        let back = part_content_from_value(&value).unwrap();
        assert_eq!(back, content);
        assert_eq!(back.text(), Some("round trip"));
    }

    #[test]
    fn new_part_serializes_content_and_role() {
        let content = PartContent::text("payload");
        let new_part =
            new_part_from_content("text", PartRole::User, &content, PartState::Completed).unwrap();
        assert_eq!(new_part.kind, "text");
        assert_eq!(new_part.role, PartRole::User);
        assert_eq!(new_part.state, PartState::Completed);
        assert_eq!(part_content_from_value(&new_part.content).unwrap(), content);
    }
}
