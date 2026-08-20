//! In-memory backend for the v2 parts-first store.
//!
//! Implements [`PersistenceEngine`] with plain Rust collections, so tests and
//! small deployments run without SQLite while exercising the exact same
//! contract, invariants (identity, lifecycle, shared-part read/append-only,
//! lease write-ownership), and state derivation as the production SQLite
//! engine. The facade cannot distinguish the two (design 15.4).

use portable_atomic::AtomicI64;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::Ordering;

use agena_domain::{SessionLifecycleState, SessionRelationKind};
use async_trait::async_trait;
use serde_json::{Value, json};

#[cfg(test)]
use super::PendingInteraction;
use super::{
    BackgroundDelivery, BackgroundDeliveryPhase, BackgroundEventRequest, BackgroundOperation,
    BackgroundOperationPhase, BackgroundOperationTransition, BackgroundSettleOutcome, InFlightRun,
    InteractionAnswerOutcome, LEASE_STALENESS_MS, LeaseAcquire, LeaseState, MaintenanceOutcome,
    NewBackgroundOperation, NewPart, NewSession, Part, PartCursor, PartDelta, PartRole, PartState,
    PartVisibility, PersistenceEngine, ReconcileOutcome, RunOutcome, SessionListQuery, SessionMeta,
    SessionMetadataPatch, SessionPartPage, SessionState, SessionSummary, SessionView, StateInputs,
    StoreError, SubmitOutcome, UsageGroup, UsageQuery, UsageRecord, UsageStats,
    apply_part_transition, derive_session_state,
};
use crate::store::jsonl;

/// Configuration for the in-memory engine. Kept minimal; streaming-throttle
/// policy lives in the facade (D10), not the engine.
#[derive(Debug, Clone)]
pub struct InMemoryEngineConfig {
    /// Starting value of the monotonic part-id allocator. Defaults to 1 so no
    /// part ever carries id 0 (the "no part" sentinel).
    pub first_part_id: i64,
}

impl Default for InMemoryEngineConfig {
    fn default() -> Self {
        Self { first_part_id: 1 }
    }
}

/// An in-memory [`PersistenceEngine`] for tests and small deployments.
#[derive(Debug, Clone)]
pub struct InMemoryEngine {
    next_part_id: Arc<AtomicI64>,
    next_session_id: Arc<AtomicI64>,
    now_ms: Arc<RwLock<i64>>,
    sessions: Arc<RwLock<BTreeMap<i64, SessionMeta>>>,
    parts: Arc<RwLock<HashMap<i64, Part>>>,
    /// session_id -> ordered set of member part_ids.
    membership: Arc<RwLock<HashMap<i64, BTreeSet<i64>>>>,
    leases: Arc<RwLock<HashMap<i64, LeaseState>>>,
    usage: Arc<RwLock<Vec<UsageRecord>>>,
    /// (session_id, idempotency_key) -> run_id.
    idempotency: Arc<RwLock<HashMap<(i64, String), i64>>>,
    background_operations: Arc<RwLock<HashMap<String, BackgroundOperation>>>,
    background_deliveries: Arc<RwLock<HashMap<String, BackgroundDelivery>>>,
    /// Serializes compound aggregate + transcript projection mutations so the
    /// test backend honors the production transaction boundary.
    background_write: Arc<Mutex<()>>,
}

impl Default for InMemoryEngine {
    fn default() -> Self {
        Self::new(InMemoryEngineConfig::default())
    }
}

impl InMemoryEngine {
    pub fn new(config: InMemoryEngineConfig) -> Self {
        Self {
            next_part_id: Arc::new(AtomicI64::new(config.first_part_id)),
            next_session_id: Arc::new(AtomicI64::new(1)),
            now_ms: Arc::new(RwLock::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            )),
            sessions: Arc::new(RwLock::new(BTreeMap::new())),
            parts: Arc::new(RwLock::new(HashMap::new())),
            membership: Arc::new(RwLock::new(HashMap::new())),
            leases: Arc::new(RwLock::new(HashMap::new())),
            usage: Arc::new(RwLock::new(Vec::new())),
            idempotency: Arc::new(RwLock::new(HashMap::new())),
            background_operations: Arc::new(RwLock::new(HashMap::new())),
            background_deliveries: Arc::new(RwLock::new(HashMap::new())),
            background_write: Arc::new(Mutex::new(())),
        }
    }

    /// Override the engine clock (tests).
    pub fn set_now(&self, now_ms: i64) {
        *self.now_ms.write().expect("now lock") = now_ms;
    }

    pub fn now_ms(&self) -> i64 {
        *self.now_ms.read().expect("now lock")
    }

    fn next_part_id(&self) -> i64 {
        self.next_part_id.fetch_add(1, Ordering::SeqCst)
    }

    fn next_session_id(&self) -> i64 {
        self.next_session_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Advance the persisted position for one session mutation (8.6). Callers
    /// invoke this only after releasing part/membership locks.
    fn bump_session_version(&self, session_id: i64, now_ms: i64) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(())
    }

    /// Advance every session whose view contains one of the changed parts.
    /// This includes fork/rewind children that share an origin part.
    fn bump_member_session_versions(
        &self,
        part_ids: &[i64],
        now_ms: i64,
    ) -> Result<(), StoreError> {
        if part_ids.is_empty() {
            return Ok(());
        }
        let session_ids = {
            let membership = self.membership.read().expect("membership lock");
            membership
                .iter()
                .filter(|(_, members)| part_ids.iter().any(|part_id| members.contains(part_id)))
                .map(|(session_id, _)| *session_id)
                .collect::<Vec<_>>()
        };
        if session_ids.is_empty() {
            return Err(StoreError::InvalidState(
                "changed part has no session membership".to_owned(),
            ));
        }
        let mut sessions = self.sessions.write().expect("sessions lock");
        for session_id in session_ids {
            let meta = sessions
                .get_mut(&session_id)
                .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
            meta.version += 1;
            meta.updated_at_ms = now_ms;
        }
        Ok(())
    }

    /// Allocate a run marker + content parts (7.1). Shared by user send and
    /// start_run.
    fn create_batch(
        &self,
        session_id: i64,
        marker_role: PartRole,
        marker_state: PartState,
        marker_content: Value,
        content_parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        // Idempotency: a replay of the same key returns the prior run.
        if let Some(key) = idempotency_key.as_deref() {
            let existing = self
                .idempotency
                .read()
                .expect("idempotency lock")
                .get(&(session_id, key.to_owned()))
                .copied();
            if let Some(run_id) = existing {
                let parts = self.run_parts(run_id);
                return Ok(SubmitOutcome {
                    run_id,
                    created: false,
                    parts,
                });
            }
        }

        let marker_id = self.next_part_id();
        let marker = Part {
            part_id: marker_id,
            kind: "run".to_owned(),
            role: marker_role,
            state: marker_state,
            content: marker_content,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: session_id,
            revision: 1,
            started_at_ms: now_ms,
            // Mirror the sqlite schema lifecycle invariant (a terminal state
            // must carry a finish time), matching `content_part` — a batch
            // marker created already-terminal would otherwise violate it.
            finished_at_ms: marker_state.is_terminal().then_some(now_ms),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            provider_state: None,
        };

        let mut created = vec![marker.clone()];
        {
            let mut parts = self.parts.write().expect("parts lock");
            let mut membership = self.membership.write().expect("membership lock");
            parts.insert(marker_id, marker);
            membership.entry(session_id).or_default().insert(marker_id);

            for new_part in content_parts {
                let id = self.next_part_id();
                let part = Part {
                    part_id: id,
                    kind: new_part.kind,
                    role: new_part.role,
                    state: new_part.state,
                    content: new_part.content,
                    summary: new_part.summary,
                    visibility: new_part.visibility,
                    rendered_markdown: new_part.rendered_markdown,
                    parent_part_id: new_part.parent_part_id,
                    run_id: Some(marker_id),
                    origin_session_id: session_id,
                    revision: 1,
                    started_at_ms: now_ms,
                    finished_at_ms: new_part.state.is_terminal().then_some(now_ms),
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    provider_state: None,
                };
                parts.insert(id, part.clone());
                membership.entry(session_id).or_default().insert(id);
                created.push(part);
            }
        }

        if let Some(key) = idempotency_key {
            self.idempotency
                .write()
                .expect("idempotency lock")
                .insert((session_id, key), marker_id);
        }
        self.bump_session_version(session_id, now_ms)?;
        Ok(SubmitOutcome {
            run_id: marker_id,
            created: true,
            parts: created,
        })
    }

    fn run_parts(&self, run_id: i64) -> Vec<Part> {
        let parts = self.parts.read().expect("parts lock");
        let mut out: Vec<Part> = parts
            .values()
            .filter(|part| part.run_id == Some(run_id))
            .cloned()
            .collect();
        out.sort_by_key(|part| (part.created_at_ms, part.part_id));
        out
    }

    /// Check that `owner_id` holds a fresh lease on `session_id`, distinguishing
    /// "no lease" from "held by someone else".
    fn ensure_lease(&self, session_id: i64, owner_id: &str, now_ms: i64) -> Result<(), StoreError> {
        let leases = self.leases.read().expect("leases lock");
        match leases.get(&session_id) {
            None => Err(StoreError::LeaseNotHeld { session_id }),
            Some(lease) => {
                if lease.owner_id != owner_id {
                    return Err(StoreError::LeaseHeldByOther {
                        session_id,
                        owner_id: lease.owner_id.clone(),
                        heartbeat_at_ms: lease.heartbeat_at_ms,
                    });
                }
                if now_ms - lease.heartbeat_at_ms > LEASE_STALENESS_MS {
                    return Err(StoreError::LeaseNotHeld { session_id });
                }
                Ok(())
            }
        }
    }

    /// The child's `(depth, root_id)` given its parent: `depth = parent.depth +
    /// 1`, `root_id = parent.root_id` — matching the SQLite engine and the
    /// schema invariant (`NEW.depth = parent.depth + 1 AND NEW.root_id =
    /// parent.root_id`).
    fn parent_lineage(&self, parent_id: Option<i64>) -> Result<(i64, i64), StoreError> {
        let Some(parent_id) = parent_id else {
            return Ok((0, 0));
        };
        let sessions = self.sessions.read().expect("sessions lock");
        let parent = sessions
            .get(&parent_id)
            .ok_or_else(|| StoreError::not_found(format!("parent session {parent_id}")))?;
        if parent.lifecycle_state != SessionLifecycleState::Ready {
            return Err(StoreError::InvalidState(format!(
                "parent session {parent_id} is not ready"
            )));
        }
        Ok((parent.depth + 1, parent.root_id))
    }

    /// Parts of `session_id` ordered by `(created_at_ms, part_id)`.
    fn ordered_parts(&self, session_id: i64) -> Vec<Part> {
        let membership = self.membership.read().expect("membership lock");
        let parts = self.parts.read().expect("parts lock");
        let mut out: Vec<Part> = membership
            .get(&session_id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|id| parts.get(id).cloned()))
            .collect();
        out.sort_by_key(|part| (part.created_at_ms, part.part_id));
        out
    }

    fn in_flight_runs(&self, session_id: i64) -> Vec<InFlightRun> {
        self.ordered_parts(session_id)
            .iter()
            .filter(|part| part.is_run_marker() && part.state.is_in_flight())
            .map(|part| InFlightRun {
                part_id: part.part_id,
                created_at_ms: part.created_at_ms,
            })
            .collect()
    }

    /// Abort a set of run markers and cancel their in-flight children.
    /// Returns the aborted run ids. Runs `origin_session_id = session_id`.
    fn abort_runs(
        &self,
        session_id: i64,
        run_ids: &[i64],
        reason: &str,
        now_ms: i64,
    ) -> Result<ReconcileOutcome, StoreError> {
        if run_ids.is_empty() {
            return Ok(ReconcileOutcome::default());
        }
        let marker_state = if reason == "user_cancelled" {
            PartState::Cancelled
        } else {
            PartState::Failed
        };
        let mut parts = self.parts.write().expect("parts lock");
        let mut outcome = ReconcileOutcome::default();
        for part in parts.values_mut() {
            if part.origin_session_id != session_id {
                continue;
            }
            if part.is_run_marker() && run_ids.contains(&part.part_id) && part.state.is_in_flight()
            {
                part.state = marker_state;
                part.finished_at_ms = Some(now_ms);
                part.updated_at_ms = now_ms;
                part.revision += 1;
                if let Value::Object(map) = &mut part.content {
                    map.insert("abort_reason".to_owned(), Value::String(reason.to_owned()));
                }
                outcome.aborted_runs.push(part.part_id);
                outcome.updated_parts.push(part.clone());
            } else if part.run_id.is_some_and(|run| run_ids.contains(&run))
                && part.state.is_in_flight()
            {
                part.state = PartState::Cancelled;
                part.finished_at_ms = Some(now_ms);
                part.updated_at_ms = now_ms;
                part.revision += 1;
                outcome.cancelled_parts += 1;
                outcome.updated_parts.push(part.clone());
            }
        }
        drop(parts);
        outcome
            .updated_parts
            .sort_by_key(|part| (part.created_at_ms, part.part_id));
        let changed_ids = outcome
            .updated_parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        self.bump_member_session_versions(&changed_ids, now_ms)?;
        Ok(outcome)
    }
}

fn user_send_marker_content(execution_id: Option<&str>) -> Value {
    let mut content = json!({ "run_kind": "user_send", "abort_reason": null });
    if let Some(execution_id) = execution_id {
        content["execution_id"] = Value::String(execution_id.to_owned());
    }
    content
}

#[async_trait]
impl PersistenceEngine for InMemoryEngine {
    async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError> {
        let NewSession {
            workspace_id,
            parent_id,
            relation_kind,
            cutoff_part_id,
            title,
            task_id,
            config_json,
            provider_anchors_json,
        } = new_session;
        if parent_id.is_none() && relation_kind != SessionRelationKind::Root {
            return Err(StoreError::InvalidState(
                "root session must have relation_kind = root".to_owned(),
            ));
        }
        if parent_id.is_some() && relation_kind == SessionRelationKind::Root {
            return Err(StoreError::InvalidState(
                "child session cannot have relation_kind = root".to_owned(),
            ));
        }
        let is_branch = matches!(
            relation_kind,
            SessionRelationKind::Fork | SessionRelationKind::Rewind
        );
        if is_branch != cutoff_part_id.is_some() {
            return Err(StoreError::InvalidState(
                "fork/rewind sessions require a cutoff_part_id".to_owned(),
            ));
        }
        if cutoff_part_id.is_some()
            && !self
                .parts
                .read()
                .expect("parts lock")
                .contains_key(&cutoff_part_id.unwrap())
        {
            return Err(StoreError::not_found("cutoff part"));
        }
        let (depth, root_id) = self.parent_lineage(parent_id)?;
        let id = self.next_session_id();
        let now_ms = self.now_ms();
        let mut meta = SessionMeta {
            id,
            parent_id,
            depth,
            root_id,
            workspace_id,
            relation_kind,
            cutoff_part_id,
            title,
            favorite: false,
            pinned: false,
            version: 1,
            lifecycle_state: SessionLifecycleState::Ready,
            creation_failure: None,
            task_id,
            subtask_status: None,
            subtask_started_at_ms: None,
            subtask_finished_at_ms: None,
            subtask_failure: None,
            config_json,
            provider_anchors_json,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        if parent_id.is_none() {
            // Root: root_id = own id (the DB finalizes this via a trigger).
            meta.root_id = id;
        }
        self.sessions
            .write()
            .expect("sessions lock")
            .insert(id, meta.clone());
        Ok(meta)
    }

    async fn session_meta(&self, session_id: i64) -> Result<SessionMeta, StoreError> {
        self.sessions
            .read()
            .expect("sessions lock")
            .get(&session_id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))
    }

    async fn load_session(&self, session_id: i64) -> Result<SessionView, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let parts = self.ordered_parts(session_id);
        Ok(SessionView { meta, parts })
    }

    async fn load_session_page(
        &self,
        session_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let take = usize::try_from(limit.max(1).saturating_add(1)).unwrap_or(usize::MAX);
        let membership = self.membership.read().expect("membership lock");
        let all_parts = self.parts.read().expect("parts lock");
        let mut parts = Vec::with_capacity(take.min(16));
        for part in membership
            .get(&session_id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|id| all_parts.get(id)))
        {
            if before.is_some_and(|before| {
                (part.created_at_ms, part.part_id) >= (before.created_at_ms, before.part_id)
            }) {
                continue;
            }
            if parts.len() < take {
                parts.push(part.clone());
                parts.sort_unstable_by_key(|part| (part.created_at_ms, part.part_id));
            } else if parts.first().is_some_and(|oldest| {
                (part.created_at_ms, part.part_id) > (oldest.created_at_ms, oldest.part_id)
            }) {
                parts[0] = part.clone();
                parts.sort_unstable_by_key(|part| (part.created_at_ms, part.part_id));
            }
        }
        let has_more = parts.len() == take;
        parts.reverse();
        if has_more {
            parts.truncate(take.saturating_sub(1));
        }
        Ok(SessionPartPage {
            meta,
            parts,
            has_more,
        })
    }

    async fn load_run_page(
        &self,
        session_id: i64,
        run_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let take = usize::try_from(limit.max(1).saturating_add(1)).unwrap_or(usize::MAX);
        let membership = self.membership.read().expect("membership lock");
        let all_parts = self.parts.read().expect("parts lock");
        let mut parts = membership
            .get(&session_id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|id| all_parts.get(id)))
            .filter(|part| part.run_id == Some(run_id))
            .filter(|part| {
                before.is_none_or(|before| {
                    (part.created_at_ms, part.part_id) < (before.created_at_ms, before.part_id)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        parts.sort_unstable_by_key(|part| (part.created_at_ms, part.part_id));
        let has_more = parts.len() > usize::try_from(limit.max(1)).unwrap_or(usize::MAX);
        if has_more {
            parts.truncate(usize::try_from(limit.max(1)).unwrap_or(usize::MAX));
        }
        parts.reverse();
        if parts.len() > take.saturating_sub(1) {
            parts.truncate(take.saturating_sub(1));
        }
        Ok(SessionPartPage {
            meta,
            parts,
            has_more,
        })
    }

    async fn newest_member_cursor(
        &self,
        session_id: i64,
    ) -> Result<Option<(i64, i64)>, StoreError> {
        self.session_meta(session_id).await?;
        let (membership, parts) = (
            self.membership.read().expect("membership lock"),
            self.parts.read().expect("parts lock"),
        );
        let newest = membership
            .get(&session_id)
            .into_iter()
            .flat_map(|set| set.iter().copied())
            .filter_map(|id| parts.get(&id))
            .max_by_key(|part| (part.created_at_ms, part.part_id));
        Ok(newest.map(|part| (part.created_at_ms, part.part_id)))
    }

    async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<SessionMeta, StoreError> {
        let now_ms = self.now_ms();
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        meta.title = title;
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(meta.clone())
    }

    async fn update_session_metadata(
        &self,
        session_id: i64,
        patch: SessionMetadataPatch,
    ) -> Result<SessionMeta, StoreError> {
        if patch.is_empty() {
            return Err(StoreError::InvalidState(
                "session metadata patch cannot be empty".to_owned(),
            ));
        }
        let now_ms = self.now_ms();
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        if let Some(title) = patch.title {
            meta.title = title;
        }
        if let Some(favorite) = patch.favorite {
            meta.favorite = favorite;
        }
        if let Some(pinned) = patch.pinned {
            meta.pinned = pinned;
        }
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(meta.clone())
    }

    async fn set_provider_anchors(
        &self,
        session_id: i64,
        anchors: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let now_ms = self.now_ms();
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        meta.provider_anchors_json = anchors;
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(meta.clone())
    }

    async fn set_config_json(
        &self,
        session_id: i64,
        config: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let now_ms = self.now_ms();
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        meta.config_json = config;
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(meta.clone())
    }

    async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionMeta>, StoreError> {
        let sessions = self.sessions.read().expect("sessions lock");
        Ok(sessions
            .values()
            .find(|meta| {
                meta.parent_id == Some(parent_session_id)
                    && meta.task_id.as_deref() == Some(task_id)
            })
            .cloned())
    }

    async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
        now_ms: i64,
    ) -> Result<SessionMeta, StoreError> {
        if self
            .find_subagent_by_task_id(parent_session_id, &task_id)
            .await?
            .is_some()
        {
            return Err(StoreError::InvalidState(format!(
                "subtask '{task_id}' already exists under session {parent_session_id}"
            )));
        }
        // A subagent is a child branch of the parent's root, inheriting its
        // workspace (matches v1 `create_subagent_session` semantics).
        let parent = self
            .sessions
            .read()
            .expect("sessions lock")
            .get(&parent_session_id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("session {parent_session_id}")))?;
        // Depth = parent.depth + 1, matching the SQLite engine and the schema
        // invariant (`NEW.depth = parent.depth + 1`).
        let depth = parent.depth + 1;
        let id = self.next_session_id();
        let meta = SessionMeta {
            id,
            parent_id: Some(parent_session_id),
            depth,
            root_id: parent.root_id,
            workspace_id: parent.workspace_id,
            relation_kind: SessionRelationKind::Subagent,
            cutoff_part_id: None,
            title,
            favorite: false,
            pinned: false,
            version: 1,
            lifecycle_state: SessionLifecycleState::Creating,
            creation_failure: None,
            task_id: Some(task_id),
            // `created` is the initial delegated-task lifecycle (matches the
            // SQLite schema trigger `agena_sessions_subagent_shape`).
            subtask_status: Some("created".to_owned()),
            subtask_started_at_ms: None,
            subtask_finished_at_ms: None,
            subtask_failure: None,
            config_json: None,
            provider_anchors_json: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.sessions
            .write()
            .expect("sessions lock")
            .insert(id, meta.clone());
        Ok(meta)
    }

    async fn update_subtask_state(
        &self,
        session_id: i64,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let now_ms = self.now_ms();
        let mut sessions = self.sessions.write().expect("sessions lock");
        let meta = sessions
            .get_mut(&session_id)
            .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))?;
        meta.subtask_status = status;
        meta.subtask_started_at_ms = started_at_ms;
        meta.subtask_finished_at_ms = finished_at_ms;
        meta.subtask_failure = failure;
        meta.version += 1;
        meta.updated_at_ms = now_ms;
        Ok(meta.clone())
    }

    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let sessions = self.sessions.read().expect("sessions lock");
        let membership = self.membership.read().expect("membership lock");
        let parts = self.parts.read().expect("parts lock");

        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let mut summaries: Vec<SessionSummary> = sessions
            .values()
            .filter(|meta| query.workspace_id.is_none_or(|ws| meta.workspace_id == ws))
            .filter(|meta| {
                query
                    .parent_id
                    .is_none_or(|parent_id| meta.parent_id == Some(parent_id))
            })
            .filter(|meta| !query.roots_only || meta.parent_id.is_none())
            .filter(|meta| !query.exclude_subagents || !meta.relation_kind.is_subagent())
            .filter(|meta| {
                search
                    .as_ref()
                    .is_none_or(|needle| meta.title.to_lowercase().contains(needle))
            })
            .map(|meta| {
                let message_count = membership
                    .get(&meta.id)
                    .into_iter()
                    .flat_map(|ids| ids.iter())
                    .filter(|id| parts.get(id).is_some_and(|part| part.is_run_marker()))
                    .count() as i64;
                let child_session_count = sessions
                    .values()
                    .filter(|other| other.parent_id == Some(meta.id))
                    .count() as i64;
                let last_message_at_ms = membership
                    .get(&meta.id)
                    .into_iter()
                    .flat_map(|ids| ids.iter().filter_map(|id| parts.get(id)))
                    .map(|part| part.created_at_ms)
                    .max();
                SessionSummary {
                    id: meta.id,
                    workspace_id: meta.workspace_id,
                    parent_id: meta.parent_id,
                    depth: meta.depth,
                    root_id: meta.root_id,
                    title: meta.title.clone(),
                    favorite: meta.favorite,
                    pinned: meta.pinned,
                    relation_kind: meta.relation_kind,
                    lifecycle_state: meta.lifecycle_state,
                    version: meta.version,
                    task_id: meta.task_id.clone(),
                    subtask_status: meta.subtask_status.clone(),
                    message_count,
                    child_session_count,
                    last_message_at_ms,
                    created_at_ms: meta.created_at_ms,
                    updated_at_ms: meta.updated_at_ms,
                }
            })
            .collect();
        summaries.sort_by_key(|s| (std::cmp::Reverse(s.updated_at_ms), std::cmp::Reverse(s.id)));
        if let Some(before) = query.before {
            summaries.retain(|s| (s.updated_at_ms, s.id) < (before.updated_at_ms, before.id));
        }
        if let Some(limit) = query.limit {
            summaries.truncate(limit.max(0) as usize);
        }
        Ok(summaries)
    }

    async fn session_states(
        &self,
        session_ids: &[i64],
        now_ms: i64,
    ) -> Result<HashMap<i64, SessionState>, StoreError> {
        let mut states = HashMap::with_capacity(session_ids.len());
        for &session_id in session_ids {
            let meta = self
                .sessions
                .read()
                .expect("sessions lock")
                .get(&session_id)
                .cloned();
            let Some(meta) = meta else {
                continue;
            };
            let lease = self
                .leases
                .read()
                .expect("leases lock")
                .get(&session_id)
                .cloned();
            let view = SessionView {
                meta: meta.clone(),
                parts: self.ordered_parts(session_id),
            };
            let inputs = StateInputs::from_view(&view);
            states.insert(
                session_id,
                derive_session_state(
                    Some(&meta),
                    &inputs.in_flight_runs,
                    &inputs.pending_interactions,
                    lease.as_ref(),
                    now_ms,
                ),
            );
        }
        Ok(states)
    }

    async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, StoreError> {
        // Build a summary for the single session (mirrors the row projection).
        let sessions = self.sessions.read().expect("sessions lock");
        let membership = self.membership.read().expect("membership lock");
        let parts = self.parts.read().expect("parts lock");
        let Some(meta) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let message_count = membership
            .get(&meta.id)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter(|id| parts.get(id).is_some_and(|part| part.is_run_marker()))
            .count() as i64;
        let child_session_count = sessions
            .values()
            .filter(|other| other.parent_id == Some(meta.id))
            .count() as i64;
        let last_message_at_ms = membership
            .get(&meta.id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|id| parts.get(id)))
            .map(|part| part.created_at_ms)
            .max();
        Ok(Some(SessionSummary {
            id: meta.id,
            workspace_id: meta.workspace_id,
            parent_id: meta.parent_id,
            depth: meta.depth,
            root_id: meta.root_id,
            title: meta.title.clone(),
            favorite: meta.favorite,
            pinned: meta.pinned,
            relation_kind: meta.relation_kind,
            lifecycle_state: meta.lifecycle_state,
            version: meta.version,
            task_id: meta.task_id.clone(),
            subtask_status: meta.subtask_status.clone(),
            message_count,
            child_session_count,
            last_message_at_ms,
            created_at_ms: meta.created_at_ms,
            updated_at_ms: meta.updated_at_ms,
        }))
    }

    async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, StoreError> {
        let sessions = self.sessions.read().expect("sessions lock");
        let wanted = workspace_ids.iter().copied().collect::<BTreeSet<_>>();
        let mut counts = HashMap::with_capacity(workspace_ids.len());
        for workspace_id in &wanted {
            counts.insert(*workspace_id, 0);
        }
        for meta in sessions.values() {
            if let Some(count) = counts.get_mut(&meta.workspace_id) {
                *count += 1;
            }
        }
        Ok(counts)
    }

    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError> {
        let sessions = self.sessions.read().expect("sessions lock");
        let membership = self.membership.read().expect("membership lock");
        let parts = self.parts.read().expect("parts lock");
        let mut summaries: Vec<SessionSummary> = sessions
            .values()
            .filter(|meta| meta.root_id == root_id)
            .map(|meta| {
                let message_count = membership
                    .get(&meta.id)
                    .into_iter()
                    .flat_map(|ids| ids.iter())
                    .filter(|id| parts.get(id).is_some_and(|part| part.is_run_marker()))
                    .count() as i64;
                let child_session_count = sessions
                    .values()
                    .filter(|other| other.parent_id == Some(meta.id))
                    .count() as i64;
                let last_message_at_ms = membership
                    .get(&meta.id)
                    .into_iter()
                    .flat_map(|ids| ids.iter().filter_map(|id| parts.get(id)))
                    .map(|part| part.created_at_ms)
                    .max();
                SessionSummary {
                    id: meta.id,
                    workspace_id: meta.workspace_id,
                    parent_id: meta.parent_id,
                    depth: meta.depth,
                    root_id: meta.root_id,
                    title: meta.title.clone(),
                    favorite: meta.favorite,
                    pinned: meta.pinned,
                    relation_kind: meta.relation_kind,
                    lifecycle_state: meta.lifecycle_state,
                    version: meta.version,
                    task_id: meta.task_id.clone(),
                    subtask_status: meta.subtask_status.clone(),
                    message_count,
                    child_session_count,
                    last_message_at_ms,
                    created_at_ms: meta.created_at_ms,
                    updated_at_ms: meta.updated_at_ms,
                }
            })
            .collect();
        summaries.sort_by_key(|s| (std::cmp::Reverse(s.updated_at_ms), std::cmp::Reverse(s.id)));
        Ok(summaries)
    }

    async fn delete_session(&self, session_id: i64) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().expect("sessions lock");
        let mut membership = self.membership.write().expect("membership lock");
        // Recursive descendant cascade (matches ON DELETE CASCADE on parent_id).
        let mut to_delete = vec![session_id];
        let mut index = 0;
        while index < to_delete.len() {
            let current = to_delete[index];
            for child_id in sessions
                .values()
                .filter(|meta| meta.parent_id == Some(current))
                .map(|meta| meta.id)
                .collect::<Vec<_>>()
            {
                to_delete.push(child_id);
            }
            index += 1;
        }
        for id in &to_delete {
            sessions.remove(id);
            membership.remove(id);
        }
        let _ = &mut membership;
        Ok(())
    }

    async fn try_acquire_lease(
        &self,
        session_id: i64,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<LeaseAcquire, StoreError> {
        if !self
            .sessions
            .read()
            .expect("sessions lock")
            .contains_key(&session_id)
        {
            return Err(StoreError::not_found(format!("session {session_id}")));
        }
        let mut leases = self.leases.write().expect("leases lock");
        if let Some(existing) = leases.get(&session_id) {
            if now_ms - existing.heartbeat_at_ms <= LEASE_STALENESS_MS {
                return Ok(LeaseAcquire::HeldBy {
                    owner_id: existing.owner_id.clone(),
                    heartbeat_at_ms: existing.heartbeat_at_ms,
                });
            }
            // Stale: steal atomically — take the lease and abort the residual
            // in-flight run markers in the same critical section (invariant 2).
            let aborted = self
                .in_flight_runs(session_id)
                .iter()
                .map(|run| run.part_id)
                .collect::<Vec<_>>();
            let outcome = self.abort_runs(session_id, &aborted, "lease_stolen", now_ms)?;
            leases.insert(
                session_id,
                LeaseState {
                    session_id,
                    owner_id: owner_id.to_owned(),
                    run_id: None,
                    lease_started_at_ms: now_ms,
                    heartbeat_at_ms: now_ms,
                },
            );
            return Ok(LeaseAcquire::Acquired {
                reconciled_runs: outcome.aborted_runs,
                updated_parts: outcome.updated_parts,
            });
        }
        leases.insert(
            session_id,
            LeaseState {
                session_id,
                owner_id: owner_id.to_owned(),
                run_id: None,
                lease_started_at_ms: now_ms,
                heartbeat_at_ms: now_ms,
            },
        );
        Ok(LeaseAcquire::Acquired {
            reconciled_runs: Vec::new(),
            updated_parts: Vec::new(),
        })
    }

    async fn heartbeat_lease(
        &self,
        session_id: i64,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let mut leases = self.leases.write().expect("leases lock");
        let Some(lease) = leases.get_mut(&session_id) else {
            return Ok(false);
        };
        if lease.owner_id != owner_id {
            return Ok(false);
        }
        lease.heartbeat_at_ms = now_ms;
        Ok(true)
    }

    async fn release_lease(&self, session_id: i64, owner_id: &str) -> Result<bool, StoreError> {
        let mut leases = self.leases.write().expect("leases lock");
        if leases
            .get(&session_id)
            .is_some_and(|lease| lease.owner_id == owner_id)
        {
            leases.remove(&session_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn current_lease(&self, session_id: i64) -> Result<Option<LeaseState>, StoreError> {
        Ok(self
            .leases
            .read()
            .expect("leases lock")
            .get(&session_id)
            .cloned())
    }

    async fn reap_stale_leases(&self, stale_before_ms: i64) -> Result<Vec<i64>, StoreError> {
        let mut leases = self.leases.write().expect("leases lock");
        let stale: Vec<i64> = leases
            .iter()
            .filter(|(_, lease)| lease.heartbeat_at_ms < stale_before_ms)
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            leases.remove(id);
        }
        Ok(stale)
    }

    async fn create_background_operation(
        &self,
        new: NewBackgroundOperation,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = self
            .background_operations
            .read()
            .expect("background operations lock")
            .get(&new.operation_id)
            .cloned()
        {
            if existing.session_id == new.session_id
                && existing.launch_run_id == new.launch_run_id
                && existing.launch_tool_part_id == new.launch_tool_part_id
                && existing.kind == new.kind
            {
                return Ok(existing);
            }
            return Err(StoreError::InvalidState(format!(
                "background operation {} already identifies a different launch",
                new.operation_id
            )));
        }
        if !self
            .sessions
            .read()
            .expect("sessions lock")
            .contains_key(&new.session_id)
        {
            return Err(StoreError::not_found(format!("session {}", new.session_id)));
        }
        if new.kind != super::BackgroundOperationKind::ScheduledDelivery
            && let Some(tool_part_id) = new.launch_tool_part_id
            && let Some(existing) = self
                .background_operations
                .read()
                .expect("background operations lock")
                .values()
                .find(|operation| {
                    operation.session_id == new.session_id
                        && operation.launch_tool_part_id == Some(tool_part_id)
                        && operation.kind != super::BackgroundOperationKind::ScheduledDelivery
                })
        {
            return Err(StoreError::InvalidState(format!(
                "tool part {tool_part_id} already owns background operation {}",
                existing.operation_id
            )));
        }
        match (new.launch_run_id, new.launch_tool_part_id) {
            (None, None) if new.kind == super::BackgroundOperationKind::ScheduledDelivery => {}
            (Some(run_id), Some(tool_part_id)) => {
                let parts = self.parts.read().expect("parts lock");
                let run = parts
                    .get(&run_id)
                    .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
                if !run.is_run_marker() || run.origin_session_id != new.session_id {
                    return Err(StoreError::InvalidState(format!(
                        "background launch run {run_id} does not belong to session {}",
                        new.session_id
                    )));
                }
                let tool = parts
                    .get(&tool_part_id)
                    .ok_or_else(|| StoreError::not_found(format!("tool part {tool_part_id}")))?;
                if tool.kind != "tool_call"
                    || tool.origin_session_id != new.session_id
                    || tool.run_id != Some(run_id)
                {
                    return Err(StoreError::InvalidState(format!(
                        "background launch tool {tool_part_id} is not owned by run {run_id} in session {}",
                        new.session_id
                    )));
                }
            }
            _ => {
                return Err(StoreError::InvalidState(
                    "background launch ids must be paired, and non-scheduled operations require them"
                        .to_owned(),
                ));
            }
        }
        let operation = BackgroundOperation {
            operation_id: new.operation_id.clone(),
            session_id: new.session_id,
            launch_run_id: new.launch_run_id,
            launch_tool_part_id: new.launch_tool_part_id,
            kind: new.kind,
            external_id: None,
            phase: BackgroundOperationPhase::LaunchRequested,
            outcome: None,
            failure: None,
            last_event_seq: 0,
            owner_id: None,
            lease_until_ms: None,
            revision: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            finished_at_ms: None,
        };
        self.background_operations
            .write()
            .expect("background operations lock")
            .insert(new.operation_id, operation.clone());
        Ok(operation)
    }

    async fn background_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError> {
        Ok(self
            .background_operations
            .read()
            .expect("background operations lock")
            .get(operation_id)
            .cloned())
    }

    async fn background_operation_by_external_id(
        &self,
        kind: super::BackgroundOperationKind,
        external_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError> {
        Ok(self
            .background_operations
            .read()
            .expect("background operations lock")
            .values()
            .find(|operation| {
                operation.kind == kind && operation.external_id.as_deref() == Some(external_id)
            })
            .cloned())
    }

    async fn active_background_operations(
        &self,
        kind: Option<super::BackgroundOperationKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundOperation>, StoreError> {
        let mut operations = self
            .background_operations
            .read()
            .expect("background operations lock")
            .values()
            .filter(|operation| {
                !operation.phase.is_terminal() && kind.is_none_or(|kind| operation.kind == kind)
            })
            .cloned()
            .collect::<Vec<_>>();
        operations
            .sort_by_key(|operation| (operation.created_at_ms, operation.operation_id.clone()));
        operations.truncate(limit);
        Ok(operations)
    }

    async fn transition_background_operation(
        &self,
        transition: BackgroundOperationTransition,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut operations = self
            .background_operations
            .write()
            .expect("background operations lock");
        let current = operations
            .get(&transition.operation_id)
            .cloned()
            .ok_or_else(|| {
                StoreError::not_found(format!("background operation {}", transition.operation_id))
            })?;
        if current.revision != transition.expected_revision {
            return Err(StoreError::InvalidState(format!(
                "background operation {} revision changed: expected {}, found {}",
                current.operation_id, transition.expected_revision, current.revision
            )));
        }
        if !current.phase.can_transition(transition.next_phase) {
            return Err(StoreError::InvalidState(format!(
                "invalid background transition {} -> {} for {}",
                current.phase.as_str(),
                transition.next_phase.as_str(),
                current.operation_id
            )));
        }
        if let Some(external_id) = transition.external_id.as_deref()
            && operations.values().any(|operation| {
                operation.operation_id != current.operation_id
                    && operation.kind == current.kind
                    && operation.external_id.as_deref() == Some(external_id)
            })
        {
            return Err(StoreError::InvalidState(format!(
                "background external id {}:{} already exists",
                current.kind.as_str(),
                external_id
            )));
        }
        let mut next = current;
        next.phase = transition.next_phase;
        if transition.external_id.is_some() {
            next.external_id = transition.external_id;
        }
        if transition.outcome.is_some() {
            next.outcome = transition.outcome;
        }
        if transition.failure.is_some() {
            next.failure = transition.failure;
        }
        next.owner_id = transition.owner_id;
        next.lease_until_ms = transition.lease_until_ms;
        next.revision += 1;
        next.updated_at_ms = now_ms;
        next.finished_at_ms = next.phase.is_terminal().then_some(now_ms);
        if next.phase == BackgroundOperationPhase::Running && next.external_id.is_none() {
            return Err(StoreError::InvalidState(format!(
                "background operation {} cannot enter running without an external id",
                next.operation_id
            )));
        }
        operations.insert(next.operation_id.clone(), next.clone());
        Ok(next)
    }

    async fn record_background_event(
        &self,
        request: BackgroundEventRequest,
        now_ms: i64,
    ) -> Result<BackgroundSettleOutcome, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let delivery_id = format!("{}:{}", request.operation_id, request.event_key);
        if let Some(delivery) = self
            .background_deliveries
            .read()
            .expect("background deliveries lock")
            .get(&delivery_id)
            .cloned()
        {
            let operation = self
                .background_operations
                .read()
                .expect("background operations lock")
                .get(&request.operation_id)
                .cloned()
                .ok_or_else(|| {
                    StoreError::not_found(format!("background operation {}", request.operation_id))
                })?;
            let notification_part_id = delivery.notification_part_id.ok_or_else(|| {
                StoreError::InvalidState(format!(
                    "background delivery {} has no notification part",
                    delivery.delivery_id
                ))
            })?;
            let notification_part = self
                .parts
                .read()
                .expect("parts lock")
                .get(&notification_part_id)
                .cloned()
                .ok_or_else(|| StoreError::not_found(format!("part {notification_part_id}")))?;
            return Ok(BackgroundSettleOutcome {
                operation,
                delivery,
                notification_part,
                created: false,
            });
        }
        let current = self
            .background_operations
            .read()
            .expect("background operations lock")
            .get(&request.operation_id)
            .cloned()
            .ok_or_else(|| {
                StoreError::not_found(format!("background operation {}", request.operation_id))
            })?;
        if let Some(next_phase) = request.next_phase
            && !current.phase.can_transition(next_phase)
        {
            return Err(StoreError::InvalidState(format!(
                "invalid background transition {} -> {} for {}",
                current.phase.as_str(),
                next_phase.as_str(),
                current.operation_id
            )));
        }
        let mut notification = request.notification;
        notification.state = PartState::Completed;
        let serde_json::Value::Object(notification_content) = &mut notification.content else {
            return Err(StoreError::InvalidState(
                "background notification content must be a JSON object".to_owned(),
            ));
        };
        notification_content.insert(
            "delivery_protocol".to_owned(),
            serde_json::Value::String("provider_round_v1".to_owned()),
        );
        let notification_part = if let Some(run_id) = current.launch_run_id {
            let run = self
                .parts
                .read()
                .expect("parts lock")
                .get(&run_id)
                .cloned()
                .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
            if !run.is_run_marker()
                || run.origin_session_id != current.session_id
                || run.role != PartRole::Assistant
            {
                return Err(StoreError::InvalidState(format!(
                    "background operation {} launch run {run_id} is not an assistant run owned by session {}",
                    current.operation_id, current.session_id
                )));
            }
            notification.role = PartRole::Assistant;
            let id = self.next_part_id();
            let part = Part {
                part_id: id,
                kind: notification.kind,
                role: notification.role,
                state: notification.state,
                content: notification.content,
                summary: notification.summary,
                visibility: notification.visibility,
                rendered_markdown: notification.rendered_markdown,
                parent_part_id: notification.parent_part_id,
                run_id: Some(run_id),
                origin_session_id: current.session_id,
                revision: 1,
                started_at_ms: now_ms,
                finished_at_ms: Some(now_ms),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                provider_state: None,
            };
            self.parts
                .write()
                .expect("parts lock")
                .insert(id, part.clone());
            self.membership
                .write()
                .expect("membership lock")
                .entry(current.session_id)
                .or_default()
                .insert(id);
            self.bump_session_version(current.session_id, now_ms)?;
            part
        } else {
            notification.role = PartRole::Runtime;
            let projected = self.create_batch(
                current.session_id,
                PartRole::Runtime,
                PartState::Completed,
                json!({
                    "run_kind": "runtime_ingress",
                    "source": "background_operation",
                    "operation_id": current.operation_id,
                    "abort_reason": null,
                }),
                vec![notification],
                Some(delivery_id.clone()),
                now_ms,
            )?;
            projected.parts.get(1).cloned().ok_or_else(|| {
                StoreError::InvalidState("runtime ingress omitted notification".into())
            })?
        };
        let mut next = current;
        if let Some(next_phase) = request.next_phase {
            next.phase = next_phase;
        }
        if request.outcome.is_some() {
            next.outcome = request.outcome;
        }
        if request.failure.is_some() {
            next.failure = request.failure;
        }
        if let Some(event_seq) = request.event_seq {
            next.last_event_seq = next.last_event_seq.max(event_seq);
        }
        if next.phase.is_terminal() {
            next.owner_id = None;
            next.lease_until_ms = None;
        }
        next.revision += 1;
        next.updated_at_ms = now_ms;
        next.finished_at_ms = if next.phase.is_terminal() {
            next.finished_at_ms.or(Some(now_ms))
        } else {
            None
        };
        let delivery = BackgroundDelivery {
            delivery_id: delivery_id.clone(),
            operation_id: next.operation_id.clone(),
            session_id: next.session_id,
            event_key: request.event_key,
            payload: notification_part.content.clone(),
            phase: BackgroundDeliveryPhase::Pending,
            claim_owner: None,
            claim_until_ms: None,
            attempts: 0,
            notification_part_id: Some(notification_part.part_id),
            last_error: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            consumed_at_ms: None,
            next_attempt_at_ms: 0,
        };
        self.background_operations
            .write()
            .expect("background operations lock")
            .insert(next.operation_id.clone(), next.clone());
        self.background_deliveries
            .write()
            .expect("background deliveries lock")
            .insert(delivery_id, delivery.clone());
        Ok(BackgroundSettleOutcome {
            operation: next,
            delivery,
            notification_part,
            created: true,
        })
    }

    async fn claim_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        claim_until_ms: i64,
        now_ms: i64,
    ) -> Result<Option<BackgroundDelivery>, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut deliveries = self
            .background_deliveries
            .write()
            .expect("background deliveries lock");
        let Some(delivery) = deliveries.get_mut(delivery_id) else {
            return Ok(None);
        };
        let claimable = (delivery.phase == BackgroundDeliveryPhase::Pending
            && delivery.next_attempt_at_ms <= now_ms)
            || (delivery.phase == BackgroundDeliveryPhase::Claimed
                && delivery.claim_until_ms.is_some_and(|until| until <= now_ms));
        if !claimable {
            return Ok(None);
        }
        delivery.phase = BackgroundDeliveryPhase::Claimed;
        delivery.claim_owner = Some(owner_id.to_owned());
        delivery.claim_until_ms = Some(claim_until_ms);
        delivery.attempts = delivery.attempts.saturating_add(1);
        delivery.updated_at_ms = now_ms;
        Ok(Some(delivery.clone()))
    }

    async fn consume_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut deliveries = self
            .background_deliveries
            .write()
            .expect("background deliveries lock");
        let delivery = deliveries
            .get_mut(delivery_id)
            .ok_or_else(|| StoreError::not_found(format!("delivery {delivery_id}")))?;
        if matches!(
            delivery.phase,
            BackgroundDeliveryPhase::Consumed | BackgroundDeliveryPhase::Failed
        ) {
            return Ok(delivery.clone());
        }
        if delivery.phase != BackgroundDeliveryPhase::Claimed
            || delivery.claim_owner.as_deref() != Some(owner_id)
        {
            return Err(StoreError::InvalidState(format!(
                "background delivery {delivery_id} is not claimed by {owner_id}"
            )));
        }
        delivery.phase = BackgroundDeliveryPhase::Consumed;
        delivery.claim_owner = None;
        delivery.claim_until_ms = None;
        delivery.updated_at_ms = now_ms;
        delivery.consumed_at_ms = Some(now_ms);
        delivery.next_attempt_at_ms = now_ms;
        Ok(delivery.clone())
    }

    async fn retry_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: Value,
        next_attempt_at_ms: i64,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut deliveries = self
            .background_deliveries
            .write()
            .expect("background deliveries lock");
        let delivery = deliveries
            .get_mut(delivery_id)
            .ok_or_else(|| StoreError::not_found(format!("delivery {delivery_id}")))?;
        if delivery.phase != BackgroundDeliveryPhase::Claimed
            || delivery.claim_owner.as_deref() != Some(owner_id)
        {
            return Err(StoreError::InvalidState(format!(
                "background delivery {delivery_id} is not claimed by {owner_id}"
            )));
        }
        delivery.phase = BackgroundDeliveryPhase::Pending;
        delivery.claim_owner = None;
        delivery.claim_until_ms = None;
        delivery.last_error = Some(error);
        delivery.updated_at_ms = now_ms;
        delivery.next_attempt_at_ms = next_attempt_at_ms;
        Ok(delivery.clone())
    }

    async fn fail_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: Value,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut deliveries = self
            .background_deliveries
            .write()
            .expect("background deliveries lock");
        let delivery = deliveries
            .get_mut(delivery_id)
            .ok_or_else(|| StoreError::not_found(format!("delivery {delivery_id}")))?;
        if matches!(
            delivery.phase,
            BackgroundDeliveryPhase::Consumed | BackgroundDeliveryPhase::Failed
        ) {
            return Ok(delivery.clone());
        }
        if delivery.phase != BackgroundDeliveryPhase::Claimed
            || delivery.claim_owner.as_deref() != Some(owner_id)
        {
            return Err(StoreError::InvalidState(format!(
                "background delivery {delivery_id} is not claimed by {owner_id}"
            )));
        }
        delivery.phase = BackgroundDeliveryPhase::Failed;
        delivery.claim_owner = None;
        delivery.claim_until_ms = None;
        delivery.last_error = Some(error);
        delivery.updated_at_ms = now_ms;
        delivery.next_attempt_at_ms = now_ms;
        Ok(delivery.clone())
    }

    async fn fail_pending_background_deliveries(
        &self,
        session_id: i64,
        error: Value,
        now_ms: i64,
    ) -> Result<usize, StoreError> {
        let _write = self
            .background_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut deliveries = self
            .background_deliveries
            .write()
            .expect("background deliveries lock");
        let mut changed = 0;
        for delivery in deliveries.values_mut().filter(|delivery| {
            delivery.session_id == session_id
                && matches!(
                    delivery.phase,
                    BackgroundDeliveryPhase::Pending | BackgroundDeliveryPhase::Claimed
                )
        }) {
            delivery.phase = BackgroundDeliveryPhase::Failed;
            delivery.claim_owner = None;
            delivery.claim_until_ms = None;
            delivery.last_error = Some(error.clone());
            delivery.updated_at_ms = now_ms;
            delivery.next_attempt_at_ms = now_ms;
            changed += 1;
        }
        Ok(changed)
    }

    async fn pending_background_deliveries(
        &self,
        limit: usize,
        now_ms: i64,
    ) -> Result<Vec<BackgroundDelivery>, StoreError> {
        let mut deliveries = self
            .background_deliveries
            .read()
            .expect("background deliveries lock")
            .values()
            .filter(|delivery| {
                (delivery.phase == BackgroundDeliveryPhase::Pending
                    && delivery.next_attempt_at_ms <= now_ms)
                    || (delivery.phase == BackgroundDeliveryPhase::Claimed
                        && delivery.claim_until_ms.is_some_and(|until| until <= now_ms))
            })
            .cloned()
            .collect::<Vec<_>>();
        deliveries.sort_by_key(|delivery| (delivery.created_at_ms, delivery.delivery_id.clone()));
        deliveries.truncate(limit);
        Ok(deliveries)
    }

    async fn submit_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let marker_state = if parts.iter().all(|part| part.state.is_terminal()) {
            PartState::Completed
        } else {
            PartState::Pending
        };
        self.create_batch(
            session_id,
            PartRole::User,
            marker_state,
            user_send_marker_content(None),
            parts,
            idempotency_key,
            now_ms,
        )
    }

    async fn submit_user_run_for_execution(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        execution_id: &str,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let marker_state = if parts.iter().all(|part| part.state.is_terminal()) {
            PartState::Completed
        } else {
            PartState::Pending
        };
        self.create_batch(
            session_id,
            PartRole::User,
            marker_state,
            user_send_marker_content(Some(execution_id)),
            parts,
            idempotency_key,
            now_ms,
        )
    }

    async fn settle_background_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        tool_part: Option<(i64, PartState, serde_json::Value)>,
        new_parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        // Lease refresh (see the trait doc): a stale lease (held by this
        // owner or another) is re-heartbeated so the settle may write; a fresh
        // lease held by another owner is a live conflict. Other in-flight runs
        // are deliberately NOT aborted — the settle targets one specific
        // launching run and must never destroy a different run a live
        // execution is still driving.
        {
            let mut leases = self.leases.write().expect("leases lock");
            if let Some(existing) = leases.get(&session_id) {
                if existing.owner_id != owner_id
                    && now_ms - existing.heartbeat_at_ms <= LEASE_STALENESS_MS
                {
                    return Err(StoreError::LeaseHeldByOther {
                        session_id,
                        owner_id: existing.owner_id.clone(),
                        heartbeat_at_ms: existing.heartbeat_at_ms,
                    });
                }
                leases.insert(
                    session_id,
                    LeaseState {
                        session_id,
                        owner_id: owner_id.to_owned(),
                        run_id: None,
                        lease_started_at_ms: now_ms,
                        heartbeat_at_ms: now_ms,
                    },
                );
            } else {
                leases.insert(
                    session_id,
                    LeaseState {
                        session_id,
                        owner_id: owner_id.to_owned(),
                        run_id: None,
                        lease_started_at_ms: now_ms,
                        heartbeat_at_ms: now_ms,
                    },
                );
            }
        }
        // Transition the launching tool part when supplied. An InProgress
        // transition is the atomic background-launch checkpoint; terminal
        // transitions settle the operation.
        if let Some((part_id, next_state, content)) = tool_part {
            let mut part = self
                .parts
                .write()
                .expect("parts lock")
                .get_mut(&part_id)
                .cloned()
                .ok_or_else(|| StoreError::not_found(format!("part {part_id}")))?;
            part.state = next_state;
            part.content = content;
            part.finished_at_ms = next_state.is_terminal().then_some(now_ms);
            part.revision += 1;
            part.updated_at_ms = now_ms;
            self.parts
                .write()
                .expect("parts lock")
                .insert(part_id, part);
        }
        // Append companion parts under the launching run — the launch guard
        // or settled notifications, with their supplied roles — and create no
        // new run marker.
        let mut created = Vec::with_capacity(new_parts.len());
        {
            let mut all_parts = self.parts.write().expect("parts lock");
            let mut membership = self.membership.write().expect("membership lock");
            for new_part in new_parts {
                let id = self.next_part_id();
                let part = Part {
                    part_id: id,
                    kind: new_part.kind,
                    role: new_part.role,
                    state: new_part.state,
                    content: new_part.content,
                    summary: new_part.summary,
                    visibility: new_part.visibility,
                    rendered_markdown: new_part.rendered_markdown,
                    parent_part_id: new_part.parent_part_id,
                    run_id: Some(run_id),
                    origin_session_id: session_id,
                    revision: 1,
                    started_at_ms: now_ms,
                    finished_at_ms: new_part.state.is_terminal().then_some(now_ms),
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    provider_state: None,
                };
                all_parts.insert(id, part.clone());
                membership.entry(session_id).or_default().insert(id);
                created.push(part);
            }
        }
        // Terminalize the launching run marker once no in-flight child remains.
        let mut run = self
            .parts
            .read()
            .expect("parts lock")
            .get(&run_id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
        if run.is_run_marker() && run.state.is_in_flight() {
            let remaining = self.parts.read().expect("parts lock").values().any(|part| {
                part.origin_session_id == session_id
                    && part.run_id == Some(run_id)
                    && part.part_id != run_id
                    && part.state.is_in_flight()
            });
            if !remaining {
                run.state = PartState::Completed;
                run.finished_at_ms = Some(now_ms);
                if let serde_json::Value::Object(map) = &mut run.content {
                    map.insert("abort_reason".to_owned(), serde_json::Value::Null);
                }
                run.revision += 1;
                run.updated_at_ms = now_ms;
                self.parts.write().expect("parts lock").insert(run_id, run);
            }
        }
        if !created.is_empty() {
            self.bump_session_version(session_id, now_ms)?;
        }
        Ok(created)
    }

    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let run = self
            .parts
            .read()
            .expect("parts lock")
            .get(&run_id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
        if !run.is_run_marker() || !run.state.is_in_flight() {
            return Err(StoreError::InvalidState(format!(
                "run {run_id} is not an in-flight run marker"
            )));
        }
        let mut created = Vec::with_capacity(parts.len());
        {
            let mut all_parts = self.parts.write().expect("parts lock");
            let mut membership = self.membership.write().expect("membership lock");
            for new_part in parts {
                let id = self.next_part_id();
                let part = Part {
                    part_id: id,
                    kind: new_part.kind,
                    role: new_part.role,
                    state: new_part.state,
                    content: new_part.content,
                    summary: new_part.summary,
                    visibility: new_part.visibility,
                    rendered_markdown: new_part.rendered_markdown,
                    parent_part_id: new_part.parent_part_id,
                    run_id: Some(run_id),
                    origin_session_id: session_id,
                    revision: 1,
                    started_at_ms: now_ms,
                    finished_at_ms: new_part.state.is_terminal().then_some(now_ms),
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    provider_state: None,
                };
                all_parts.insert(id, part.clone());
                membership.entry(session_id).or_default().insert(id);
                created.push(part);
            }
        }
        if !created.is_empty() {
            self.bump_session_version(session_id, now_ms)?;
        }
        Ok(created)
    }

    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
        now_ms: i64,
    ) -> Result<Part, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let updated = {
            let mut parts = self.parts.write().expect("parts lock");
            let part = parts
                .get_mut(&part_id)
                .ok_or_else(|| StoreError::not_found(format!("part {part_id}")))?;
            // Shared-part rule (8.4): only the creating session updates in place.
            if part.origin_session_id != session_id {
                return Err(StoreError::InvalidState(format!(
                    "part {part_id} is shared; only its origin session {session_id} may update it in place"
                )));
            }
            if let Some(to) = delta.state {
                apply_part_transition(part, to, now_ms, true)?;
            }
            if let Some(content) = delta.content {
                part.content = content;
            } else if let Some(delta_text) = delta.content_text_delta {
                append_text_delta(&mut part.content, &delta_text)?;
            }
            if let Some(summary) = delta.summary {
                part.summary = Some(summary);
            }
            if let Some(rendered) = delta.rendered_markdown {
                part.rendered_markdown = Some(rendered);
            }
            if let Some(provider_state) = delta.provider_state {
                part.provider_state = Some(provider_state);
            }
            if let Some(finished) = delta.finished_at_ms {
                part.finished_at_ms = Some(finished);
            }
            if part.state.is_terminal() && part.finished_at_ms.is_none() {
                part.finished_at_ms = Some(now_ms);
            }
            if part.state == PartState::InProgress {
                // Retry clears the finished timestamp.
                part.finished_at_ms = None;
            }
            part.revision += 1;
            part.updated_at_ms = now_ms;
            part.clone()
        };
        self.bump_member_session_versions(&[part_id], now_ms)?;
        Ok(updated)
    }

    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
        now_ms: i64,
    ) -> Result<Part, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        if !outcome.status.is_terminal() {
            return Err(StoreError::InvalidState(
                "complete_run requires a terminal outcome".to_owned(),
            ));
        }
        if matches!(outcome.status, PartState::Failed | PartState::Cancelled)
            && outcome.abort_reason.is_none()
        {
            return Err(StoreError::InvalidState(
                "terminal run markers require an abort_reason".to_owned(),
            ));
        }
        let updated = {
            let mut parts = self.parts.write().expect("parts lock");
            let part = parts
                .get_mut(&run_id)
                .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
            if !part.is_run_marker() {
                return Err(StoreError::InvalidState(format!(
                    "part {run_id} is not a run marker"
                )));
            }
            if part.origin_session_id != session_id {
                return Err(StoreError::InvalidState(format!(
                    "run marker {run_id} is shared; only its origin session may complete it"
                )));
            }
            let mut content = outcome.content.unwrap_or_else(|| part.content.clone());
            if let Value::Object(map) = &mut content {
                map.insert(
                    "abort_reason".to_owned(),
                    match outcome.abort_reason {
                        Some(reason) => Value::String(reason),
                        None => Value::Null,
                    },
                );
            }
            part.content = content;
            part.state = outcome.status;
            part.finished_at_ms = Some(now_ms);
            if let Some(provider_state) = outcome.provider_state {
                part.provider_state = Some(provider_state);
            }
            part.revision += 1;
            part.updated_at_ms = now_ms;
            part.clone()
        };
        self.bump_member_session_versions(&[run_id], now_ms)?;
        Ok(updated)
    }

    async fn start_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_kind: &str,
        content: Value,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let role = match run_kind {
            "user_send" => PartRole::User,
            "continue" | "compaction" | "steer" | "execution" => PartRole::Assistant,
            _ => PartRole::Runtime,
        };
        let mut marker_content = content;
        if let Value::Object(map) = &mut marker_content {
            map.insert("run_kind".to_owned(), Value::String(run_kind.to_owned()));
        }
        self.create_batch(
            session_id,
            role,
            PartState::Pending,
            marker_content,
            Vec::new(),
            idempotency_key,
            now_ms,
        )
    }

    async fn cancel_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        Ok(self
            .abort_runs(session_id, &[run_id], "user_cancelled", now_ms)?
            .updated_parts)
    }

    async fn withdraw_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;

        let member_ids = self
            .membership
            .read()
            .expect("membership lock")
            .get(&session_id)
            .cloned()
            .unwrap_or_default();
        if !member_ids.contains(&run_id) {
            // A repeated withdrawal, or a run that was already removed from
            // this projection, is intentionally idempotent.
            return Ok(Vec::new());
        }

        let parts = self.parts.read().expect("parts lock");
        let marker = parts
            .get(&run_id)
            .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
        if !marker.is_run_marker()
            || marker.role != PartRole::User
            || marker.origin_session_id != session_id
            || marker.content.get("run_kind").and_then(Value::as_str) != Some("user_send")
        {
            return Err(StoreError::InvalidState(format!(
                "part {run_id} is not a user_send run owned by session {session_id}"
            )));
        }
        let removed = member_ids
            .iter()
            .filter_map(|part_id| parts.get(part_id))
            .filter(|part| part.part_id == run_id || part.run_id == Some(run_id))
            .cloned()
            .collect::<Vec<_>>();
        drop(parts);

        if removed.is_empty() {
            return Ok(Vec::new());
        }
        let removed_ids = removed
            .iter()
            .map(|part| part.part_id)
            .collect::<BTreeSet<_>>();
        self.membership
            .write()
            .expect("membership lock")
            .entry(session_id)
            .or_default()
            .retain(|part_id| !removed_ids.contains(part_id));
        self.idempotency
            .write()
            .expect("idempotency lock")
            .retain(|(sid, _), mapped_run_id| *sid != session_id || *mapped_run_id != run_id);
        self.bump_session_version(session_id, now_ms)?;
        Ok(removed)
    }

    async fn answer_interaction(
        &self,
        session_id: i64,
        owner_id: &str,
        interaction_part_id: i64,
        reply: NewPart,
        now_ms: i64,
    ) -> Result<InteractionAnswerOutcome, StoreError> {
        self.ensure_lease(session_id, owner_id, now_ms)?;
        let mut parts = self.parts.write().expect("parts lock");
        let interaction = parts.get_mut(&interaction_part_id).ok_or_else(|| {
            StoreError::not_found(format!("interaction part {interaction_part_id}"))
        })?;
        if interaction.kind != "interaction" || !interaction.state.is_in_flight() {
            return Err(StoreError::InvalidState(format!(
                "part {interaction_part_id} is not a pending interaction"
            )));
        }
        if interaction.origin_session_id != session_id {
            return Err(StoreError::InvalidState(format!(
                "interaction {interaction_part_id} is shared; only its origin session may answer it"
            )));
        }
        let owning_run = interaction.run_id;
        interaction.state = PartState::Completed;
        interaction.finished_at_ms = Some(now_ms);
        interaction.updated_at_ms = now_ms;
        interaction.revision += 1;
        let interaction_part = interaction.clone();

        let reply_id = self.next_part_id();
        let reply_part = Part {
            part_id: reply_id,
            kind: reply.kind,
            role: reply.role,
            state: reply.state,
            content: reply.content,
            summary: reply.summary,
            visibility: reply.visibility,
            rendered_markdown: reply.rendered_markdown,
            parent_part_id: Some(interaction_part_id),
            run_id: owning_run,
            origin_session_id: session_id,
            revision: 1,
            started_at_ms: now_ms,
            finished_at_ms: reply.state.is_terminal().then_some(now_ms),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            provider_state: None,
        };
        parts.insert(reply_id, reply_part.clone());
        drop(parts);
        self.membership
            .write()
            .expect("membership lock")
            .entry(session_id)
            .or_default()
            .insert(reply_id);
        self.bump_member_session_versions(&[interaction_part_id, reply_id], now_ms)?;
        Ok(InteractionAnswerOutcome {
            interaction: interaction_part,
            reply: reply_part,
        })
    }

    async fn fork_session(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
        rewind: bool,
        _now_ms: i64,
    ) -> Result<SessionMeta, StoreError> {
        let parent = self.session_meta(session_id).await?;
        let (parent_members, parent_parts) = {
            let membership = self.membership.read().expect("membership lock");
            let parts = self.parts.read().expect("parts lock");
            let ids: Vec<i64> = membership
                .get(&session_id)
                .into_iter()
                .flat_map(|set| set.iter().copied())
                .collect();
            let cutoff = parts
                .get(&at_part_id)
                .ok_or_else(|| StoreError::not_found(format!("cutoff part {at_part_id}")))?;
            (ids, (cutoff.created_at_ms, cutoff.part_id, parts.clone()))
        };
        let (cutoff_created, cutoff_id, parts_map) = parent_parts;
        let relation_kind = if rewind {
            SessionRelationKind::Rewind
        } else {
            SessionRelationKind::Fork
        };
        let child = NewSession {
            workspace_id: parent.workspace_id,
            parent_id: Some(session_id),
            relation_kind,
            cutoff_part_id: Some(at_part_id),
            title,
            task_id: None,
            config_json: parent.config_json.clone(),
            provider_anchors_json: None,
        };
        let child_meta = self.create_session(child).await?;
        let child_id = child_meta.id;

        let mut membership = self.membership.write().expect("membership lock");
        let child_set = membership.entry(child_id).or_default();
        for part_id in parent_members {
            let Some(part) = parts_map.get(&part_id) else {
                continue;
            };
            let included = if rewind {
                part.created_at_ms < cutoff_created
                    || (part.created_at_ms == cutoff_created && part.part_id < cutoff_id)
            } else {
                part.created_at_ms < cutoff_created
                    || (part.created_at_ms == cutoff_created && part.part_id <= cutoff_id)
            };
            if included {
                child_set.insert(part_id);
            }
        }
        Ok(child_meta)
    }

    async fn reconcile(
        &self,
        session_id: i64,
        now_ms: i64,
    ) -> Result<ReconcileOutcome, StoreError> {
        let in_flight = self.in_flight_runs(session_id);
        let run_ids: Vec<i64> = in_flight.iter().map(|run| run.part_id).collect();
        self.abort_runs(session_id, &run_ids, "process_restart", now_ms)
    }

    async fn maintenance(&self, now_ms: i64) -> Result<MaintenanceOutcome, StoreError> {
        let reaped = self.reap_stale_leases(now_ms - LEASE_STALENESS_MS).await?;
        let mut outcome = MaintenanceOutcome {
            reaped_sessions: reaped,
            ..Default::default()
        };

        // Refcount-guarded GC (7.6 + invariant 4): a part is deleted only when
        // it has zero membership AND it is not itself an in-flight run marker
        // AND its run reference is absent or terminal. Rows belonging to an
        // active run are never collected.
        let orphan_ids = {
            let membership = self.membership.read().expect("membership lock");
            let parts = self.parts.read().expect("parts lock");
            let mut candidates: Vec<i64> = parts
                .iter()
                .filter(|(id, part)| {
                    let has_membership = membership.values().any(|set| set.contains(id));
                    if has_membership {
                        return false;
                    }
                    if part.is_run_marker() && part.state.is_in_flight() {
                        return false;
                    }
                    match part.run_id {
                        None => true,
                        Some(run_id) => parts
                            .get(&run_id)
                            .is_none_or(|run| !run.state.is_in_flight()),
                    }
                })
                .map(|(id, _)| *id)
                .collect();
            // Children before parents (a child orphan whose parent is also an
            // orphan must be removed first to satisfy FK-like ordering).
            candidates.sort_by_key(|id| {
                let parent = parts.get(id).and_then(|part| part.parent_part_id);
                (parent.is_some(), *id)
            });
            candidates
        };

        let mut parts = self.parts.write().expect("parts lock");
        for id in orphan_ids {
            if parts.remove(&id).is_some() {
                outcome.gc_deleted_parts += 1;
            }
        }
        Ok(outcome)
    }

    async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError> {
        self.usage.write().expect("usage lock").push(record);
        Ok(())
    }

    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError> {
        let usage = self.usage.read().expect("usage lock");
        let mut groups: BTreeMap<(String, String), UsageGroup> = BTreeMap::new();
        for record in usage.iter().filter(|record| query.matches(record)) {
            let group = groups
                .entry((record.provider_id.clone(), record.model_id.clone()))
                .or_insert_with(|| UsageGroup {
                    provider_id: record.provider_id.clone(),
                    model_id: record.model_id.clone(),
                    calls: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    reasoning_tokens: 0,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    total_cost_micros: 0,
                });
            group.calls += 1;
            group.input_tokens += record.input_tokens;
            group.output_tokens += record.output_tokens;
            group.reasoning_tokens += record.reasoning_tokens;
            group.cache_write_tokens += record.cache_write_tokens;
            group.cache_read_tokens += record.cache_read_tokens;
            group.total_cost_micros += record.total_cost_micros;
        }
        let mut stats = UsageStats::default();
        for (_, group) in groups {
            stats.total_calls += group.calls;
            stats.total_input_tokens += group.input_tokens;
            stats.total_output_tokens += group.output_tokens;
            stats.total_cost_micros += group.total_cost_micros;
            stats.groups.push(group);
        }
        Ok(stats)
    }

    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError> {
        let view = self.load_session(session_id).await?;
        jsonl::serialize(&view)
    }

    async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
        now_ms: i64,
    ) -> Result<i64, StoreError> {
        let parsed = jsonl::parse(bundle)?;
        let meta = self
            .create_session(NewSession {
                workspace_id,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: parsed.title.clone(),
                task_id: parsed.task_id.clone(),
                config_json: parsed.config_json.clone(),
                provider_anchors_json: parsed.provider_anchors_json.clone(),
            })
            .await?;
        let session_id = meta.id;

        // Remap part ids so run_id/parent_part_id references stay valid even
        // if the exported ids collide with existing parts.
        let mut id_map: HashMap<i64, i64> = HashMap::new();
        for part in &parsed.parts {
            id_map.insert(part.part_id, self.next_part_id());
        }
        {
            let mut membership = self.membership.write().expect("membership lock");
            let mut parts = self.parts.write().expect("parts lock");
            let set = membership.entry(session_id).or_default();
            for part in &parsed.parts {
                let new_id = id_map[&part.part_id];
                let mut remapped = part.clone();
                remapped.part_id = new_id;
                remapped.run_id = part.run_id.map(|run| id_map[&run]);
                remapped.parent_part_id = part.parent_part_id.map(|parent| id_map[&parent]);
                remapped.origin_session_id = session_id;
                parts.insert(new_id, remapped);
                set.insert(new_id);
            }
        }
        if !parsed.parts.is_empty() {
            self.bump_session_version(session_id, now_ms)?;
        }
        Ok(session_id)
    }
}

impl UsageQuery {
    fn matches(&self, record: &UsageRecord) -> bool {
        if let Some(session_id) = self.session_id
            && record.session_id != session_id
        {
            return false;
        }
        if let Some(workspace_id) = self.workspace_id
            && record.workspace_id != workspace_id
        {
            return false;
        }
        if let Some(provider_id) = self.provider_id.as_deref()
            && record.provider_id != provider_id
        {
            return false;
        }
        if let Some(model_id) = self.model_id.as_deref()
            && record.model_id != model_id
        {
            return false;
        }
        if let Some(after_ms) = self.after_ms
            && record.created_at_ms < after_ms
        {
            return false;
        }
        if let Some(before_ms) = self.before_ms
            && record.created_at_ms >= before_ms
        {
            return false;
        }
        true
    }
}

fn append_text_delta(content: &mut Value, delta: &str) -> Result<(), StoreError> {
    match content {
        Value::String(text) => {
            text.push_str(delta);
            Ok(())
        }
        Value::Object(map) if map.get("text").and_then(Value::as_str).is_some() => {
            if let Some(Value::String(text)) = map.get_mut("text") {
                text.push_str(delta);
            }
            Ok(())
        }
        _ => Err(StoreError::InvalidState(
            "content_text_delta requires a text-shaped content".to_owned(),
        )),
    }
}

/// Derive the presentation from an engine's rows (test helper).
#[cfg(test)]
fn derive_state(
    engine: &InMemoryEngine,
    session_id: i64,
) -> Result<super::SessionPresentation, StoreError> {
    let meta = engine
        .sessions
        .read()
        .expect("sessions lock")
        .get(&session_id)
        .cloned();
    let lease = engine
        .leases
        .read()
        .expect("leases lock")
        .get(&session_id)
        .cloned();
    let now_ms = engine.now_ms();
    let mut parts = engine.ordered_parts(session_id);
    parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
    let in_flight: Vec<InFlightRun> = parts
        .iter()
        .filter(|part| part.is_run_marker() && part.state.is_in_flight())
        .map(|part| InFlightRun {
            part_id: part.part_id,
            created_at_ms: part.created_at_ms,
        })
        .collect();
    let interactions: Vec<PendingInteraction> = parts
        .iter()
        .filter(|part| {
            if !part.state.is_in_flight() {
                return false;
            }
            // Legacy `interaction` parts, plus canonical in-flight tool_call
            // parts whose operation is awaiting a user-input reply.
            part.kind == "interaction"
                || (part.kind == "tool_call"
                    && super::state::tool_call_first_awaiting_user_input(&part.content).is_some())
        })
        .map(|part| PendingInteraction {
            part_id: part.part_id,
            created_at_ms: part.created_at_ms,
            part_kind: part.kind.clone(),
            content: part.content.clone(),
        })
        .collect();
    let last_error = parts
        .iter()
        .filter(|part| part.kind == "error")
        .max_by_key(|part| part.created_at_ms)
        .map(|part| part.content.clone());
    super::presentation(
        meta.as_ref(),
        &in_flight,
        &interactions,
        last_error.as_ref(),
        lease.as_ref(),
        now_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{BackgroundOperationKind, NewPart, SessionState};

    fn text_part(text: &str) -> NewPart {
        NewPart::pending("text", PartRole::User, json!({ "text": text }))
    }

    async fn setup() -> (InMemoryEngine, i64) {
        let engine = InMemoryEngine::new(InMemoryEngineConfig::default());
        engine.set_now(1_000_000);
        let meta = engine
            .create_session(NewSession {
                workspace_id: 1,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "test".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create session");
        let session_id = meta.id;
        let acquire = engine
            .try_acquire_lease(session_id, "owner-a", engine.now_ms())
            .await
            .expect("acquire lease");
        assert!(
            matches!(acquire, LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs.is_empty())
        );
        (engine, session_id)
    }

    #[tokio::test]
    async fn list_exclude_subagents_hides_only_task_children() {
        let (engine, parent_id) = setup().await;
        let parent = engine.session_meta(parent_id).await.expect("parent meta");
        engine
            .create_subagent_session(
                parent_id,
                "task-9".to_owned(),
                "sub task".to_owned(),
                engine.now_ms(),
            )
            .await
            .expect("create subagent");
        engine
            .create_session(NewSession {
                workspace_id: parent.workspace_id,
                parent_id: Some(parent_id),
                relation_kind: SessionRelationKind::Child,
                cutoff_part_id: None,
                title: "user child".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create child");

        let all = engine
            .list_session_summaries(SessionListQuery {
                workspace_id: Some(parent.workspace_id),
                parent_id: None,
                roots_only: false,
                exclude_subagents: false,
                search: None,
                limit: None,
                before: None,
            })
            .await
            .expect("list all");
        assert_eq!(all.len(), 3, "without the filter every session is listed");

        let parents_only = engine
            .list_session_summaries(SessionListQuery {
                workspace_id: Some(parent.workspace_id),
                parent_id: None,
                roots_only: false,
                exclude_subagents: true,
                search: None,
                limit: None,
                before: None,
            })
            .await
            .expect("list excluding subagents");
        assert!(
            parents_only
                .iter()
                .all(|summary| summary.title != "sub task"),
            "task child must be hidden"
        );
        assert_eq!(parents_only.len(), 2, "root + user child remain");
    }

    #[tokio::test]
    async fn user_send_creates_marker_and_content_parts_with_membership() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        assert!(outcome.created);
        assert_eq!(outcome.parts.len(), 2);
        let marker = &outcome.parts[0];
        assert!(marker.is_run_marker());
        assert_eq!(marker.content["run_kind"], "user_send");

        let view = engine.load_session(session_id).await.expect("load");
        assert_eq!(view.parts.len(), 2);
        assert_eq!(view.parts[0].part_id, marker.part_id);
        assert_eq!(view.parts[1].kind, "text");
    }

    #[tokio::test]
    async fn completed_user_send_is_a_terminal_input_receipt_not_a_liveness_guard() {
        let (engine, session_id) = setup().await;
        let mut input = text_part("already committed");
        input.state = PartState::Completed;
        let outcome = engine
            .submit_user_run(session_id, "owner-a", vec![input], None, engine.now_ms())
            .await
            .expect("submit completed input");
        let marker = &outcome.parts[0];
        assert_eq!(marker.state, PartState::Completed);
        assert!(marker.finished_at_ms.is_some());
        assert_eq!(marker.content["abort_reason"], serde_json::Value::Null);

        let view = engine.load_session(session_id).await.expect("load input");
        assert!(
            view.parts
                .iter()
                .filter(|part| part.is_run_marker())
                .all(|part| part.state.is_terminal()),
            "a completed input contributes no in-flight run marker"
        );
    }

    #[tokio::test]
    async fn idempotency_key_deduplicates_user_send() {
        let (engine, session_id) = setup().await;
        let first = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hi")],
                Some("key-1".to_owned()),
                engine.now_ms(),
            )
            .await
            .expect("first submit");
        let second = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hi again")],
                Some("key-1".to_owned()),
                engine.now_ms(),
            )
            .await
            .expect("second submit");
        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.run_id, second.run_id);
    }

    #[tokio::test]
    async fn execution_aware_user_send_is_withdrawable_and_replay_keeps_original_owner() {
        let (engine, session_id) = setup().await;
        let first = engine
            .submit_user_run_for_execution(
                session_id,
                "owner-a",
                vec![text_part("first")],
                Some("key-execution".to_owned()),
                "execution-a",
                engine.now_ms(),
            )
            .await
            .expect("first submit");
        assert!(first.created);
        assert_eq!(
            first.parts[0].content["execution_id"],
            serde_json::json!("execution-a")
        );

        let replay = engine
            .submit_user_run_for_execution(
                session_id,
                "owner-a",
                vec![text_part("replay")],
                Some("key-execution".to_owned()),
                "execution-b",
                engine.now_ms(),
            )
            .await
            .expect("idempotency replay");
        assert!(!replay.created);
        assert_eq!(replay.run_id, first.run_id);
        assert_eq!(
            engine
                .load_session(session_id)
                .await
                .expect("load replay")
                .parts[0]
                .content["execution_id"],
            serde_json::json!("execution-a")
        );

        let removed = engine
            .withdraw_user_run(session_id, "owner-a", first.run_id, engine.now_ms())
            .await
            .expect("withdraw");
        assert_eq!(removed.len(), 2);
        assert!(
            engine
                .load_session(session_id)
                .await
                .expect("load after withdraw")
                .parts
                .is_empty()
        );
        assert!(
            engine
                .withdraw_user_run(session_id, "owner-a", first.run_id, engine.now_ms())
                .await
                .expect("repeat withdraw")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn writes_without_a_fresh_lease_are_refused() {
        let engine = InMemoryEngine::new(InMemoryEngineConfig::default());
        engine.set_now(1_000_000);
        let meta = engine
            .create_session(NewSession {
                workspace_id: 1,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "t".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create");
        let error = engine
            .submit_user_run(
                meta.id,
                "no-lease",
                vec![text_part("x")],
                None,
                engine.now_ms(),
            )
            .await
            .expect_err("no lease");
        assert!(matches!(error, StoreError::LeaseNotHeld { .. }));
    }

    #[tokio::test]
    async fn lease_steal_aborts_stale_run_atomically() {
        let (engine, session_id) = setup().await;
        engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        // Let the lease go stale and another owner steal it.
        engine.set_now(engine.now_ms() + 60_000);
        let stolen = engine
            .try_acquire_lease(session_id, "owner-b", engine.now_ms())
            .await
            .expect("steal");
        match stolen {
            LeaseAcquire::Acquired {
                reconciled_runs, ..
            } => {
                assert_eq!(reconciled_runs.len(), 1);
            }
            LeaseAcquire::HeldBy { .. } => panic!("lease must be stale"),
        }
        let view = engine.load_session(session_id).await.expect("load");
        let marker = view
            .parts
            .iter()
            .find(|part| part.is_run_marker())
            .expect("marker");
        assert_eq!(marker.state, PartState::Failed);
        assert_eq!(marker.content["abort_reason"], "lease_stolen");
        let child = view
            .parts
            .iter()
            .find(|part| part.kind == "text")
            .expect("text part");
        assert_eq!(child.state, PartState::Cancelled);
    }

    #[tokio::test]
    async fn fork_copies_edges_up_to_cutoff_and_child_reads_shared_prefix() {
        let (engine, session_id) = setup().await;
        engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("one")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("first send");
        engine.set_now(engine.now_ms() + 1000);
        engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("two")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("second send");
        let view = engine.load_session(session_id).await.expect("load");
        // Cut off after the second run's marker: only run 1 + its text.
        let cutoff = view
            .parts
            .iter()
            .find(|part| part.kind == "text" && part.content["text"] == "one")
            .expect("first text")
            .part_id;
        let child = engine
            .fork_session(
                session_id,
                cutoff,
                "forked".to_owned(),
                false,
                engine.now_ms(),
            )
            .await
            .expect("fork");
        let child_view = engine.load_session(child.id).await.expect("child view");
        assert_eq!(child_view.parts.len(), 2);
        assert!(
            child_view
                .parts
                .iter()
                .all(|part| part.content["text"] != "two")
        );
        // The text part is SHARED: same part_id in parent and child.
        assert!(engine.session_meta(session_id).await.expect("parent").id > 0);
        assert_eq!(child_view.parts[1].part_id, cutoff);
    }

    #[tokio::test]
    async fn fork_during_streaming_shares_completion_and_rejects_child_mutation() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"text": "partial"}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
                engine.now_ms(),
            )
            .await
            .expect("start stream");
        let streamed_id = outcome.parts[1].part_id;
        let child = engine
            .fork_session(
                session_id,
                streamed_id,
                "stream child".to_owned(),
                false,
                engine.now_ms(),
            )
            .await
            .expect("fork stream");
        engine
            .try_acquire_lease(child.id, "child-owner", engine.now_ms())
            .await
            .expect("child lease");
        engine
            .update_part(
                session_id,
                "owner-a",
                streamed_id,
                PartDelta {
                    state: Some(PartState::Completed),
                    content_text_delta: Some(" complete".to_owned()),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("parent completes shared stream");
        let child_view = engine.load_session(child.id).await.expect("child view");
        let shared = child_view
            .parts
            .iter()
            .find(|part| part.part_id == streamed_id)
            .expect("shared part");
        assert_eq!(shared.state, PartState::Completed);
        assert_eq!(shared.content["text"], "partial complete");

        let error = engine
            .update_part(
                child.id,
                "child-owner",
                streamed_id,
                PartDelta {
                    content: Some(json!({"text": "overwrite"})),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect_err("shared parent part is read-only in child");
        assert!(matches!(error, StoreError::InvalidState(_)));
        let appended = engine
            .append_parts(
                child.id,
                "child-owner",
                outcome.run_id,
                vec![text_part("child divergence")],
                engine.now_ms(),
            )
            .await
            .expect("child diverges by append");
        assert_eq!(appended[0].origin_session_id, child.id);
    }

    #[tokio::test]
    async fn retry_transitions_failed_to_in_progress_with_revision_bump_but_not_for_runs() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "tool_call".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"name": "fs.read", "input": {}}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    // A tool call starts executing immediately (in_progress).
                    state: PartState::InProgress,
                }],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let call_id = outcome.parts[1].part_id;
        engine
            .update_part(
                session_id,
                "owner-a",
                call_id,
                PartDelta {
                    state: Some(PartState::Failed),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("fail the call");
        engine.set_now(engine.now_ms() + 1);
        let retried = engine
            .update_part(
                session_id,
                "owner-a",
                call_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("retry");
        assert_eq!(retried.state, PartState::InProgress);
        assert_eq!(retried.revision, 3);
        assert!(retried.finished_at_ms.is_none());

        // A run marker may not retry failed -> in_progress.
        let marker_id = outcome.run_id;
        engine
            .complete_run(
                session_id,
                "owner-a",
                marker_id,
                RunOutcome {
                    status: PartState::Failed,
                    abort_reason: Some("provider_error".to_owned()),
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("fail run");
        let error = engine
            .update_part(
                session_id,
                "owner-a",
                marker_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect_err("run marker cannot retry");
        assert!(matches!(error, StoreError::InvalidState(_)));
    }

    #[tokio::test]
    async fn retry_history_retains_error_and_updates_the_single_tool_call_result() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "tool_call".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"name": "fs.read", "input": {}}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
                engine.now_ms(),
            )
            .await
            .expect("start tool");
        let tool_id = outcome.parts[1].part_id;
        engine
            .update_part(
                session_id,
                "owner-a",
                tool_id,
                PartDelta {
                    state: Some(PartState::Failed),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("fail tool");
        engine
            .append_parts(
                session_id,
                "owner-a",
                outcome.run_id,
                vec![NewPart {
                    kind: "error".to_owned(),
                    role: PartRole::Runtime,
                    content: json!({"message": "attempt one failed", "attempt": 1}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: Some(tool_id),
                    state: PartState::Failed,
                }],
                engine.now_ms(),
            )
            .await
            .expect("append error");
        engine
            .update_part(
                session_id,
                "owner-a",
                tool_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("retry tool");
        engine
            .update_part(
                session_id,
                "owner-a",
                tool_id,
                PartDelta {
                    state: Some(PartState::Completed),
                    content: Some(json!({
                        "name": "fs.read",
                        "input": {},
                        "state": "completed",
                        "output": {"payload": {"output": "ok", "ok": true}}
                    })),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("complete tool");
        engine
            .complete_run(
                session_id,
                "owner-a",
                outcome.run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("complete run");

        let history = engine.load_session(session_id).await.expect("history");
        let error = history
            .parts
            .iter()
            .find(|part| part.kind == "error")
            .expect("error persists");
        let result = history
            .parts
            .iter()
            .find(|part| part.part_id == tool_id)
            .expect("success persists");
        assert_eq!(error.parent_part_id, Some(tool_id));
        assert_eq!(error.state, PartState::Failed);
        assert_eq!(result.parent_part_id, None);
        assert_eq!(result.state, PartState::Completed);
        assert_eq!(result.content["output"]["payload"]["output"], "ok");
    }

    #[tokio::test]
    async fn complete_run_requires_abort_reason_on_failure_and_keeps_children() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
        let missing = engine
            .complete_run(
                session_id,
                "owner-a",
                run_id,
                RunOutcome {
                    status: PartState::Failed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect_err("missing abort reason");
        assert!(matches!(missing, StoreError::InvalidState(_)));
        let done = engine
            .complete_run(
                session_id,
                "owner-a",
                run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: Some(json!({"run_kind": "user_send"})),
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("complete");
        assert_eq!(done.state, PartState::Completed);
        assert_eq!(done.content["abort_reason"], Value::Null);
        assert!(done.finished_at_ms.is_some());
    }

    #[tokio::test]
    async fn answer_interaction_completes_it_and_appends_a_reply_part() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Runtime,
                    json!({"kind": "ask_user", "prompt": "Continue?"}),
                )],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let interaction_id = outcome.parts[1].part_id;
        let answer = engine
            .answer_interaction(
                session_id,
                "owner-a",
                interaction_id,
                NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
                engine.now_ms(),
            )
            .await
            .expect("answer");
        assert_eq!(answer.reply.parent_part_id, Some(interaction_id));
        assert_eq!(answer.reply.run_id, Some(outcome.run_id));
        assert_eq!(answer.interaction.part_id, interaction_id);
        assert_eq!(answer.interaction.state, PartState::Completed);
        let view = engine.load_session(session_id).await.expect("load");
        let interaction = view
            .parts
            .iter()
            .find(|part| part.part_id == interaction_id)
            .expect("interaction");
        assert_eq!(interaction.state, PartState::Completed);
    }

    #[tokio::test]
    async fn fork_cannot_answer_a_shared_interaction_in_place() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Assistant,
                    json!({"kind": "ask_user", "prompt": "Continue?"}),
                )],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit interaction");
        let interaction_id = outcome.parts[1].part_id;
        let child = engine
            .fork_session(
                session_id,
                interaction_id,
                "fork".to_owned(),
                false,
                engine.now_ms(),
            )
            .await
            .expect("fork");
        engine
            .try_acquire_lease(child.id, "child-owner", engine.now_ms())
            .await
            .expect("child lease");

        let error = engine
            .answer_interaction(
                child.id,
                "child-owner",
                interaction_id,
                NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
                engine.now_ms(),
            )
            .await
            .expect_err("shared interaction is origin-owned");
        assert!(matches!(error, StoreError::InvalidState(_)));
        let parent = engine.load_session(session_id).await.expect("parent");
        assert_eq!(
            parent
                .parts
                .iter()
                .find(|part| part.part_id == interaction_id)
                .expect("interaction")
                .state,
            PartState::Pending
        );
    }

    #[tokio::test]
    async fn state_derivation_covers_all_sessions_states() {
        let (engine, session_id) = setup().await;
        engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hi")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let presentation = derive_state(&engine, session_id).expect("presentation");
        assert_eq!(presentation.state, SessionState::Running);
        // The marker is the first allocated part (id 1); the text part is 2.
        assert_eq!(presentation.active_run_id, Some(1));

        // Stale lease -> Interrupted.
        engine.set_now(engine.now_ms() + 60_000);
        let interrupted = derive_state(&engine, session_id).expect("presentation");
        assert_eq!(interrupted.state, SessionState::Interrupted);

        // Reconcile -> Ready. The stale lease row remains; the process
        // re-acquires a fresh lease before running again (17.4).
        engine
            .reconcile(session_id, engine.now_ms())
            .await
            .expect("reconcile");
        let ready = derive_state(&engine, session_id).expect("presentation");
        assert_eq!(ready.state, SessionState::Ready);
        let acquire = engine
            .try_acquire_lease(session_id, "owner-a", engine.now_ms())
            .await
            .expect("re-acquire lease");
        assert!(
            matches!(acquire, LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs.is_empty())
        );

        // Pending interaction -> AwaitingInteraction, even when the lease is gone.
        let paused = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Runtime,
                    json!({"kind": "plan_review", "prompt": "Approve?"}),
                )],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit with interaction");
        assert!(
            engine
                .release_lease(session_id, "owner-a")
                .await
                .expect("release paused lease")
        );
        let awaiting = derive_state(&engine, session_id).expect("presentation");
        assert_eq!(awaiting.state, SessionState::AwaitingInteraction);
        let interaction = awaiting.pending_interaction.expect("pending interaction");
        assert_eq!(interaction.kind, "plan_review");
        let paused_view = engine.load_session(session_id).await.expect("paused view");
        assert!(
            paused_view
                .parts
                .iter()
                .find(|part| part.part_id == paused.run_id)
                .expect("paused run marker")
                .state
                .is_in_flight(),
            "a pending interaction wins even without a lease"
        );
    }

    #[tokio::test]
    async fn gc_deletes_only_refcount_orphans() {
        let (engine, session_id) = setup().await;
        let outcome = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        // Complete the run so its rows are no longer referenced by an active
        // run; only then may GC collect them once the session is gone.
        engine
            .complete_run(
                session_id,
                "owner-a",
                outcome.run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("complete run");
        // Delete the session: edges cascade, parts become orphans.
        engine.delete_session(session_id).await.expect("delete");
        let outcome = engine
            .maintenance(engine.now_ms())
            .await
            .expect("maintenance");
        assert_eq!(outcome.gc_deleted_parts, 2);
    }

    #[tokio::test]
    async fn session_part_pages_are_newest_first_and_cursor_disjoint() {
        let (engine, session_id) = setup().await;
        for _ in 0..3 {
            engine
                .submit_user_run(
                    session_id,
                    "owner-a",
                    vec![text_part("page")],
                    None,
                    engine.now_ms(),
                )
                .await
                .expect("append page fixture");
        }

        let first = engine
            .load_session_page(session_id, None, 2)
            .await
            .expect("load first page");
        assert_eq!(first.parts.len(), 2);
        assert!(first.has_more);
        assert!(first.parts.windows(2).all(|parts| {
            (parts[0].created_at_ms, parts[0].part_id) > (parts[1].created_at_ms, parts[1].part_id)
        }));

        let before = PartCursor {
            created_at_ms: first.parts.last().expect("first page row").created_at_ms,
            part_id: first.parts.last().expect("first page row").part_id,
        };
        let second = engine
            .load_session_page(session_id, Some(before), 2)
            .await
            .expect("load second page");
        assert_eq!(second.parts.len(), 2);
        assert!(second.has_more);
        assert!(second.parts.iter().all(
            |part| (part.created_at_ms, part.part_id) < (before.created_at_ms, before.part_id)
        ));
        assert!(first.parts.iter().all(|first| {
            second
                .parts
                .iter()
                .all(|second| first.part_id != second.part_id)
        }));

        let before = PartCursor {
            created_at_ms: second.parts.last().expect("second page row").created_at_ms,
            part_id: second.parts.last().expect("second page row").part_id,
        };
        let third = engine
            .load_session_page(session_id, Some(before), 2)
            .await
            .expect("load final page");
        assert_eq!(third.parts.len(), 2);
        assert!(!third.has_more);
    }

    #[tokio::test]
    async fn jsonl_round_trip_preserves_ordering_and_references() {
        let (engine, session_id) = setup().await;
        engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let bundle = engine
            .export_session_jsonl(session_id)
            .await
            .expect("export");
        assert_eq!(bundle.lines().count(), 3); // meta + marker + text

        let imported_id = engine
            .import_session_jsonl(1, &bundle, engine.now_ms())
            .await
            .expect("import");
        let imported = engine.load_session(imported_id).await.expect("load");
        assert_eq!(imported.parts.len(), 2);
        let marker = &imported.parts[0];
        assert!(marker.is_run_marker());
        assert_eq!(imported.parts[1].run_id, Some(marker.part_id));
        assert_eq!(imported.parts[1].content["text"], "hello");
    }

    #[tokio::test]
    async fn usage_stats_groups_by_provider_and_model() {
        let engine = InMemoryEngine::new(InMemoryEngineConfig::default());
        engine.set_now(1_000_000);
        for i in 0..3 {
            engine
                .record_usage(UsageRecord {
                    workspace_id: 1,
                    session_id: 1,
                    run_id: Some(1),
                    provider_id: "anthropic".to_owned(),
                    model_id: "claude-5".to_owned(),
                    created_at_ms: 1_000_000 + i,
                    input_tokens: 100,
                    output_tokens: 50,
                    reasoning_tokens: 10,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    tool_use_tokens: 0,
                    other_tokens: 0,
                    total_cost_micros: 1500,
                    recorded_cost_micros: None,
                    cost_estimate_incomplete: false,
                    detail_json: None,
                })
                .await
                .expect("record");
        }
        let stats = engine
            .usage_stats(UsageQuery {
                workspace_id: Some(1),
                ..Default::default()
            })
            .await
            .expect("stats");
        assert_eq!(stats.total_calls, 3);
        assert_eq!(stats.total_input_tokens, 300);
        assert_eq!(stats.total_cost_micros, 4500);
        assert_eq!(stats.groups.len(), 1);
        assert_eq!(stats.groups[0].model_id, "claude-5");
    }

    #[tokio::test]
    async fn every_part_mutation_bumps_version_but_idempotency_replay_does_not() {
        let (engine, session_id) = setup().await;
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 1);

        let submitted = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("hello")],
                Some("send-1".to_owned()),
                engine.now_ms(),
            )
            .await
            .expect("submit");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 2);
        let replay = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("ignored")],
                Some("send-1".to_owned()),
                engine.now_ms(),
            )
            .await
            .expect("replay");
        assert!(!replay.created);
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 2);

        let appended = engine
            .append_parts(
                session_id,
                "owner-a",
                submitted.run_id,
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Assistant,
                    json!({"kind": "ask_user", "prompt": "Continue?"}),
                )],
                engine.now_ms(),
            )
            .await
            .expect("append interaction");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 3);

        engine
            .update_part(
                session_id,
                "owner-a",
                submitted.parts[1].part_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    ..Default::default()
                },
                engine.now_ms(),
            )
            .await
            .expect("update part");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 4);

        engine
            .answer_interaction(
                session_id,
                "owner-a",
                appended[0].part_id,
                NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
                engine.now_ms(),
            )
            .await
            .expect("answer");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 5);

        engine
            .complete_run(
                session_id,
                "owner-a",
                submitted.run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("complete");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 6);

        let cancelled = engine
            .start_run(
                session_id,
                "owner-a",
                "continue",
                json!({}),
                None,
                engine.now_ms(),
            )
            .await
            .expect("start cancellation run");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 7);
        engine
            .cancel_run(session_id, "owner-a", cancelled.run_id, engine.now_ms())
            .await
            .expect("cancel");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 8);

        engine
            .start_run(
                session_id,
                "owner-a",
                "continue",
                json!({}),
                None,
                engine.now_ms(),
            )
            .await
            .expect("start interrupted run");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 9);
        engine
            .release_lease(session_id, "owner-a")
            .await
            .expect("release lease");
        engine
            .reconcile(session_id, engine.now_ms())
            .await
            .expect("reconcile");
        assert_eq!(engine.session_meta(session_id).await.unwrap().version, 10);

        let bundle = engine
            .export_session_jsonl(session_id)
            .await
            .expect("export");
        let imported_id = engine
            .import_session_jsonl(1, &bundle, engine.now_ms())
            .await
            .expect("import");
        assert_eq!(
            engine.session_meta(imported_id).await.unwrap().version,
            2,
            "imported membership advances the fresh session position"
        );
    }

    #[tokio::test]
    async fn updating_a_shared_part_bumps_every_member_session_version() {
        let (engine, session_id) = setup().await;
        let submitted = engine
            .submit_user_run(
                session_id,
                "owner-a",
                vec![text_part("shared")],
                None,
                engine.now_ms(),
            )
            .await
            .expect("submit");
        let child = engine
            .fork_session(
                session_id,
                submitted.run_id,
                "fork".to_owned(),
                false,
                engine.now_ms(),
            )
            .await
            .expect("fork");
        let parent_before = engine.session_meta(session_id).await.unwrap().version;
        let child_before = engine.session_meta(child.id).await.unwrap().version;

        engine
            .complete_run(
                session_id,
                "owner-a",
                submitted.run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
                engine.now_ms(),
            )
            .await
            .expect("complete shared marker");

        assert_eq!(
            engine.session_meta(session_id).await.unwrap().version,
            parent_before + 1
        );
        assert_eq!(
            engine.session_meta(child.id).await.unwrap().version,
            child_before + 1
        );
    }

    #[tokio::test]
    async fn background_aggregate_matches_sqlite_event_and_delivery_semantics() {
        let (engine, session_id) = setup().await;
        let submitted = engine
            .start_run(
                session_id,
                "owner-a",
                "continue",
                json!({"run_kind": "continue", "abort_reason": null}),
                None,
                engine.now_ms(),
            )
            .await
            .expect("start assistant launch run");
        let submitted_parts = engine
            .append_parts(
                session_id,
                "owner-a",
                submitted.run_id,
                vec![NewPart::pending(
                    "tool_call",
                    PartRole::Assistant,
                    json!({"operation": {"title": "background receipt"}}),
                )],
                engine.now_ms(),
            )
            .await
            .expect("submit launch receipt");
        let tool_part_id = submitted_parts[0].part_id;
        let operation_id = format!("bg_{session_id}_{tool_part_id}");
        let created = engine
            .create_background_operation(
                NewBackgroundOperation {
                    operation_id: operation_id.clone(),
                    session_id,
                    launch_run_id: Some(submitted.run_id),
                    launch_tool_part_id: Some(tool_part_id),
                    kind: BackgroundOperationKind::Monitor,
                },
                1_000_001,
            )
            .await
            .expect("create operation");
        let launching = engine
            .transition_background_operation(
                BackgroundOperationTransition {
                    operation_id: operation_id.clone(),
                    expected_revision: created.revision,
                    next_phase: BackgroundOperationPhase::Launching,
                    external_id: Some("proc_memory".to_owned()),
                    outcome: None,
                    failure: None,
                    owner_id: Some("test".to_owned()),
                    lease_until_ms: Some(1_030_000),
                },
                1_000_002,
            )
            .await
            .expect("launching");
        engine
            .transition_background_operation(
                BackgroundOperationTransition {
                    operation_id: operation_id.clone(),
                    expected_revision: launching.revision,
                    next_phase: BackgroundOperationPhase::Running,
                    external_id: Some("proc_memory".to_owned()),
                    outcome: None,
                    failure: None,
                    owner_id: None,
                    lease_until_ms: None,
                },
                1_000_003,
            )
            .await
            .expect("running");
        let request = |seq| {
            let mut notification = NewPart::pending(
                "system_notification",
                PartRole::Assistant,
                json!({"operation_id":"proc_memory","operation_kind":"monitor","status":"event","event_seq":seq}),
            );
            notification.state = PartState::Completed;
            BackgroundEventRequest {
                operation_id: operation_id.clone(),
                event_key: format!("event:{seq}"),
                event_seq: Some(seq),
                next_phase: None,
                outcome: None,
                failure: None,
                notification,
            }
        };
        assert!(
            engine
                .record_background_event(request(2), 1_000_004)
                .await
                .expect("seq 2")
                .created
        );
        assert!(
            engine
                .record_background_event(request(1), 1_000_005)
                .await
                .expect("out-of-order seq 1")
                .created
        );
        assert!(
            !engine
                .record_background_event(request(1), 1_000_006)
                .await
                .expect("duplicate seq 1")
                .created
        );
        let operation = engine
            .background_operation(&operation_id)
            .await
            .expect("load")
            .expect("operation");
        assert_eq!(operation.last_event_seq, 2);
        assert_eq!(operation.phase, BackgroundOperationPhase::Running);
        let pending = engine
            .pending_background_deliveries(10, 1_000_010)
            .await
            .expect("pending deliveries");
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|delivery| {
            delivery.phase == BackgroundDeliveryPhase::Pending
                && delivery.notification_part_id.is_some()
        }));
        let view = engine
            .load_session(session_id)
            .await
            .expect("load transcript");
        assert!(
            view.parts
                .iter()
                .filter(|part| part.kind == "system_notification")
                .all(|part| {
                    part.role == PartRole::Assistant && part.run_id == Some(submitted.run_id)
                }),
            "AI-launched events append to their assistant launch run"
        );
    }
}
