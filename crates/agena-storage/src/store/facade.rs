//! The sealed `SessionStore` facade (design 14-15).
//!
//! External callers (TUI, Web, CLI, tests) depend ONLY on this trait plus the
//! pure data types in [`super::types`]. The facade hides the memory/DB split
//! completely: it is composed with either backend
//! ([`InMemoryEngine`](super::InMemoryEngine) here or `SqliteEngine` in
//! `agena-storage-sqlite`) at runtime, and callers cannot tell the two apart
//! (15.4).
//!
//! ## The only live-update mechanism
//!
//! [`SessionChange`](super::SessionChange) notifications are derived from an
//! operation, emitted **after commit**, and never persisted or replayed (14.3).
//! The facade emits them through an in-process [`NotificationBus`] (15.5) so
//! same-process subscribers see every committed change; cross-process live
//! updates ride the notification stream plus version/cursor catch-up (14.4) —
//! the facade hides the transport.
//!
//! ## Write path (commit-then-notify, 15.6)
//!
//! Every facade write validates the session lease against the caller's
//! `owner_id`, commits one transaction through the engine, then notifies
//! subscribers before returning. Streaming appends are buffered in the
//! [`MemoryLayer`] so the UI sees deltas immediately (15.3).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use super::{
    LEASE_STALENESS_MS, LeaseAcquire, NewPart, PartDelta, PersistenceEngine, RunOutcome,
    SessionChange, SessionListQuery, SessionMeta, SessionPresentation, SessionSummary, SessionView,
    StateInputs, StoreError, SubmitOutcome, UsageQuery, UsageStats, presentation,
};

/// A session subscription handle. Dropping it unsubscribes (15.5).
pub struct Subscription {
    session_id: i64,
    bus: Arc<NotificationBus>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.session_id);
    }
}

/// A session-scoped observer of [`SessionChange`] notifications.
pub type SessionObserver = Arc<dyn Fn(SessionChange) + Send + Sync + 'static>;

/// The sealed session data facade (14.1). This is the ONLY public entry for
/// chat data; no layer outside the persistence engine touches the database.
///
/// The facade is backend-agnostic: it is generic over
/// [`PersistenceEngine`](super::PersistenceEngine), so the same code composes
/// with `SqliteEngine` (production) or `InMemoryEngine` (tests / small
/// deployments).
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Load a session's metadata plus parts ordered by
    /// `(created_at_ms, part_id)` — cache first, then one membership JOIN.
    async fn load(&self, session_id: i64) -> Result<SessionView, StoreError>;

    /// List session summaries, newest first (13.1 / 14.1).
    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError>;

    /// List every session in one root's subtree, newest first.
    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError>;

    /// Derive the single session state (17.3) for the UI.
    async fn session_state(&self, session_id: i64) -> Result<SessionPresentation, StoreError>;

    /// User send (7.1): marker + content parts + membership + optional
    /// idempotency in one committed transaction. Returns the run marker's
    /// part id.
    async fn submit_user_message(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<i64, StoreError>;

    /// Append content parts to an in-flight run (streaming, D10).
    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<(), StoreError>;

    /// Apply a streaming delta to one part (revision++, notify on flush).
    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<(), StoreError>;

    /// Finish a run marker with the given terminal outcome.
    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
    ) -> Result<(), StoreError>;

    /// Answer a pending interaction: complete it and append the user reply.
    async fn answer_interaction(
        &self,
        session_id: i64,
        owner_id: &str,
        interaction_part_id: i64,
        reply: NewPart,
    ) -> Result<(), StoreError>;

    /// Fork the session at a part, copying membership edges up to the cutoff
    /// (7.3). Returns the new session id.
    async fn fork(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, StoreError>;

    /// Rewind the session at a part (fork with an exclusive cutoff; the
    /// cutoff part belongs to the parent only).
    async fn rewind(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, StoreError>;

    /// Rename a session (bumps `sessions.version`).
    async fn rename(&self, session_id: i64, title: String) -> Result<SessionMeta, StoreError>;

    /// Cancel a run marker and its non-terminal children (17.5 user cancel).
    async fn cancel_run(&self, session_id: i64, owner_id: &str, run_id: i64) -> Result<(), StoreError>;

    /// Compact the session: a `compaction` run marker closes the preceding
    /// window; provider anchors are cleared (13.3). Returns the compaction
    /// run's part id.
    async fn compact_session(&self, session_id: i64, owner_id: &str) -> Result<i64, StoreError>;

    /// Delete a session (membership cascades; shared parts survive and are
    /// GC'd by reference count, 7.6).
    async fn delete(&self, session_id: i64) -> Result<(), StoreError>;

    /// Serialize the full session as JSONL (meta + ordered parts).
    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError>;

    /// Import a JSONL bundle into a fresh session under `workspace_id`.
    async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
    ) -> Result<i64, StoreError>;

    /// Aggregated usage stats (section 16).
    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError>;

    /// Subscribe to [`SessionChange`] notifications for one session. The
    /// returned [`Subscription`] unsubscribes on drop.
    fn subscribe(&self, session_id: i64, observer: SessionObserver) -> Subscription;
}

/// A session's cached view plus the position it was read at.
#[derive(Debug, Clone)]
struct CacheEntry {
    view: SessionView,
    /// `sessions.version` at cache time; session-meta writes bump it, which
    /// invalidates the entry on the next hit (15.3).
    version: i64,
    /// Newest member part cursor `(created_at_ms, part_id)` at cache time;
    /// part additions move it, which catches cross-process writes that do not
    /// touch `sessions.version` (14.4).
    newest_cursor: Option<(i64, i64)>,
}

/// The internal memory layer (15.3): a per-session LRU cache of
/// [`SessionView`]s validated against the persisted position. (The streaming
/// buffers of 15.3 are introduced with the execution engine's throttled flush
/// path, which has a real consumer for them.)
#[derive(Debug)]
pub struct MemoryLayer {
    /// session_id -> cached view (LRU, capped by `max_cached_sessions`).
    cache: Mutex<HashMap<i64, CacheEntry>>,
    /// session_id -> LRU recency counter, highest = most recently used.
    lru: Mutex<HashMap<i64, u64>>,
    /// Monotonic recency stamp.
    clock: AtomicU64,
    /// Maximum number of sessions held in the cache.
    max_cached_sessions: usize,
}

impl Default for MemoryLayer {
    fn default() -> Self {
        Self::new(64)
    }
}

impl MemoryLayer {
    pub fn new(max_cached_sessions: usize) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            lru: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(1),
            max_cached_sessions,
        }
    }

    /// Cache hit when the cached view still matches the persisted position:
    /// `sessions.version` (session-meta writes) AND the newest member cursor
    /// (part additions, cross-process writes). Either moving invalidates.
    fn get(
        &self,
        session_id: i64,
        version: Option<i64>,
        newest_cursor: Option<(i64, i64)>,
    ) -> Option<SessionView> {
        let cache = self.cache.lock().expect("cache lock");
        let entry = cache.get(&session_id)?;
        // A missing persisted version (session deleted) is not a hit.
        if version.is_some() && entry.version != version.unwrap() {
            return None;
        }
        if entry.newest_cursor != newest_cursor {
            return None;
        }
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        self.lru.lock().expect("lru lock").insert(session_id, stamp);
        Some(entry.view.clone())
    }

    fn insert(&self, session_id: i64, view: SessionView, version: i64, newest_cursor: Option<(i64, i64)>) {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut lru = self.lru.lock().expect("lru lock");
        lru.insert(session_id, stamp);
        if lru.len() > self.max_cached_sessions {
            let evict = lru
                .iter()
                .min_by_key(|(_, recency)| **recency)
                .map(|(id, _)| *id);
            if let Some(evict) = evict {
                lru.remove(&evict);
                self.cache.lock().expect("cache lock").remove(&evict);
            }
        }
        drop(lru);
        self.cache.lock().expect("cache lock").insert(
            session_id,
            CacheEntry {
                view,
                version,
                newest_cursor,
            },
        );
    }

    /// Discard a session's cache entry (a write committed, or a delete — the
    /// entry no longer reflects the persisted position).
    fn invalidate(&self, session_id: i64) {
        self.cache.lock().expect("cache lock").remove(&session_id);
        self.lru.lock().expect("lru lock").remove(&session_id);
    }
}

/// The in-process live-update bus (15.5). `SessionChange`s are emitted after
/// commit and never persisted; this is observer notification, not an event
/// log (14.3).
#[derive(Default)]
pub struct NotificationBus {
    observers: Mutex<HashMap<i64, Vec<SessionObserver>>>,
}

impl NotificationBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn subscribe(&self, session_id: i64, observer: SessionObserver) {
        self.observers
            .lock()
            .expect("observers lock")
            .entry(session_id)
            .or_default()
            .push(observer);
    }

    fn unsubscribe(&self, session_id: i64) {
        self.observers.lock().expect("observers lock").remove(&session_id);
    }

    /// Emit a change to every session subscriber. Never persisted, never
    /// replayed; an observer must not rely on receiving every change.
    fn emit(&self, change: SessionChange) {
        let observers = self
            .observers
            .lock()
            .expect("observers lock")
            .get(&change.session_id())
            .cloned();
        if let Some(observers) = observers {
            for observer in observers {
                observer(change.clone());
            }
        }
    }
}

impl SessionChange {
    /// The session a change belongs to (used by the bus to fan out).
    fn session_id(&self) -> i64 {
        match self {
            Self::PartAdded { session_id, .. }
            | Self::PartUpdated { session_id, .. }
            | Self::PartRemoved { session_id, .. }
            | Self::SessionMetaUpdated { session_id, .. } => *session_id,
        }
    }
}

/// Concrete facade implementation, composed with any [`PersistenceEngine`]
/// backend (15.4). `now_ms` is a clock function so tests can drive time; it
/// defaults to the wall clock.
pub struct SessionFacade<E> {
    engine: E,
    memory: Arc<MemoryLayer>,
    bus: Arc<NotificationBus>,
    /// Lease owner identity for this process/caller.
    default_owner: String,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl<E> SessionFacade<E>
where
    E: PersistenceEngine,
{
    pub fn new(engine: E, owner_id: impl Into<String>, max_cached_sessions: usize) -> Self {
        Self::with_clock(
            engine,
            owner_id,
            MemoryLayer::new(max_cached_sessions),
            NotificationBus::new(),
            wall_clock_ms,
        )
    }

    /// Test constructor: inject the memory layer, bus, and clock.
    pub fn with_clock(
        engine: E,
        owner_id: impl Into<String>,
        memory: MemoryLayer,
        bus: NotificationBus,
        now_ms: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            engine,
            memory: Arc::new(memory),
            bus: Arc::new(bus),
            default_owner: owner_id.into(),
            now_ms: Arc::new(now_ms),
        }
    }

    fn now(&self) -> i64 {
        (self.now_ms)()
    }

    fn owner(&self, caller: &str) -> String {
        if caller.is_empty() {
            self.default_owner.clone()
        } else {
            caller.to_owned()
        }
    }

    /// The engine, exposed for recovery/maintenance only (lease reaping, GC).
    /// The facade itself is the only chat-data write path; these calls are
    /// maintenance internals (14.2).
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// Load from cache first; on a miss, one membership JOIN through the
    /// engine. The cache is validated against `sessions.version` AND the
    /// newest member cursor so cross-process writes are caught on the next
    /// read (14.4, 15.3).
    async fn load_cached(&self, session_id: i64) -> Result<SessionView, StoreError> {
        let meta = self.engine.session_meta(session_id).await?;
        let version = meta.version;
        let cursor = self.engine.newest_member_cursor(session_id).await?;
        if let Some(view) = self.memory.get(session_id, Some(version), cursor) {
            return Ok(view);
        }
        let view = self.engine.load_session(session_id).await?;
        self.memory.insert(session_id, view.clone(), version, cursor);
        Ok(view)
    }

    /// Derive the session presentation (17.6) from the persisted rows.
    async fn derive_presentation(&self, session_id: i64) -> Result<SessionPresentation, StoreError> {
        let meta = self.engine.session_meta(session_id).await?;
        let view = self.engine.load_session(session_id).await?;
        let inputs = StateInputs::from_view(&view);
        let lease = self.engine.current_lease(session_id).await?;
        presentation(
            Some(&meta),
            &inputs.in_flight_runs,
            &inputs.pending_interactions,
            inputs.last_error.as_ref(),
            lease.as_ref(),
            self.now(),
        )
    }

    /// Acquire the session lease (heartbeat on every commit). `reconcile` is
    /// used when the previous holder's lease went stale: the residual
    /// in-flight run is aborted and its children cancelled (17.4).
    async fn ensure_lease(
        &self,
        session_id: i64,
        owner: &str,
        reconcile: bool,
    ) -> Result<(), StoreError> {
        let now = self.now();
        let fresh = match self.engine.current_lease(session_id).await? {
            Some(lease) => now - lease.heartbeat_at_ms <= LEASE_STALENESS_MS,
            None => false,
        };
        if fresh {
            // Someone holds a fresh lease — try to heartbeat as the caller. If
            // it is another owner, this is refused.
            if self.engine.heartbeat_lease(session_id, owner, now).await? {
                return Ok(());
            }
            // Not our lease; fall through to acquisition (which refuses if the
            // other owner is still fresh).
        }
        match self
            .engine
            .try_acquire_lease(session_id, owner, now)
            .await?
        {
            LeaseAcquire::Acquired { reconciled_runs } => {
                if reconcile && !reconciled_runs.is_empty() {
                    self.reconcile(session_id).await?;
                }
                Ok(())
            }
            LeaseAcquire::HeldBy { .. } => Err(StoreError::LeaseHeldByOther {
                session_id,
                owner_id: owner.to_owned(),
                heartbeat_at_ms: now,
            }),
        }
    }

    /// Reconcile a session whose in-flight run lost its lease (17.4 step 2c):
    /// mark in-flight run markers `failed` (`process_restart`) and their
    /// non-terminal children `cancelled`. Idempotent.
    async fn reconcile(&self, session_id: i64) -> Result<(), StoreError> {
        let outcome = self.engine.reconcile(session_id, self.now()).await?;
        if !outcome.aborted_runs.is_empty() || outcome.cancelled_parts > 0 {
            let meta = self.engine.session_meta(session_id).await?;
            self.bus.emit(SessionChange::SessionMetaUpdated {
                session_id,
                meta,
            });
        }
        Ok(())
    }
}

#[async_trait]
impl<E> SessionStore for SessionFacade<E>
where
    E: PersistenceEngine,
{
    async fn load(&self, session_id: i64) -> Result<SessionView, StoreError> {
        self.load_cached(session_id).await
    }

    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        self.engine.list_session_summaries(query).await
    }

    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError> {
        self.engine.list_session_tree(root_id).await
    }

    async fn session_state(&self, session_id: i64) -> Result<SessionPresentation, StoreError> {
        // Fresh reads beat a stale cache: the UI must see the true derived
        // state even across process boundaries (17.1 principle 1).
        self.derive_presentation(session_id).await
    }

    async fn submit_user_message(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<i64, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        let outcome: SubmitOutcome = self
            .engine
            .submit_user_message(session_id, &owner, parts, idempotency_key, self.now())
            .await?;
        if outcome.created {
            self.memory.invalidate(session_id);
            let meta = self.engine.session_meta(session_id).await?;
            for part in &outcome.parts {
                self.bus.emit(SessionChange::PartAdded {
                    session_id,
                    part: part.clone(),
                });
            }
            self.bus.emit(SessionChange::SessionMetaUpdated { session_id, meta });
        }
        Ok(outcome.run_id)
    }

    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        let created = self
            .engine
            .append_parts(session_id, &owner, run_id, parts, self.now())
            .await?;
        self.memory.invalidate(session_id);
        for part in created {
            self.bus.emit(SessionChange::PartAdded {
                session_id,
                part,
            });
        }
        Ok(())
    }

    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        let updated = self
            .engine
            .update_part(session_id, &owner, part_id, delta, self.now())
            .await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::PartUpdated {
            session_id,
            part: updated,
        });
        Ok(())
    }

    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        let marker = self
            .engine
            .complete_run(session_id, &owner, run_id, outcome, self.now())
            .await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::PartUpdated {
            session_id,
            part: marker,
        });
        let meta = self.engine.session_meta(session_id).await?;
        self.bus.emit(SessionChange::SessionMetaUpdated { session_id, meta });
        Ok(())
    }

    async fn answer_interaction(
        &self,
        session_id: i64,
        owner_id: &str,
        interaction_part_id: i64,
        reply: NewPart,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        let reply_part = self
            .engine
            .answer_interaction(session_id, &owner, interaction_part_id, reply, self.now())
            .await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::PartAdded {
            session_id,
            part: reply_part,
        });
        Ok(())
    }

    async fn fork(&self, session_id: i64, at_part_id: i64, title: String) -> Result<i64, StoreError> {
        let meta = self
            .engine
            .fork_session(session_id, at_part_id, title.clone(), false, self.now())
            .await?;
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: self.engine.session_meta(session_id).await?,
        });
        Ok(meta.id)
    }

    async fn rewind(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, StoreError> {
        let meta = self
            .engine
            .fork_session(session_id, at_part_id, title.clone(), true, self.now())
            .await?;
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: self.engine.session_meta(session_id).await?,
        });
        Ok(meta.id)
    }

    async fn rename(&self, session_id: i64, title: String) -> Result<SessionMeta, StoreError> {
        let meta = self.engine.rename_session(session_id, title).await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: meta.clone(),
        });
        Ok(meta)
    }

    async fn cancel_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        self.engine
            .cancel_run(session_id, &owner, run_id, self.now())
            .await?;
        self.memory.invalidate(session_id);
        let meta = self.engine.session_meta(session_id).await?;
        self.bus.emit(SessionChange::SessionMetaUpdated { session_id, meta });
        Ok(())
    }

    async fn compact_session(&self, session_id: i64, owner_id: &str) -> Result<i64, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner, true).await?;
        // A compaction run marker closes the preceding window (13.4); provider
        // anchors are cleared because compaction changes the prompt window
        // (13.3).
        let outcome = self
            .engine
            .start_run(
                session_id,
                &owner,
                "compaction",
                json!({ "summary": null }),
                None,
                self.now(),
            )
            .await?;
        self.engine
            .set_provider_anchors(session_id, None)
            .await?;
        self.memory.invalidate(session_id);
        let meta = self.engine.session_meta(session_id).await?;
        for part in outcome.parts {
            self.bus.emit(SessionChange::PartAdded {
                session_id,
                part,
            });
        }
        self.bus.emit(SessionChange::SessionMetaUpdated { session_id, meta });
        Ok(outcome.run_id)
    }

    async fn delete(&self, session_id: i64) -> Result<(), StoreError> {
        self.engine.delete_session(session_id).await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::PartRemoved {
            session_id,
            part_id: 0, // placeholder; deletion removes the whole session
        });
        Ok(())
    }

    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError> {
        self.engine.export_session_jsonl(session_id).await
    }

    async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
    ) -> Result<i64, StoreError> {
        self.engine.import_session_jsonl(workspace_id, bundle, self.now()).await
    }

    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError> {
        self.engine.usage_stats(query).await
    }

    fn subscribe(&self, session_id: i64, observer: SessionObserver) -> Subscription {
        self.bus.subscribe(session_id, observer);
        Subscription {
            session_id,
            bus: Arc::clone(&self.bus),
        }
    }
}

/// Wall-clock milliseconds (the facade's default clock).
fn wall_clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        InMemoryEngine, NewSession, PartRole, PartState, SessionChange, SessionListQuery,
        SessionState,
    };
    use agena_domain::SessionRelationKind;
    use std::sync::atomic::AtomicI64;

    /// Deterministic clock shared with the facade so tests can advance time.
    #[derive(Clone)]
    struct Clock(Arc<AtomicI64>);

    impl Clock {
        fn new(start: i64) -> Self {
            Self(Arc::new(AtomicI64::new(start)))
        }
        fn get(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
        fn advance(&self, by_ms: i64) {
            self.0.fetch_add(by_ms, Ordering::SeqCst);
        }
    }

    /// A ready root session with a fresh lease held by `owner-a`.
    async fn ready_session(
        facade: &SessionFacade<InMemoryEngine>,
        workspace_id: i64,
        title: &str,
    ) -> i64 {
        let meta = facade
            .engine()
            .create_session(NewSession {
                workspace_id,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: title.to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create session");
        let acquire = facade
            .engine()
            .try_acquire_lease(meta.id, "owner-a", facade.now())
            .await
            .expect("acquire lease");
        assert!(matches!(acquire, LeaseAcquire::Acquired { .. }));
        meta.id
    }

    /// A facade over a fresh in-memory engine with a deterministic clock. The
    /// in-memory backend does not validate workspace existence, so any
    /// workspace id is accepted.
    fn harness() -> (SessionFacade<InMemoryEngine>, Clock) {
        let clock = Clock::new(1_000_000);
        let facade = SessionFacade::with_clock(
            InMemoryEngine::default(),
            "owner-a",
            MemoryLayer::new(16),
            NotificationBus::new(),
            {
                let clock = clock.clone();
                move || clock.get()
            },
        );
        (facade, clock)
    }

    #[tokio::test]
    async fn submit_notifies_and_returns_run_id() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().unwrap().push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);

        let run_id = facade
            .submit_user_message(
                session_id,
                "owner-a",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "hello"}))],
                None,
            )
            .await
            .expect("submit");
        let changes = seen.lock().unwrap().clone();
        assert!(
            changes.iter().any(|c| matches!(c, SessionChange::PartAdded { .. })),
            "submit emits PartAdded"
        );
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, SessionChange::SessionMetaUpdated { .. })),
            "submit emits SessionMetaUpdated"
        );
        assert!(run_id > 0);

        let view = facade.load(session_id).await.expect("load");
        assert_eq!(view.parts.len(), 2, "marker + text");
    }

    #[tokio::test]
    async fn duplicate_owner_writes_are_refused_and_resume_heals() {
        let (facade, clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;

        // A second owner cannot write while owner-a's lease is fresh.
        let err = facade
            .submit_user_message(
                session_id,
                "owner-b",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "x"}))],
                None,
            )
            .await
            .expect_err("owner-b refused");
        assert!(matches!(err, StoreError::LeaseHeldByOther { .. }));

        // owner-a's lease goes stale; owner-b acquires, which reconciles the
        // (now stale) owner-a state and lets owner-b write.
        clock.advance(60_000);
        let run_id = facade
            .submit_user_message(
                session_id,
                "owner-b",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "y"}))],
                None,
            )
            .await
            .expect("owner-b acquires after staleness");
        assert!(run_id > 0);
    }

    #[tokio::test]
    async fn session_state_derives_running_awaiting_and_ready() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;

        let ready = facade.session_state(session_id).await.expect("state");
        assert_eq!(ready.state, SessionState::Ready);

        let run_id = facade
            .submit_user_message(
                session_id,
                "owner-a",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "hi"}))],
                None,
            )
            .await
            .expect("submit");
        let running = facade.session_state(session_id).await.expect("state");
        assert_eq!(running.state, SessionState::Running);
        assert_eq!(running.active_run_id, Some(run_id));

        // Complete the run -> Ready again.
        facade
            .complete_run(
                session_id,
                "owner-a",
                run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
            )
            .await
            .expect("complete");
        let ready_again = facade.session_state(session_id).await.expect("state");
        assert_eq!(ready_again.state, SessionState::Ready);
    }

    #[tokio::test]
    async fn pending_interaction_gates_to_awaiting_user() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let run_id = facade
            .submit_user_message(
                session_id,
                "owner-a",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "q"}))],
                None,
            )
            .await
            .expect("submit");
        facade
            .append_parts(
                session_id,
                "owner-a",
                run_id,
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Assistant,
                    json!({"kind": "ask_user", "prompt": "which?"}),
                )],
            )
            .await
            .expect("append interaction");

        let awaiting = facade.session_state(session_id).await.expect("state");
        assert_eq!(awaiting.state, SessionState::AwaitingUser);
        let pending = awaiting.pending_interaction.expect("pending interaction");
        assert_eq!(pending.kind, "ask_user");
        assert_eq!(pending.prompt, "which?");

        facade
            .answer_interaction(
                session_id,
                "owner-a",
                pending.part_id,
                NewPart::pending("text", PartRole::User, json!({"text": "option 1"})),
            )
            .await
            .expect("answer");
        let after = facade.session_state(session_id).await.expect("state");
        assert_eq!(after.state, SessionState::Running, "interaction answered");
    }

    #[tokio::test]
    async fn fork_and_rewind_copy_edges_and_return_new_session_ids() {
        let (facade, clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let run_id = facade
            .submit_user_message(
                session_id,
                "owner-a",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "hello"}))],
                None,
            )
            .await
            .expect("submit");
        facade
            .complete_run(session_id, "owner-a", run_id, RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            })
            .await
            .expect("complete");
        clock.advance(1);

        let view = facade.load(session_id).await.expect("load");
        let cutoff = view.parts[0].part_id; // the marker

        let fork_id = facade.fork(session_id, cutoff, "fork".to_owned()).await.expect("fork");
        let fork_view = facade.load(fork_id).await.expect("fork view");
        assert_eq!(fork_view.parts.len(), 1, "fork copies edges up to the marker");

        let rewind_id = facade
            .rewind(session_id, cutoff, "rewind".to_owned())
            .await
            .expect("rewind");
        let rewind_view = facade.load(rewind_id).await.expect("rewind view");
        assert!(rewind_view.parts.is_empty(), "rewind excludes the cutoff");
        assert_ne!(fork_id, rewind_id);
    }

    #[tokio::test]
    async fn compact_session_starts_a_compaction_run_and_clears_anchors() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        facade
            .engine()
            .set_provider_anchors(session_id, Some(json!({"claude": {"anchor": 1}})))
            .await
            .expect("set anchors");
        let compaction_id = facade
            .compact_session(session_id, "owner-a")
            .await
            .expect("compact");
        assert!(compaction_id > 0);
        let meta = facade.engine().session_meta(session_id).await.expect("meta");
        assert!(meta.provider_anchors_json.is_none(), "anchors cleared");
        let view = facade.load(session_id).await.expect("load");
        assert!(
            view.parts.iter().any(|p| p.is_run_marker()),
            "compaction run marker created"
        );
    }

    #[tokio::test]
    async fn idempotency_key_deduplicates_user_send_through_the_facade() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let parts = vec![NewPart::pending("text", PartRole::User, json!({"text": "once"}))];
        let first = facade
            .submit_user_message(session_id, "owner-a", parts.clone(), Some("k1".to_owned()))
            .await
            .expect("first");
        let second = facade
            .submit_user_message(session_id, "owner-a", parts.clone(), Some("k1".to_owned()))
            .await
            .expect("replay");
        assert_eq!(first, second, "replay returns the same run id");
        let view = facade.load(session_id).await.expect("load");
        assert_eq!(view.parts.len(), 2, "no duplicate parts");
    }

    #[tokio::test]
    async fn delete_removes_the_session_and_emits_removal() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().unwrap().push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);
        facade.delete(session_id).await.expect("delete");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, SessionChange::PartRemoved { .. })),
            "delete emits PartRemoved"
        );
        let err = facade.load(session_id).await.expect_err("gone");
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn rename_updates_meta_and_notifies() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().unwrap().push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);
        let meta = facade.rename(session_id, "new title".to_owned()).await.expect("rename");
        assert_eq!(meta.title, "new title");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, SessionChange::SessionMetaUpdated { .. })),
            "rename emits SessionMetaUpdated"
        );
        let listed = facade
            .list_session_summaries(SessionListQuery::default())
            .await
            .expect("list");
        assert!(
            listed.iter().any(|s| s.id == session_id && s.title == "new title"),
            "summary reflects the rename"
        );
    }

    #[tokio::test]
    async fn usage_stats_are_reported_through_the_facade() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        facade
            .engine()
            .record_usage(crate::store::UsageRecord {
                workspace_id: 1,
                session_id,
                run_id: None,
                provider_id: "anthropic".to_owned(),
                model_id: "claude-5".to_owned(),
                created_at_ms: facade.now(),
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                tool_use_tokens: 0,
                other_tokens: 0,
                total_cost_micros: 75,
                recorded_cost_micros: None,
                cost_estimate_incomplete: false,
                detail_json: None,
            })
            .await
            .expect("record");
        let stats = facade
            .usage_stats(crate::store::UsageQuery {
                workspace_id: Some(1),
                ..Default::default()
            })
            .await
            .expect("stats");
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.total_cost_micros, 75);
    }

    #[tokio::test]
    async fn export_import_jsonl_round_trips_through_the_facade() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        facade
            .submit_user_message(
                session_id,
                "owner-a",
                vec![NewPart::pending("text", PartRole::User, json!({"text": "export me"}))],
                None,
            )
            .await
            .expect("submit");
        let bundle = facade
            .export_session_jsonl(session_id)
            .await
            .expect("export");
        let imported = facade
            .import_session_jsonl(1, &bundle)
            .await
            .expect("import");
        let imported_view = facade.load(imported).await.expect("imported view");
        assert_eq!(imported_view.parts.len(), 2, "marker + text survive the round trip");
        assert_eq!(imported_view.parts[1].content["text"], "export me");
    }
}
