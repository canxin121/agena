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
//! same-process subscribers see every committed change. Cross-process
//! reconnect is an explicit snapshot read validated by session version and
//! member cursor; this store does not claim database-backed push delivery.
//!
//! ## Write path (commit-then-notify, 15.6)
//!
//! Every ordinary facade write validates the session lease against the
//! caller's `owner_id`, commits one transaction through the engine, then
//! notifies subscribers before returning. Text-stream deltas are accumulated
//! in the [`MemoryLayer`] and flushed after a bounded number of deltas or when
//! the run ends (D10); ordinary semantic checkpoints remain commit-synchronous.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    LEASE_STALENESS_MS, LeaseAcquire, MaintenanceOutcome, NewPart, NewSession, Part, PartDelta,
    PartState, PersistenceEngine, RunOutcome, SessionChange, SessionListQuery, SessionMeta,
    SessionPresentation, SessionState, SessionSummary, SessionView, StateInputs, StoreError,
    SubmitOutcome, UsageQuery, UsageRecord, UsageStats, apply_part_transition, presentation,
};

/// Safety ceiling for streaming deltas buffered in memory before one durable
/// part update. Streaming is end-only by default: a part's deltas accumulate
/// in the in-memory buffer and are committed once when the part reaches a
/// terminal/state change (or when its run ends), so a run's mid-flight
/// streaming never writes the database repeatedly. This threshold only
/// bounds an unusually long single part so an unbounded buffer cannot grow
/// without ever touching the durable store (crash-safety backstop).
pub const STREAMING_FLUSH_DELTA_COUNT: usize = 512;

/// A session subscription handle. Dropping it unsubscribes (15.5).
pub struct Subscription {
    session_id: i64,
    observer_id: u64,
    bus: Arc<NotificationBus>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.session_id, self.observer_id);
    }
}

/// A global subscription handle. Dropping it removes this observer from the
/// all-session feed without affecting other observers (15.5).
pub struct GlobalSubscription {
    observer_id: u64,
    bus: Arc<NotificationBus>,
}

impl Drop for GlobalSubscription {
    fn drop(&mut self) {
        self.bus.unsubscribe_all(self.observer_id);
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

    /// Create a new session row (root, child, fork/rewind, or subagent) and
    /// return its metadata. The engine validates the lineage rules
    /// (root/child/subagent must have a `cutoff_part_id`, branches must have
    /// one).
    async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError>;

    /// Find a subagent session by its unique `(parent_id, task_id)` pair.
    async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionMeta>, StoreError>;

    /// Create a subagent session (delegated subtask) under `parent_session_id`
    /// with `task_id` recorded. The schema's unique `(parent_id, task_id)`
    /// index makes concurrent creates fail loudly; callers check
    /// `find_subagent_by_task_id` first. Returns the new session id.
    async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
    ) -> Result<i64, StoreError>;

    /// Update a session's subtask columns (status / started / finished /
    /// failure), bumping `version`.
    async fn update_subtask_state(
        &self,
        session_id: i64,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// List session summaries, newest first (13.1 / 14.1).
    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError>;

    /// Fetch one session's summary row, or `None` when it does not exist.
    /// A cheap single-row projection (13.1) for existence/lifecycle/version
    /// checks on the application path.
    async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, StoreError>;

    /// Session counts per workspace (13.5 `workspace_counts`), for the
    /// workspaces listing surface.
    async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, StoreError>;

    /// List every session in one root's subtree, newest first.
    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError>;

    /// Derive the single session state (17.3) for the UI.
    async fn session_state(&self, session_id: i64) -> Result<SessionPresentation, StoreError>;

    /// User send (7.1): marker + content parts + membership + optional
    /// idempotency in one committed transaction. Returns the full
    /// [`SubmitOutcome`] — the run marker id plus the committed parts — so
    /// callers never reload after writing.
    async fn submit_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, StoreError>;

    /// Append content parts to an in-flight run (streaming, D10). Returns the
    /// committed parts with their engine-assigned ids.
    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<Vec<Part>, StoreError>;

    /// Apply a streaming delta to one part (revision++, notify on flush).
    /// Returns the updated part (the in-memory buffer overlay when the delta
    /// is still buffered, the durable row once flushed).
    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<Part, StoreError>;

    /// Finish a run marker with the given terminal outcome. Returns the
    /// terminal marker row.
    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
    ) -> Result<Part, StoreError>;

    /// Extend the session lease without mutating any part. Long stable runs
    /// (a slow reasoning stream, a multi-second tool execution) can exceed
    /// `LEASE_STALENESS_MS` between commits; a background heartbeat keeps the
    /// run's ownership fresh so the next write is not treated as a stale
    /// steal. Returns `false` when no lease row for this owner exists (already
    /// stolen, released, or never held) — the caller lets the next commit
    /// surface that authoritatively rather than raising mid-stream.
    async fn heartbeat_lease(&self, session_id: i64, owner_id: &str) -> Result<bool, StoreError>;

    /// Start a fresh run without user input (`continue`, `compaction`,
    /// `background`, `steer`). Creates a run marker with the given `run_kind`
    /// and returns the full [`SubmitOutcome`] (marker plus created parts).
    async fn start_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_kind: &str,
        content: Value,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, StoreError>;

    /// Persist `provider_anchors_json` (resume is blocking on it, D8).
    async fn set_provider_anchors(
        &self,
        session_id: i64,
        anchors: Option<Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// Persist `config_json` (execution config only, D5).
    async fn set_config_json(
        &self,
        session_id: i64,
        config: Option<Value>,
    ) -> Result<SessionMeta, StoreError>;

    /// Append one provider-call usage record (append-only, section 16). No
    /// lease: usage is written once per model call by the engine and never
    /// updated.
    async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError>;

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
    /// Returns every changed row (marker and cancelled children).
    async fn cancel_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
    ) -> Result<Vec<Part>, StoreError>;

    /// Reconcile a session whose in-flight run lost its lease (17.4 step 2c):
    /// mark in-flight run markers `failed` (`process_restart`) and their
    /// non-terminal children `cancelled`. Idempotent.
    async fn reconcile(&self, session_id: i64) -> Result<(), StoreError>;

    /// Compact the session: a `compaction` run marker closes the preceding
    /// window and records the durable checkpoint (13.3 / 13.4); provider
    /// anchors are cleared. The marker's content is the 4.1.1
    /// `CompactionContent` shape — `{"summary": ..., "window": ...}` — where
    /// `summary` is the continuation record and `window` describes the span of
    /// conversation the compaction covered (absent or null when the caller has
    /// nothing to report). Returns the compaction run's part id.
    async fn compact_session(
        &self,
        session_id: i64,
        owner_id: &str,
        summary: Option<String>,
        window: Option<String>,
    ) -> Result<i64, StoreError>;

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

    /// Maintenance internals (14.2): reap stale leases and GC orphan parts.
    /// Exposed through the sealed facade so recovery/maintenance callers never
    /// reach the engine directly. Idempotent; safe to run from any process.
    async fn maintenance(&self, now_ms: i64) -> Result<MaintenanceOutcome, StoreError>;

    /// Subscribe to process-local [`SessionChange`] notifications for one
    /// session. The returned [`Subscription`] unsubscribes only itself on
    /// drop; reconnect/cross-process catch-up uses [`Self::load`].
    fn subscribe(&self, session_id: i64, observer: SessionObserver) -> Subscription;

    /// Subscribe to [`SessionChange`] notifications for every session. The
    /// returned [`GlobalSubscription`] unsubscribes on drop (15.5). Used by
    /// same-process presentation consumers (SSE, TUI) that fan out by session
    /// themselves; cross-process catch-up is by `sessions.version` /
    /// `(created_at_ms, part_id)` (14.4).
    fn subscribe_all(&self, observer: SessionObserver) -> GlobalSubscription;
}

/// A session's cached view plus the position it was read at.
#[derive(Debug, Clone)]
struct CacheEntry {
    view: SessionView,
    /// `sessions.version` at cache time; every session-visible mutation bumps
    /// it, which invalidates the entry on the next hit (8.6 / 15.3).
    version: i64,
    /// Newest member part cursor `(created_at_ms, part_id)` at cache time; an
    /// additional membership-position check alongside the required version.
    newest_cursor: Option<(i64, i64)>,
}

/// One in-memory text stream checkpoint waiting for its bounded durable
/// flush. `part.revision` remains the last committed revision until the
/// engine accepts the coalesced update.
#[derive(Debug, Clone)]
struct StreamingBuffer {
    owner_id: String,
    part: Part,
    pending_deltas: usize,
    /// Last time the session lease was heartbeated for this buffer. The
    /// buffered path commits no rows per delta, so a long reasoning stream
    /// would otherwise let the lease age past `LEASE_STALENESS_MS` and be
    /// stolen by the next commit, aborting the in-flight run mid-stream.
    last_heartbeat_at_ms: i64,
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
    /// `(session_id, part_id)` text streams awaiting a bounded flush.
    streaming: Mutex<HashMap<(i64, i64), StreamingBuffer>>,
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
            streaming: Mutex::new(HashMap::new()),
        }
    }

    /// Cache hit when the cached view still matches the persisted position:
    /// `sessions.version` (all session-visible writes) AND the newest member
    /// cursor (membership position). Either moving invalidates.
    fn get(
        &self,
        session_id: i64,
        version: Option<i64>,
        newest_cursor: Option<(i64, i64)>,
    ) -> Option<SessionView> {
        // Read the entry and touch recency under separate locks — never nest
        // the cache and LRU mutexes (see `insert`'s eviction path and
        // [`Self::apply_committed`]).
        let view = {
            let cache = self.cache.lock().expect("cache lock");
            let entry = cache.get(&session_id)?;
            // A missing persisted version (session deleted) is not a hit.
            if version.is_some() && entry.version != version.unwrap() {
                return None;
            }
            if entry.newest_cursor != newest_cursor {
                return None;
            }
            entry.view.clone()
        };
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        self.lru.lock().expect("lru lock").insert(session_id, stamp);
        Some(view)
    }

    fn insert(
        &self,
        session_id: i64,
        view: SessionView,
        version: i64,
        newest_cursor: Option<(i64, i64)>,
    ) {
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

    /// Merge parts that were just committed (or are in an in-progress
    /// streaming buffer) into the cached view, keeping the entry's position
    /// in sync so the very next `load` is a cache hit instead of a redundant
    /// membership JOIN. In-place updates replace by `part_id`; new parts
    /// append — the `(created_at_ms, part_id)` order position is immutable,
    /// so appends keep the view ordered.
    ///
    /// `version` is `Some` when the write advanced `sessions.version` (a
    /// durable mutation) and `None` for a not-yet-flushed streaming delta,
    /// which must leave the entry's version untouched. A missing cache entry
    /// is a no-op (there is nothing to seed).
    fn apply_committed(&self, session_id: i64, parts: &[Part], version: Option<i64>) {
        // Touch recency under the LRU lock first, then update the cache under
        // the cache lock — the same lock order as `insert` (which may take the
        // cache lock while holding the LRU lock during eviction). Never nest
        // the two in the opposite order.
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        self.lru.lock().expect("lru lock").insert(session_id, stamp);
        let mut cache = self.cache.lock().expect("cache lock");
        let Some(entry) = cache.get_mut(&session_id) else {
            return;
        };
        for part in parts {
            match entry
                .view
                .parts
                .iter_mut()
                .find(|existing| existing.part_id == part.part_id)
            {
                Some(existing) => *existing = part.clone(),
                None => entry.view.parts.push(part.clone()),
            }
        }
        if let Some(version) = version {
            entry.version = version;
        }
        let parts_cursor = parts
            .iter()
            .map(|part| (part.created_at_ms, part.part_id))
            .max();
        entry.newest_cursor = match (entry.newest_cursor, parts_cursor) {
            (Some(existing), Some(incoming)) => Some(existing.max(incoming)),
            (existing, incoming) => existing.or(incoming),
        };
    }

    /// Overlay same-process, not-yet-flushed text deltas on a persisted/cache
    /// view. Other processes intentionally see only bounded checkpoints.
    fn overlay_streaming(&self, session_id: i64, view: &mut SessionView) {
        let streaming = self.streaming.lock().expect("streaming lock");
        for part in &mut view.parts {
            if let Some(buffer) = streaming.get(&(session_id, part.part_id)) {
                *part = buffer.part.clone();
            }
        }
    }

    fn clear_streaming_session(&self, session_id: i64) {
        self.streaming
            .lock()
            .expect("streaming lock")
            .retain(|(buffer_session_id, _), _| *buffer_session_id != session_id);
    }
}

/// The in-process live-update bus (15.5). `SessionChange`s are emitted after
/// commit and never persisted; this is observer notification, not an event
/// log (14.3).
#[derive(Default)]
pub struct NotificationBus {
    next_observer_id: AtomicU64,
    observers: Mutex<HashMap<i64, HashMap<u64, SessionObserver>>>,
    global_observers: Mutex<HashMap<u64, SessionObserver>>,
}

impl NotificationBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn subscribe(&self, session_id: i64, observer: SessionObserver) -> u64 {
        let observer_id = self.next_observer_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.observers
            .lock()
            .expect("observers lock")
            .entry(session_id)
            .or_default()
            .insert(observer_id, observer);
        observer_id
    }

    fn unsubscribe(&self, session_id: i64, observer_id: u64) {
        let mut observers = self.observers.lock().expect("observers lock");
        if let Some(session_observers) = observers.get_mut(&session_id) {
            session_observers.remove(&observer_id);
            if session_observers.is_empty() {
                observers.remove(&session_id);
            }
        }
    }

    fn subscribe_all(&self, observer: SessionObserver) -> u64 {
        let observer_id = self.next_observer_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.global_observers
            .lock()
            .expect("global observers lock")
            .insert(observer_id, observer);
        observer_id
    }

    fn unsubscribe_all(&self, observer_id: u64) {
        self.global_observers
            .lock()
            .expect("global observers lock")
            .remove(&observer_id);
    }

    /// Emit a change to every session subscriber plus the global observers.
    /// Never persisted, never replayed; an observer must not rely on
    /// receiving every change.
    fn emit(&self, change: SessionChange) {
        let observers = self
            .observers
            .lock()
            .expect("observers lock")
            .get(&change.session_id())
            .map(|observers| observers.values().cloned().collect::<Vec<_>>());
        if let Some(observers) = observers {
            for observer in observers {
                observer(change.clone());
            }
        }
        let global_observers = self
            .global_observers
            .lock()
            .expect("global observers lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for observer in global_observers {
            observer(change.clone());
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
    streaming_flush_delta_count: usize,
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
            streaming_flush_delta_count: STREAMING_FLUSH_DELTA_COUNT,
        }
    }

    /// Override the D10 text-stream flush threshold. Primarily useful for
    /// deterministic tests and benchmarks; zero is normalized to one.
    pub fn with_streaming_flush_delta_count(mut self, delta_count: usize) -> Self {
        self.streaming_flush_delta_count = delta_count.max(1);
        self
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

    /// Test-only access for white-box parity assertions. Production callers
    /// cannot reach the engine through the facade; maintenance is part of the
    /// sealed [`SessionStore`] surface.
    #[cfg(test)]
    pub(crate) fn engine(&self) -> &E {
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
        if let Some(mut view) = self.memory.get(session_id, Some(version), cursor) {
            self.memory.overlay_streaming(session_id, &mut view);
            return Ok(view);
        }
        let mut view = self.engine.load_session(session_id).await?;
        self.memory
            .insert(session_id, view.clone(), version, cursor);
        self.memory.overlay_streaming(session_id, &mut view);
        Ok(view)
    }

    /// Derive the session presentation (17.6) from the persisted rows.
    async fn derive_presentation(
        &self,
        session_id: i64,
    ) -> Result<SessionPresentation, StoreError> {
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

    /// Acquire the session lease (heartbeat on every commit). A stale-lease
    /// acquisition atomically aborts the previous holder's residual run and
    /// returns the committed rows for immediate live notification.
    async fn ensure_lease(&self, session_id: i64, owner: &str) -> Result<(), StoreError> {
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
            LeaseAcquire::Acquired { updated_parts, .. } => {
                if !updated_parts.is_empty() {
                    self.memory.clear_streaming_session(session_id);
                    self.memory.invalidate(session_id);
                    for part in updated_parts {
                        self.bus
                            .emit(SessionChange::PartUpdated { session_id, part });
                    }
                    let meta = self.engine.session_meta(session_id).await?;
                    self.bus
                        .emit(SessionChange::SessionMetaUpdated { session_id, meta });
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

    /// Read-only lease validation for deltas already represented by an active
    /// in-memory stream buffer. The first delta in each buffer heartbeats via
    /// `ensure_lease`; later deltas avoid turning the lease heartbeat itself
    /// into one database write per chunk. The engine validates ownership again
    /// atomically when the buffer flushes.
    async fn validate_buffered_lease(
        &self,
        session_id: i64,
        owner_id: &str,
    ) -> Result<(), StoreError> {
        let lease = self
            .engine
            .current_lease(session_id)
            .await?
            .ok_or(StoreError::LeaseNotHeld { session_id })?;
        if self.now() - lease.heartbeat_at_ms > LEASE_STALENESS_MS {
            return Err(StoreError::LeaseNotHeld { session_id });
        }
        if lease.owner_id != owner_id {
            return Err(StoreError::LeaseHeldByOther {
                session_id,
                owner_id: lease.owner_id,
                heartbeat_at_ms: lease.heartbeat_at_ms,
            });
        }
        Ok(())
    }

    /// Buffer a text-stream delta and return the committed part when the
    /// threshold (or a non-streaming semantic change) forces a flush.
    async fn update_streaming_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<Option<Part>, StoreError> {
        let key = (session_id, part_id);
        let needs_base = !self
            .memory
            .streaming
            .lock()
            .expect("streaming lock")
            .contains_key(&key);
        // Seed the stream buffer from the facade's cached view rather than a
        // direct engine reload: the cache is kept current by `apply_committed`
        // on every commit, and a warm cache keeps streaming setup off the
        // database read path entirely.
        let persisted_base = if needs_base {
            Some(
                self.load_cached(session_id)
                    .await?
                    .parts
                    .into_iter()
                    .find(|part| part.part_id == part_id)
                    .ok_or_else(|| StoreError::not_found(format!("part {part_id}")))?,
            )
        } else {
            None
        };
        if persisted_base
            .as_ref()
            .is_some_and(|part| part.origin_session_id != session_id)
        {
            return Err(StoreError::InvalidState(format!(
                "part {part_id} is shared; only its origin session may update it in place"
            )));
        }

        let (flush, needs_heartbeat) = {
            let mut streaming = self.memory.streaming.lock().expect("streaming lock");
            let buffer = streaming.entry(key).or_insert_with(|| StreamingBuffer {
                owner_id: owner_id.to_owned(),
                part: persisted_base.expect("missing stream buffer has a persisted base"),
                pending_deltas: 0,
                last_heartbeat_at_ms: self.now(),
            });
            if buffer.owner_id != owner_id {
                return Err(StoreError::InvalidState(format!(
                    "part {part_id} has a streaming buffer owned by another lease holder"
                )));
            }
            if buffer.part.origin_session_id != session_id {
                return Err(StoreError::InvalidState(format!(
                    "part {part_id} is shared; only its origin session may update it in place"
                )));
            }
            // The buffered path commits no row per delta, so a long reasoning
            // stream would let the lease age past LEASE_STALENESS_MS and get
            // stolen (aborting the in-flight run) on the next commit. Heartbeat
            // at half the staleness window: never a database write per chunk,
            // only every ~7.5s of uninterrupted streaming.
            let now = self.now();
            let needs_heartbeat = now - buffer.last_heartbeat_at_ms > LEASE_STALENESS_MS / 2;
            if needs_heartbeat {
                buffer.last_heartbeat_at_ms = now;
            }
            let mut next_part = buffer.part.clone();
            let state_changed = apply_buffered_delta(&mut next_part, delta, now)?;
            buffer.part = next_part;
            buffer.pending_deltas += 1;
            // End-only streaming: commit once when the part transitions state
            // (terminalize, tool-call completion) or when a pathological
            // single part exceeds the safety ceiling. The former `!is_text_delta`
            // clause flushed every non-text delta (think parts rewrite their
            // whole content document per token), which wrote the database
            // ~100+ times per second during reasoning; the durable row is only
            // meaningful at part completion and reads of the in-memory buffer
            // already overlay live content.
            let should_flush =
                state_changed || buffer.pending_deltas >= self.streaming_flush_delta_count;
            (
                should_flush.then(|| {
                    streaming
                        .remove(&key)
                        .expect("stream buffer exists while flushing")
                }),
                needs_heartbeat,
            )
        };

        if needs_heartbeat {
            // Extend the lease so the stream survives a model turn longer than
            // the staleness window. `heartbeat_lease` is a single UPDATE keyed
            // on our owner id: if the lease was already stolen or released it
            // reports `false`, which the next flush/commit surfaces
            // authoritatively — no error to raise mid-stream.
            let heartbeat = self.heartbeat_lease(session_id, owner_id).await?;
            if heartbeat {
                tracing::debug!(%session_id, %part_id, "streaming lease heartbeat extended");
            } else {
                tracing::warn!(%session_id, %part_id, "streaming heartbeat had no lease row");
            }
        }

        match flush {
            Some(buffer) => self
                .flush_streaming_buffer(session_id, buffer)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn flush_streaming_buffer(
        &self,
        session_id: i64,
        buffer: StreamingBuffer,
    ) -> Result<Part, StoreError> {
        let part = buffer.part;
        self.engine
            .update_part(
                session_id,
                &buffer.owner_id,
                part.part_id,
                PartDelta {
                    state: Some(part.state),
                    content: Some(part.content),
                    content_text_delta: None,
                    summary: part.summary,
                    rendered_markdown: part.rendered_markdown,
                    provider_state: part.provider_state,
                    finished_at_ms: part.finished_at_ms,
                },
                self.now(),
            )
            .await
    }

    /// Flush every buffered member of a run before its marker becomes
    /// terminal. This is the mandatory tail flush in D10.
    async fn flush_streaming_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
    ) -> Result<(), StoreError> {
        let buffers = {
            let mut streaming = self.memory.streaming.lock().expect("streaming lock");
            let keys = streaming
                .iter()
                .filter(|((buffer_session_id, part_id), buffer)| {
                    *buffer_session_id == session_id
                        && (*part_id == run_id || buffer.part.run_id == Some(run_id))
                })
                .map(|(key, _)| *key)
                .collect::<Vec<_>>();
            if keys.iter().any(|key| {
                streaming
                    .get(key)
                    .is_some_and(|buffer| buffer.owner_id != owner_id)
            }) {
                return Err(StoreError::InvalidState(format!(
                    "run {run_id} has a streaming buffer owned by another lease holder"
                )));
            }
            let mut buffers = Vec::with_capacity(keys.len());
            for key in keys {
                let buffer = streaming
                    .remove(&key)
                    .expect("collected stream buffer exists");
                buffers.push(buffer);
            }
            buffers
        };
        let mut flushed = Vec::with_capacity(buffers.len());
        for buffer in buffers {
            flushed.push(self.flush_streaming_buffer(session_id, buffer).await?);
        }
        if !flushed.is_empty() {
            // Merge the committed rows into the cache instead of invalidating
            // it: flushing on part completion is the normal tail of a run, and
            // invalidating here would force the next read back to the engine
            // every time a run ends. `apply_committed` keeps the warm cache
            // authoritative (and lets `load_cached` stay a hit).
            let meta = self.engine.session_meta(session_id).await?;
            self.memory
                .apply_committed(session_id, &flushed, Some(meta.version));
            for part in flushed {
                self.bus
                    .emit(SessionChange::PartUpdated { session_id, part });
            }
        }
        Ok(())
    }

    /// Reconcile a session whose in-flight run lost its lease (17.4 step 2c):
    /// mark in-flight run markers `failed` (`process_restart`) and their
    /// non-terminal children `cancelled`. Idempotent.
    async fn reconcile(&self, session_id: i64) -> Result<(), StoreError> {
        let presentation = self.derive_presentation(session_id).await?;
        if presentation.state != SessionState::Interrupted {
            // Running and AwaitingUser must be preserved; Ready/Failed have no
            // crashed in-flight marker to reconcile. Recovery is idempotent.
            return Ok(());
        }
        // A process-restart reconciliation cannot safely commit a process-local
        // stream tail. Drop it before deriving/announcing the terminal rows.
        self.memory.clear_streaming_session(session_id);
        let outcome = self.engine.reconcile(session_id, self.now()).await?;
        if !outcome.updated_parts.is_empty() {
            self.memory.invalidate(session_id);
            for part in outcome.updated_parts {
                self.bus
                    .emit(SessionChange::PartUpdated { session_id, part });
            }
            let meta = self.engine.session_meta(session_id).await?;
            self.bus
                .emit(SessionChange::SessionMetaUpdated { session_id, meta });
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

    async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError> {
        let meta = self.engine.create_session(new_session).await?;
        // A fresh session has no subscribers yet, but emit for consistency
        // with the write path (a child create may be observed by a parent
        // subscriber).
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id: meta.id,
            meta: meta.clone(),
        });
        Ok(meta)
    }

    async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionMeta>, StoreError> {
        self.engine
            .find_subagent_by_task_id(parent_session_id, task_id)
            .await
    }

    async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
    ) -> Result<i64, StoreError> {
        let meta = self
            .engine
            .create_subagent_session(parent_session_id, task_id, title, self.now())
            .await?;
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id: meta.id,
            meta: meta.clone(),
        });
        Ok(meta.id)
    }

    async fn update_subtask_state(
        &self,
        session_id: i64,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let meta = self
            .engine
            .update_subtask_state(session_id, status, started_at_ms, finished_at_ms, failure)
            .await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: meta.clone(),
        });
        Ok(meta)
    }

    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        self.engine.list_session_summaries(query).await
    }

    async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, StoreError> {
        self.engine.get_session_summary(session_id).await
    }

    async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, StoreError> {
        self.engine.session_counts_by_workspace(workspace_ids).await
    }

    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError> {
        self.engine.list_session_tree(root_id).await
    }

    async fn session_state(&self, session_id: i64) -> Result<SessionPresentation, StoreError> {
        // Fresh reads beat a stale cache: the UI must see the true derived
        // state even across process boundaries (17.1 principle 1).
        self.derive_presentation(session_id).await
    }

    async fn submit_user_run(
        &self,
        session_id: i64,
        owner_id: &str,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        let outcome: SubmitOutcome = self
            .engine
            .submit_user_run(session_id, &owner, parts, idempotency_key, self.now())
            .await?;
        if outcome.created {
            let meta = self.engine.session_meta(session_id).await?;
            self.memory
                .apply_committed(session_id, &outcome.parts, Some(meta.version));
            for part in &outcome.parts {
                self.bus.emit(SessionChange::PartAdded {
                    session_id,
                    part: part.clone(),
                });
            }
            self.bus
                .emit(SessionChange::SessionMetaUpdated { session_id, meta });
        }
        Ok(outcome)
    }

    async fn append_parts(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        parts: Vec<NewPart>,
    ) -> Result<Vec<Part>, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        let created = self
            .engine
            .append_parts(session_id, &owner, run_id, parts, self.now())
            .await?;
        let meta = self.engine.session_meta(session_id).await?;
        self.memory
            .apply_committed(session_id, &created, Some(meta.version));
        for part in &created {
            self.bus.emit(SessionChange::PartAdded {
                session_id,
                part: part.clone(),
            });
        }
        Ok(created)
    }

    async fn update_part(
        &self,
        session_id: i64,
        owner_id: &str,
        part_id: i64,
        delta: PartDelta,
    ) -> Result<Part, StoreError> {
        let owner = self.owner(owner_id);
        let has_buffer = self
            .memory
            .streaming
            .lock()
            .expect("streaming lock")
            .contains_key(&(session_id, part_id));
        if has_buffer {
            self.validate_buffered_lease(session_id, &owner).await?;
        } else {
            self.ensure_lease(session_id, &owner).await?;
        }
        let is_text_delta = delta.content.is_none() && delta.content_text_delta.is_some();
        // Route an in-progress content update (think/tool-call whole-document
        // deltas are `content`-shaped, not `content_text_delta`-shaped)
        // through the streaming buffer too. Without this, reasoning deltas
        // bypass the buffer entirely and write the database once per token
        // (~100+ writes per second while thinking). The buffer keeps them in
        // memory and commits once when the part terminalizes or its run ends.
        // A content update with a terminal state (a checkpoint editing a
        // completed part) is a durable edit and stays on the direct commit
        // path so its revision advances.
        let streaming_content_update =
            delta.content.is_some() && delta.state == Some(PartState::InProgress);
        if has_buffer || is_text_delta || streaming_content_update {
            if let Some(updated) = self
                .update_streaming_part(session_id, &owner, part_id, delta)
                .await?
            {
                // The delta was flushed: merge the committed row into the
                // cache (the buffer no longer overlays it) and notify (D10 —
                // notifications follow committed flushes).
                let meta = self.engine.session_meta(session_id).await?;
                self.memory.apply_committed(
                    session_id,
                    std::slice::from_ref(&updated),
                    Some(meta.version),
                );
                self.bus.emit(SessionChange::PartUpdated {
                    session_id,
                    part: updated.clone(),
                });
                return Ok(updated);
            }
            // Still buffered: return the in-memory overlay (authoritative for
            // this process) without a notification and without touching the
            // persisted cache.
            let buffered = self
                .memory
                .streaming
                .lock()
                .expect("streaming lock")
                .get(&(session_id, part_id))
                .expect("buffered part exists")
                .part
                .clone();
            return Ok(buffered);
        }
        let updated = self
            .engine
            .update_part(session_id, &owner, part_id, delta, self.now())
            .await?;
        let meta = self.engine.session_meta(session_id).await?;
        self.memory.apply_committed(
            session_id,
            std::slice::from_ref(&updated),
            Some(meta.version),
        );
        self.bus.emit(SessionChange::PartUpdated {
            session_id,
            part: updated.clone(),
        });
        Ok(updated)
    }

    async fn complete_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_id: i64,
        outcome: RunOutcome,
    ) -> Result<Part, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        self.flush_streaming_run(session_id, &owner, run_id).await?;
        let marker = self
            .engine
            .complete_run(session_id, &owner, run_id, outcome, self.now())
            .await?;
        let meta = self.engine.session_meta(session_id).await?;
        self.memory.apply_committed(
            session_id,
            std::slice::from_ref(&marker),
            Some(meta.version),
        );
        self.bus.emit(SessionChange::PartUpdated {
            session_id,
            part: marker.clone(),
        });
        self.bus
            .emit(SessionChange::SessionMetaUpdated { session_id, meta });
        Ok(marker)
    }

    async fn heartbeat_lease(&self, session_id: i64, owner_id: &str) -> Result<bool, StoreError> {
        let owner = self.owner(owner_id);
        self.engine
            .heartbeat_lease(session_id, &owner, self.now())
            .await
    }

    async fn start_run(
        &self,
        session_id: i64,
        owner_id: &str,
        run_kind: &str,
        content: Value,
        idempotency_key: Option<String>,
    ) -> Result<SubmitOutcome, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        let outcome = self
            .engine
            .start_run(
                session_id,
                &owner,
                run_kind,
                content,
                idempotency_key,
                self.now(),
            )
            .await?;
        if outcome.created {
            let meta = self.engine.session_meta(session_id).await?;
            self.memory
                .apply_committed(session_id, &outcome.parts, Some(meta.version));
            for part in &outcome.parts {
                self.bus.emit(SessionChange::PartAdded {
                    session_id,
                    part: part.clone(),
                });
            }
            self.bus
                .emit(SessionChange::SessionMetaUpdated { session_id, meta });
        }
        Ok(outcome)
    }

    async fn set_provider_anchors(
        &self,
        session_id: i64,
        anchors: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let meta = self
            .engine
            .set_provider_anchors(session_id, anchors)
            .await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: meta.clone(),
        });
        Ok(meta)
    }

    async fn set_config_json(
        &self,
        session_id: i64,
        config: Option<Value>,
    ) -> Result<SessionMeta, StoreError> {
        let meta = self.engine.set_config_json(session_id, config).await?;
        self.memory.invalidate(session_id);
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: meta.clone(),
        });
        Ok(meta)
    }

    async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError> {
        self.engine.record_usage(record).await
    }

    async fn answer_interaction(
        &self,
        session_id: i64,
        owner_id: &str,
        interaction_part_id: i64,
        reply: NewPart,
    ) -> Result<(), StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        let outcome = self
            .engine
            .answer_interaction(session_id, &owner, interaction_part_id, reply, self.now())
            .await?;
        let meta = self.engine.session_meta(session_id).await?;
        let committed = [outcome.interaction.clone(), outcome.reply.clone()];
        self.memory
            .apply_committed(session_id, &committed, Some(meta.version));
        self.bus.emit(SessionChange::PartUpdated {
            session_id,
            part: outcome.interaction,
        });
        self.bus.emit(SessionChange::PartAdded {
            session_id,
            part: outcome.reply,
        });
        Ok(())
    }

    async fn fork(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
    ) -> Result<i64, StoreError> {
        let meta = self
            .engine
            .fork_session(session_id, at_part_id, title.clone(), false, self.now())
            .await?;
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id: meta.id,
            meta: meta.clone(),
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
            session_id: meta.id,
            meta: meta.clone(),
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
    ) -> Result<Vec<Part>, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        self.flush_streaming_run(session_id, &owner, run_id).await?;
        let updated_parts = self
            .engine
            .cancel_run(session_id, &owner, run_id, self.now())
            .await?;
        if !updated_parts.is_empty() {
            let meta = self.engine.session_meta(session_id).await?;
            self.memory
                .apply_committed(session_id, &updated_parts, Some(meta.version));
            for part in &updated_parts {
                self.bus.emit(SessionChange::PartUpdated {
                    session_id,
                    part: part.clone(),
                });
            }
            self.bus
                .emit(SessionChange::SessionMetaUpdated { session_id, meta });
        }
        Ok(updated_parts)
    }

    async fn reconcile(&self, session_id: i64) -> Result<(), StoreError> {
        SessionFacade::reconcile(self, session_id).await
    }

    async fn compact_session(
        &self,
        session_id: i64,
        owner_id: &str,
        summary: Option<String>,
        window: Option<String>,
    ) -> Result<i64, StoreError> {
        let owner = self.owner(owner_id);
        self.ensure_lease(session_id, &owner).await?;
        // A compaction run marker closes the preceding window and records the
        // durable checkpoint as its content (4.1.1 `CompactionContent`,
        // 13.4); provider anchors are cleared because compaction changes the
        // prompt window (13.3).
        let outcome = self
            .engine
            .start_run(
                session_id,
                &owner,
                "compaction",
                json!({ "summary": summary, "window": window }),
                None,
                self.now(),
            )
            .await?;
        self.engine.set_provider_anchors(session_id, None).await?;
        let meta = self.engine.session_meta(session_id).await?;
        self.memory
            .apply_committed(session_id, &outcome.parts, Some(meta.version));
        for part in &outcome.parts {
            self.bus.emit(SessionChange::PartAdded {
                session_id,
                part: part.clone(),
            });
        }
        self.bus
            .emit(SessionChange::SessionMetaUpdated { session_id, meta });
        Ok(outcome.run_id)
    }

    async fn delete(&self, session_id: i64) -> Result<(), StoreError> {
        let target = self.engine.session_meta(session_id).await?;
        let tree = self.engine.list_session_tree(target.root_id).await?;
        let mut deleted_session_ids = HashSet::from([session_id]);
        loop {
            let before = deleted_session_ids.len();
            for summary in &tree {
                if summary
                    .parent_id
                    .is_some_and(|parent_id| deleted_session_ids.contains(&parent_id))
                {
                    deleted_session_ids.insert(summary.id);
                }
            }
            if deleted_session_ids.len() == before {
                break;
            }
        }
        let mut removed_memberships = Vec::new();
        for deleted_id in &deleted_session_ids {
            let view = self.engine.load_session(*deleted_id).await?;
            removed_memberships.push((
                *deleted_id,
                view.parts
                    .into_iter()
                    .map(|part| part.part_id)
                    .collect::<Vec<_>>(),
            ));
        }
        self.engine.delete_session(session_id).await?;
        removed_memberships.sort_by_key(|(deleted_id, _)| *deleted_id);
        for (deleted_id, part_ids) in removed_memberships {
            self.memory.clear_streaming_session(deleted_id);
            self.memory.invalidate(deleted_id);
            for part_id in part_ids {
                self.bus.emit(SessionChange::PartRemoved {
                    session_id: deleted_id,
                    part_id,
                });
            }
        }
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
        let session_id = self
            .engine
            .import_session_jsonl(workspace_id, bundle, self.now())
            .await?;
        let view = self.engine.load_session(session_id).await?;
        self.bus.emit(SessionChange::SessionMetaUpdated {
            session_id,
            meta: view.meta,
        });
        for part in view.parts {
            self.bus.emit(SessionChange::PartAdded { session_id, part });
        }
        Ok(session_id)
    }

    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError> {
        self.engine.usage_stats(query).await
    }

    async fn maintenance(&self, now_ms: i64) -> Result<MaintenanceOutcome, StoreError> {
        let outcome = self.engine.maintenance(now_ms).await?;
        for session_id in &outcome.reaped_sessions {
            self.memory.clear_streaming_session(*session_id);
            self.memory.invalidate(*session_id);
            let meta = self.engine.session_meta(*session_id).await?;
            self.bus.emit(SessionChange::SessionMetaUpdated {
                session_id: *session_id,
                meta,
            });
        }
        Ok(outcome)
    }

    fn subscribe(&self, session_id: i64, observer: SessionObserver) -> Subscription {
        let observer_id = self.bus.subscribe(session_id, observer);
        Subscription {
            session_id,
            observer_id,
            bus: Arc::clone(&self.bus),
        }
    }

    fn subscribe_all(&self, observer: SessionObserver) -> GlobalSubscription {
        let observer_id = self.bus.subscribe_all(observer);
        GlobalSubscription {
            observer_id,
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

/// Apply a streaming delta to the in-memory checkpoint without advancing the
/// persisted revision. The engine advances revision once when the coalesced
/// checkpoint is committed.
fn apply_buffered_delta(
    part: &mut Part,
    delta: PartDelta,
    now_ms: i64,
) -> Result<bool, StoreError> {
    let state_changed = delta.state.is_some_and(|state| state != part.state);
    if let Some(state) = delta.state {
        apply_part_transition(part, state, now_ms, true)?;
    }
    if let Some(content) = delta.content {
        part.content = content;
    } else if let Some(delta_text) = delta.content_text_delta {
        append_buffered_text_delta(&mut part.content, &delta_text)?;
    }
    if let Some(summary) = delta.summary {
        part.summary = Some(summary);
    }
    if let Some(rendered_markdown) = delta.rendered_markdown {
        part.rendered_markdown = Some(rendered_markdown);
    }
    if let Some(provider_state) = delta.provider_state {
        part.provider_state = Some(provider_state);
    }
    if let Some(finished_at_ms) = delta.finished_at_ms {
        part.finished_at_ms = Some(finished_at_ms);
    }
    if part.state.is_terminal() && part.finished_at_ms.is_none() {
        part.finished_at_ms = Some(now_ms);
    }
    if part.state == super::PartState::InProgress {
        part.finished_at_ms = None;
    }
    part.updated_at_ms = now_ms;
    Ok(state_changed)
}

fn append_buffered_text_delta(content: &mut Value, delta: &str) -> Result<(), StoreError> {
    match content {
        Value::String(text) => {
            text.push_str(delta);
            Ok(())
        }
        Value::Object(map) => match map.get_mut("text") {
            Some(Value::String(text)) => {
                text.push_str(delta);
                Ok(())
            }
            _ => Err(StoreError::InvalidState(
                "content_text_delta requires a text-shaped content".to_owned(),
            )),
        },
        _ => Err(StoreError::InvalidState(
            "content_text_delta requires a text-shaped content".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        InMemoryEngine, InteractionAnswerOutcome, LeaseState, NewSession, PartRole, PartState,
        ReconcileOutcome, SessionChange, SessionListQuery, SessionState,
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

    /// A ready root session with a fresh lease held by `owner-a`. Generic over
    /// the engine so tests can drive `SessionFacade<CountingEngine>` too.
    async fn ready_session<E: PersistenceEngine>(
        facade: &SessionFacade<E>,
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

        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "hello"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
        let changes = seen.lock().unwrap().clone();
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, SessionChange::PartAdded { .. })),
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
    async fn text_stream_deltas_are_amortized_and_run_end_flushes_the_tail() {
        let clock = Clock::new(1_000_000);
        let engine = InMemoryEngine::default();
        let facade = SessionFacade::with_clock(
            engine.clone(),
            "owner-a",
            MemoryLayer::new(16),
            NotificationBus::new(),
            {
                let clock = clock.clone();
                move || clock.get()
            },
        )
        .with_streaming_flush_delta_count(3);
        let session_id = ready_session(&facade, 1, "stream").await;
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"text": ""}),
                    summary: None,
                    visibility: crate::store::PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
            )
            .await
            .expect("start streamed part");
        let run_id = outcome.run_id;
        let part_id = engine
            .load_session(session_id)
            .await
            .expect("load persisted stream")
            .parts[1]
            .part_id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);

        for delta in ["a", "b"] {
            facade
                .update_part(
                    session_id,
                    "owner-a",
                    part_id,
                    PartDelta {
                        content_text_delta: Some(delta.to_owned()),
                        ..Default::default()
                    },
                )
                .await
                .expect("buffer delta");
        }
        let persisted_before_threshold = engine
            .load_session(session_id)
            .await
            .expect("load before threshold");
        assert_eq!(persisted_before_threshold.parts[1].content["text"], "");
        assert_eq!(persisted_before_threshold.parts[1].revision, 1);
        assert_eq!(
            facade.load(session_id).await.expect("overlay stream").parts[1].content["text"],
            "ab",
            "same-process readers see the in-memory stream before its checkpoint"
        );

        facade
            .update_part(
                session_id,
                "owner-a",
                part_id,
                PartDelta {
                    content_text_delta: Some("c".to_owned()),
                    ..Default::default()
                },
            )
            .await
            .expect("threshold delta");
        let first_flush = engine
            .load_session(session_id)
            .await
            .expect("load first flush");
        assert_eq!(first_flush.parts[1].content["text"], "abc");
        assert_eq!(first_flush.parts[1].revision, 2, "three deltas, one write");

        for delta in ["d", "e"] {
            facade
                .update_part(
                    session_id,
                    "owner-a",
                    part_id,
                    PartDelta {
                        content_text_delta: Some(delta.to_owned()),
                        ..Default::default()
                    },
                )
                .await
                .expect("buffer tail delta");
        }
        assert_eq!(
            engine
                .load_session(session_id)
                .await
                .expect("tail remains buffered")
                .parts[1]
                .content["text"],
            "abc"
        );

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
            .expect("run completion flushes tail");
        let completed = engine
            .load_session(session_id)
            .await
            .expect("load completed stream");
        assert_eq!(completed.parts[1].content["text"], "abcde");
        assert_eq!(
            completed.parts[1].revision, 3,
            "five deltas persisted in two part updates"
        );
        let content_flushes = seen
            .lock()
            .expect("seen lock")
            .iter()
            .filter(|change| {
                matches!(
                    change,
                    SessionChange::PartUpdated { part, .. } if part.part_id == part_id
                )
            })
            .count();
        assert_eq!(content_flushes, 2, "notifications follow committed flushes");
    }

    #[tokio::test]
    async fn in_progress_content_updates_are_end_only_and_terminalize_in_one_write() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "stream").await;
        let _outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "think".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"summary": []}),
                    summary: None,
                    visibility: crate::store::PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
            )
            .await
            .expect("start streamed part");
        let part_id = facade
            .engine()
            .load_session(session_id)
            .await
            .expect("load persisted stream")
            .parts[1]
            .part_id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);

        // A long in-progress content stream (a reasoning part rewriting its
        // whole content document per token) must stay in the in-memory buffer:
        // zero durable writes and zero session version bumps until the part
        // terminalizes.
        for i in 0..40 {
            facade
                .update_part(
                    session_id,
                    "owner-a",
                    part_id,
                    PartDelta {
                        state: Some(PartState::InProgress),
                        content: Some(json!({"summary": [format!("t{i}")]})),
                        ..Default::default()
                    },
                )
                .await
                .expect("buffer content delta");
        }
        let persisted_mid_stream = facade
            .engine()
            .load_session(session_id)
            .await
            .expect("load mid-stream");
        let persisted_part = persisted_mid_stream
            .parts
            .iter()
            .find(|part| part.part_id == part_id)
            .expect("streamed part present");
        assert_eq!(
            persisted_part.revision, 1,
            "in-progress content deltas must not write the durable store"
        );
        assert_eq!(
            persisted_part.content["summary"],
            json!([]),
            "the durable row stays at its creation shape"
        );
        assert_eq!(
            facade
                .engine()
                .session_meta(session_id)
                .await
                .expect("session meta")
                .version,
            persisted_mid_stream.parts[0].part_id + 1,
            "session version must not advance on buffered content deltas"
        );
        assert_eq!(
            facade
                .load(session_id)
                .await
                .expect("overlay")
                .parts
                .iter()
                .find(|part| part.part_id == part_id)
                .expect("overlaid part")
                .content["summary"],
            json!(["t39"]),
            "same-process readers see the live stream through the buffer overlay"
        );

        // Terminalizing the part flushes exactly once.
        facade
            .update_part(
                session_id,
                "owner-a",
                part_id,
                PartDelta {
                    state: Some(PartState::Completed),
                    content: Some(json!({"summary": ["final"]})),
                    ..Default::default()
                },
            )
            .await
            .expect("terminalize flushes");
        let persisted_after = facade
            .engine()
            .load_session(session_id)
            .await
            .expect("load after terminalize");
        let terminal_part = persisted_after
            .parts
            .iter()
            .find(|part| part.part_id == part_id)
            .expect("terminal part present");
        assert_eq!(
            terminal_part.revision, 2,
            "one terminal write commits all deltas"
        );
        assert_eq!(terminal_part.content["summary"], json!(["final"]));
        let content_flushes = seen
            .lock()
            .expect("seen lock")
            .iter()
            .filter(|change| {
                matches!(
                    change,
                    SessionChange::PartUpdated { part, .. } if part.part_id == part_id
                )
            })
            .count();
        assert_eq!(
            content_flushes, 1,
            "one notification for the end-only flush"
        );
    }

    #[tokio::test]
    async fn duplicate_owner_writes_are_refused_and_resume_heals() {
        let (facade, clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;

        // A second owner cannot write while owner-a's lease is fresh.
        let err = facade
            .submit_user_run(
                session_id,
                "owner-b",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "x"}),
                )],
                None,
            )
            .await
            .expect_err("owner-b refused");
        assert!(matches!(err, StoreError::LeaseHeldByOther { .. }));

        // owner-a's lease goes stale; owner-b acquires, which reconciles the
        // (now stale) owner-a state and lets owner-b write.
        clock.advance(60_000);
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-b",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "y"}),
                )],
                None,
            )
            .await
            .expect("owner-b acquires after staleness");
        assert!(outcome.run_id > 0);
    }

    #[tokio::test]
    async fn session_state_derives_running_awaiting_and_ready() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;

        let ready = facade.session_state(session_id).await.expect("state");
        assert_eq!(ready.state, SessionState::Ready);

        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "hi"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
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
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "q"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
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
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "hello"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
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
        clock.advance(1);

        let view = facade.load(session_id).await.expect("load");
        let cutoff = view.parts[0].part_id; // the marker

        let fork_id = facade
            .fork(session_id, cutoff, "fork".to_owned())
            .await
            .expect("fork");
        let fork_view = facade.load(fork_id).await.expect("fork view");
        assert_eq!(
            fork_view.parts.len(),
            1,
            "fork copies edges up to the marker"
        );

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
            .compact_session(session_id, "owner-a", Some("checkpoint".to_owned()), None)
            .await
            .expect("compact");
        assert!(compaction_id > 0);
        let meta = facade
            .engine()
            .session_meta(session_id)
            .await
            .expect("meta");
        assert!(meta.provider_anchors_json.is_none(), "anchors cleared");
        let view = facade.load(session_id).await.expect("load");
        assert!(
            view.parts.iter().any(|p| p.is_run_marker()),
            "compaction run marker created"
        );
    }

    #[tokio::test]
    async fn compaction_part_content_records_the_checkpoint_summary() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let summary = "durable continuation record for the next agent step";
        facade
            .compact_session(
                session_id,
                "owner-a",
                Some(summary.to_owned()),
                Some("through:42".to_owned()),
            )
            .await
            .expect("compact");
        let view = facade.load(session_id).await.expect("load");
        let compaction = view
            .parts
            .iter()
            .find(|part| part.content["run_kind"] == json!("compaction"))
            .expect("compaction run marker");
        assert_eq!(
            compaction.content["summary"],
            json!(summary),
            "summary persisted on the compaction part"
        );
        assert_eq!(
            compaction.content["window"],
            json!("through:42"),
            "window description persisted on the compaction part"
        );
    }

    #[tokio::test]
    async fn idempotency_key_deduplicates_user_send_through_the_facade() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let parts = vec![NewPart::pending(
            "text",
            PartRole::User,
            json!({"text": "once"}),
        )];
        let first = facade
            .submit_user_run(session_id, "owner-a", parts.clone(), Some("k1".to_owned()))
            .await
            .expect("first");
        let second = facade
            .submit_user_run(session_id, "owner-a", parts.clone(), Some("k1".to_owned()))
            .await
            .expect("replay");
        // The replay resolves to the same run but is not a re-creation: the
        // marker is not re-emitted (`created == false`) while the run id and
        // content parts match.
        assert_eq!(
            first.run_id, second.run_id,
            "replay returns the same run id"
        );
        assert!(!second.created, "replay is not a re-creation");
        let view = facade.load(session_id).await.expect("load");
        assert_eq!(view.parts.len(), 2, "no duplicate parts");
    }

    #[tokio::test]
    async fn delete_removes_the_session_and_emits_removal() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "delete me"}),
                )],
                None,
            )
            .await
            .expect("submit before delete");
        let removed_ids = facade
            .load(session_id)
            .await
            .expect("load before delete")
            .parts
            .into_iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().unwrap().push(change))
        };
        let _subscription = facade.subscribe(session_id, observer);
        facade.delete(session_id).await.expect("delete");
        let emitted_ids = {
            let changes = seen.lock().unwrap();
            changes
                .iter()
                .filter_map(|change| match change {
                    SessionChange::PartRemoved { part_id, .. } => Some(*part_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(emitted_ids, removed_ids, "one patch per removed membership");
        assert!(emitted_ids.iter().all(|part_id| *part_id > 0));
        let err = facade.load(session_id).await.expect_err("gone");
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn root_delete_emits_membership_removals_for_descendants() {
        let (facade, _clock) = harness();
        let root_id = ready_session(&facade, 1, "delete tree").await;
        let outcome = facade
            .submit_user_run(
                root_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "shared"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
        let root_part_ids = facade
            .load(root_id)
            .await
            .expect("root view")
            .parts
            .into_iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        let child_id = facade
            .fork(root_id, run_id, "child".to_owned())
            .await
            .expect("fork");
        let child_part_ids = facade
            .load(child_id)
            .await
            .expect("child view")
            .parts
            .into_iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe_all({
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        facade.delete(root_id).await.expect("delete tree");

        let changes = seen.lock().expect("seen lock");
        let removed_for = |expected_session_id| {
            changes
                .iter()
                .filter_map(|change| match change {
                    SessionChange::PartRemoved {
                        session_id,
                        part_id,
                    } if *session_id == expected_session_id => Some(*part_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(removed_for(root_id), root_part_ids);
        assert_eq!(removed_for(child_id), child_part_ids);
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
        let meta = facade
            .rename(session_id, "new title".to_owned())
            .await
            .expect("rename");
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
            listed
                .iter()
                .any(|s| s.id == session_id && s.title == "new title"),
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
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "export me"}),
                )],
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
        assert_eq!(
            imported_view.parts.len(),
            2,
            "marker + text survive the round trip"
        );
        assert_eq!(imported_view.parts[1].content["text"], "export me");
    }

    #[tokio::test]
    async fn create_session_through_the_facade_returns_meta_and_notifies() {
        let (facade, _clock) = harness();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: SessionObserver = {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().unwrap().push(change))
        };
        // No subscriber before creation; subscribe to the new id after it
        // exists by scanning the emitted change.
        let meta = facade
            .create_session(NewSession {
                workspace_id: 1,
                parent_id: None,
                relation_kind: SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "root".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create root");
        assert_eq!(meta.relation_kind, SessionRelationKind::Root);
        assert!(meta.id > 0);

        let _subscription = facade.subscribe(meta.id, observer);
        let child = facade
            .create_session(NewSession {
                workspace_id: 1,
                parent_id: Some(meta.id),
                relation_kind: SessionRelationKind::Child,
                cutoff_part_id: None,
                title: "child".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create child");
        assert_eq!(
            child.depth,
            meta.depth + 1,
            "depth matches the schema invariant"
        );
        assert_eq!(child.root_id, meta.id, "child inherits the root");
    }

    #[tokio::test]
    async fn start_run_starts_a_non_user_run_and_returns_its_marker_id() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let outcome = facade
            .start_run(
                session_id,
                "owner-a",
                "background",
                json!({"kind": "background", "prompt": "do a thing"}),
                None,
            )
            .await
            .expect("start background run");
        let run_id = outcome.run_id;
        assert!(run_id > 0);
        let view = facade.load(session_id).await.expect("load");
        let marker = view
            .parts
            .iter()
            .find(|p| p.part_id == run_id)
            .expect("marker exists");
        assert_eq!(marker.kind, "run");
        assert_eq!(marker.content["run_kind"], "background");
        assert!(marker.state.is_in_flight());
    }

    #[tokio::test]
    async fn anchors_and_config_json_are_set_through_the_facade() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        let anchors = json!({"claude": {"anchor": "abc"}});
        let config = json!({"execution": {"access": "read_only"}});
        facade
            .set_provider_anchors(session_id, Some(anchors.clone()))
            .await
            .expect("set anchors");
        facade
            .set_config_json(session_id, Some(config.clone()))
            .await
            .expect("set config");
        let meta = facade
            .engine()
            .session_meta(session_id)
            .await
            .expect("meta");
        assert_eq!(meta.provider_anchors_json, Some(anchors));
        assert_eq!(meta.config_json, Some(config));
    }

    #[tokio::test]
    async fn record_usage_through_the_facade_is_reported_in_stats() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "t").await;
        facade
            .record_usage(crate::store::UsageRecord {
                workspace_id: 1,
                session_id,
                run_id: None,
                provider_id: "anthropic".to_owned(),
                model_id: "claude-5".to_owned(),
                created_at_ms: facade.now(),
                input_tokens: 5,
                output_tokens: 9,
                reasoning_tokens: 1,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                tool_use_tokens: 0,
                other_tokens: 0,
                total_cost_micros: 42,
                recorded_cost_micros: None,
                cost_estimate_incomplete: false,
                detail_json: None,
            })
            .await
            .expect("record usage");
        let stats = facade
            .usage_stats(crate::store::UsageQuery {
                session_id: Some(session_id),
                ..Default::default()
            })
            .await
            .expect("stats");
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.groups[0].input_tokens, 5);
        assert_eq!(stats.groups[0].output_tokens, 9);
    }

    #[tokio::test]
    async fn subtask_helpers_find_create_and_update_subagent_sessions() {
        let (facade, _clock) = harness();
        let parent = ready_session(&facade, 1, "parent").await;
        let child_id = facade
            .create_subagent_session(parent, "task-1".to_owned(), "sub".to_owned())
            .await
            .expect("create subagent");
        let meta = facade
            .find_subagent_by_task_id(parent, "task-1")
            .await
            .expect("find")
            .expect("subagent exists");
        assert_eq!(meta.id, child_id);
        assert_eq!(meta.parent_id, Some(parent));
        assert_eq!(meta.task_id.as_deref(), Some("task-1"));

        let updated = facade
            .update_subtask_state(
                child_id,
                Some("running".to_owned()),
                Some(facade.now()),
                None,
                None,
            )
            .await
            .expect("update subtask");
        assert_eq!(updated.subtask_status.as_deref(), Some("running"));
        assert!(updated.subtask_started_at_ms.is_some());

        // Creating the same (parent, task) again must be refused.
        let err = facade
            .create_subagent_session(parent, "task-1".to_owned(), "dup".to_owned())
            .await
            .expect_err("duplicate subagent refused");
        assert!(matches!(err, StoreError::InvalidState(_)));
    }

    #[tokio::test]
    async fn dropping_one_session_subscription_keeps_the_other_active() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "subscriptions").await;
        let first_seen = Arc::new(Mutex::new(Vec::new()));
        let second_seen = Arc::new(Mutex::new(Vec::new()));
        let first = facade.subscribe(session_id, {
            let seen = Arc::clone(&first_seen);
            Arc::new(move |change| seen.lock().expect("first seen lock").push(change))
        });
        let _second = facade.subscribe(session_id, {
            let seen = Arc::clone(&second_seen);
            Arc::new(move |change| seen.lock().expect("second seen lock").push(change))
        });

        facade
            .rename(session_id, "first rename".to_owned())
            .await
            .expect("first rename");
        drop(first);
        facade
            .rename(session_id, "second rename".to_owned())
            .await
            .expect("second rename");

        assert_eq!(first_seen.lock().expect("first seen lock").len(), 1);
        assert_eq!(second_seen.lock().expect("second seen lock").len(), 2);
    }

    #[tokio::test]
    async fn dropping_one_global_subscription_keeps_the_other_active() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "global subscriptions").await;
        let first_seen = Arc::new(Mutex::new(Vec::new()));
        let second_seen = Arc::new(Mutex::new(Vec::new()));
        let first = facade.subscribe_all({
            let seen = Arc::clone(&first_seen);
            Arc::new(move |change| seen.lock().expect("first seen lock").push(change))
        });
        let _second = facade.subscribe_all({
            let seen = Arc::clone(&second_seen);
            Arc::new(move |change| seen.lock().expect("second seen lock").push(change))
        });

        facade
            .rename(session_id, "first rename".to_owned())
            .await
            .expect("first rename");
        drop(first);
        facade
            .rename(session_id, "second rename".to_owned())
            .await
            .expect("second rename");

        assert_eq!(first_seen.lock().expect("first seen lock").len(), 1);
        assert_eq!(second_seen.lock().expect("second seen lock").len(), 2);
    }

    #[tokio::test]
    async fn answer_interaction_emits_updated_interaction_before_added_reply() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "answer patches").await;
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Assistant,
                    json!({"kind": "ask_user", "prompt": "Continue?"}),
                )],
                None,
            )
            .await
            .expect("submit interaction");
        let run_id = outcome.run_id;
        let interaction_id = facade
            .load(session_id)
            .await
            .expect("load interaction")
            .parts[1]
            .part_id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe(session_id, {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        facade
            .answer_interaction(
                session_id,
                "owner-a",
                interaction_id,
                NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
            )
            .await
            .expect("answer interaction");

        let changes = seen.lock().expect("seen lock");
        assert_eq!(changes.len(), 2);
        match &changes[0] {
            SessionChange::PartUpdated { part, .. } => {
                assert_eq!(part.part_id, interaction_id);
                assert_eq!(part.state, PartState::Completed);
                assert_eq!(part.revision, 2);
            }
            other => panic!("expected interaction update first, got {other:?}"),
        }
        match &changes[1] {
            SessionChange::PartAdded { part, .. } => {
                assert_eq!(part.parent_part_id, Some(interaction_id));
                assert_eq!(part.run_id, Some(run_id));
            }
            other => panic!("expected reply addition second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_emits_only_changed_marker_and_child_before_meta() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "cancel patches").await;
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![
                    NewPart {
                        kind: "text".to_owned(),
                        role: PartRole::Assistant,
                        content: json!({"text": "streaming"}),
                        summary: None,
                        visibility: crate::store::PartVisibility::Both,
                        rendered_markdown: None,
                        parent_part_id: None,
                        state: PartState::InProgress,
                    },
                    NewPart {
                        kind: "notice".to_owned(),
                        role: PartRole::Runtime,
                        content: json!({"message": "already done"}),
                        summary: None,
                        visibility: crate::store::PartVisibility::Both,
                        rendered_markdown: None,
                        parent_part_id: None,
                        state: PartState::Completed,
                    },
                ],
                None,
            )
            .await
            .expect("submit run");
        let run_id = outcome.run_id;
        let child_id = facade.load(session_id).await.expect("load run").parts[1].part_id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe(session_id, {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        facade
            .cancel_run(session_id, "owner-a", run_id)
            .await
            .expect("cancel run");

        let changes = seen.lock().expect("seen lock");
        assert_eq!(changes.len(), 3, "two row patches followed by meta");
        assert!(matches!(
            &changes[0],
            SessionChange::PartUpdated { part, .. }
                if part.part_id == run_id && part.state == PartState::Cancelled
        ));
        assert!(matches!(
            &changes[1],
            SessionChange::PartUpdated { part, .. }
                if part.part_id == child_id && part.state == PartState::Cancelled
        ));
        assert!(matches!(
            &changes[2],
            SessionChange::SessionMetaUpdated { .. }
        ));
    }

    #[tokio::test]
    async fn reconcile_emits_committed_part_updates_and_preserves_live_or_paused_runs() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "reconcile patches").await;
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"text": "partial"}),
                    summary: None,
                    visibility: crate::store::PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
            )
            .await
            .expect("submit run");
        let run_id = outcome.run_id;
        let child_id = facade.load(session_id).await.expect("load run").parts[1].part_id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe(session_id, {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        facade
            .reconcile(session_id)
            .await
            .expect("fresh run is preserved");
        assert!(seen.lock().expect("seen lock").is_empty());
        facade
            .engine()
            .release_lease(session_id, "owner-a")
            .await
            .expect("release lease");
        facade
            .reconcile(session_id)
            .await
            .expect("reconcile interrupted run");

        {
            let changes = seen.lock().expect("seen lock");
            assert_eq!(changes.len(), 3);
            assert!(matches!(
                &changes[0],
                SessionChange::PartUpdated { part, .. }
                    if part.part_id == run_id
                        && part.state == PartState::Failed
                        && part.content["abort_reason"] == "process_restart"
            ));
            assert!(matches!(
                &changes[1],
                SessionChange::PartUpdated { part, .. }
                    if part.part_id == child_id && part.state == PartState::Cancelled
            ));
            assert!(matches!(
                &changes[2],
                SessionChange::SessionMetaUpdated { .. }
            ));
        }

        facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "interaction",
                    PartRole::Assistant,
                    json!({"kind": "ask_user", "prompt": "paused?"}),
                )],
                None,
            )
            .await
            .expect("start paused run");
        facade
            .engine()
            .release_lease(session_id, "owner-a")
            .await
            .expect("release paused lease");
        seen.lock().expect("seen lock").clear();
        facade
            .reconcile(session_id)
            .await
            .expect("paused run is preserved");
        assert!(seen.lock().expect("seen lock").is_empty());
        assert_eq!(
            facade.session_state(session_id).await.expect("state").state,
            SessionState::AwaitingUser
        );
    }

    #[tokio::test]
    async fn fork_and_import_announce_the_new_session_to_global_subscribers() {
        let (facade, _clock) = harness();
        let session_id = ready_session(&facade, 1, "global creation patches").await;
        facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "shared"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let source = facade.load(session_id).await.expect("source view");
        let cutoff = source.parts[0].part_id;
        let bundle = facade
            .export_session_jsonl(session_id)
            .await
            .expect("export");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe_all({
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        let fork_id = facade
            .fork(session_id, cutoff, "fork".to_owned())
            .await
            .expect("fork");
        {
            let changes = seen.lock().expect("seen lock");
            assert_eq!(changes.len(), 1);
            assert!(matches!(
                &changes[0],
                SessionChange::SessionMetaUpdated { session_id, meta }
                    if *session_id == fork_id && meta.id == fork_id
            ));
        }
        seen.lock().expect("seen lock").clear();

        let imported_id = facade
            .import_session_jsonl(1, &bundle)
            .await
            .expect("import");
        let changes = seen.lock().expect("seen lock");
        assert_eq!(changes.len(), source.parts.len() + 1);
        assert!(matches!(
            &changes[0],
            SessionChange::SessionMetaUpdated { session_id, meta }
                if *session_id == imported_id && meta.id == imported_id
        ));
        assert!(changes[1..].iter().all(|change| matches!(
            change,
            SessionChange::PartAdded { session_id, part }
                if *session_id == imported_id && part.origin_session_id == imported_id
        )));
    }

    #[tokio::test]
    async fn maintenance_notifies_reaped_sessions_to_refresh_derived_state() {
        let (facade, clock) = harness();
        let session_id = ready_session(&facade, 1, "maintenance patch").await;
        facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::Assistant,
                    json!({"text": "working"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _subscription = facade.subscribe(session_id, {
            let seen = Arc::clone(&seen);
            Arc::new(move |change| seen.lock().expect("seen lock").push(change))
        });

        clock.advance(LEASE_STALENESS_MS + 1);
        let outcome = facade.maintenance(clock.get()).await.expect("maintenance");
        assert_eq!(outcome.reaped_sessions, vec![session_id]);
        assert!(matches!(
            seen.lock().expect("seen lock").as_slice(),
            [SessionChange::SessionMetaUpdated { session_id: changed, .. }] if *changed == session_id
        ));
        assert_eq!(
            facade.session_state(session_id).await.expect("state").state,
            SessionState::Interrupted
        );
    }

    /// A `PersistenceEngine` test double wrapping `InMemoryEngine` that counts
    /// every `load_session` call, so a test can prove the facade write path no
    /// longer forces an engine reload (R1: `apply_committed` seeds the memory
    /// cache). All other methods are mechanical forwards.
    struct CountingEngine {
        inner: InMemoryEngine,
        load_session_calls: AtomicU64,
    }

    impl CountingEngine {
        fn new() -> Self {
            Self {
                inner: InMemoryEngine::default(),
                load_session_calls: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl PersistenceEngine for CountingEngine {
        async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError> {
            self.inner.create_session(new_session).await
        }

        async fn session_meta(&self, session_id: i64) -> Result<SessionMeta, StoreError> {
            self.inner.session_meta(session_id).await
        }

        async fn load_session(&self, session_id: i64) -> Result<SessionView, StoreError> {
            self.load_session_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.load_session(session_id).await
        }

        async fn newest_member_cursor(
            &self,
            session_id: i64,
        ) -> Result<Option<(i64, i64)>, StoreError> {
            self.inner.newest_member_cursor(session_id).await
        }

        async fn rename_session(
            &self,
            session_id: i64,
            title: String,
        ) -> Result<SessionMeta, StoreError> {
            self.inner.rename_session(session_id, title).await
        }

        async fn set_provider_anchors(
            &self,
            session_id: i64,
            anchors: Option<Value>,
        ) -> Result<SessionMeta, StoreError> {
            self.inner.set_provider_anchors(session_id, anchors).await
        }

        async fn set_config_json(
            &self,
            session_id: i64,
            config: Option<Value>,
        ) -> Result<SessionMeta, StoreError> {
            self.inner.set_config_json(session_id, config).await
        }

        async fn find_subagent_by_task_id(
            &self,
            parent_session_id: i64,
            task_id: &str,
        ) -> Result<Option<SessionMeta>, StoreError> {
            self.inner
                .find_subagent_by_task_id(parent_session_id, task_id)
                .await
        }

        async fn create_subagent_session(
            &self,
            parent_session_id: i64,
            task_id: String,
            title: String,
            now_ms: i64,
        ) -> Result<SessionMeta, StoreError> {
            self.inner
                .create_subagent_session(parent_session_id, task_id, title, now_ms)
                .await
        }

        async fn update_subtask_state(
            &self,
            session_id: i64,
            status: Option<String>,
            started_at_ms: Option<i64>,
            finished_at_ms: Option<i64>,
            failure: Option<Value>,
        ) -> Result<SessionMeta, StoreError> {
            self.inner
                .update_subtask_state(session_id, status, started_at_ms, finished_at_ms, failure)
                .await
        }

        async fn list_session_summaries(
            &self,
            query: SessionListQuery,
        ) -> Result<Vec<SessionSummary>, StoreError> {
            self.inner.list_session_summaries(query).await
        }

        async fn get_session_summary(
            &self,
            session_id: i64,
        ) -> Result<Option<SessionSummary>, StoreError> {
            self.inner.get_session_summary(session_id).await
        }

        async fn session_counts_by_workspace(
            &self,
            workspace_ids: &[i64],
        ) -> Result<HashMap<i64, i64>, StoreError> {
            self.inner.session_counts_by_workspace(workspace_ids).await
        }

        async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError> {
            self.inner.list_session_tree(root_id).await
        }

        async fn delete_session(&self, session_id: i64) -> Result<(), StoreError> {
            self.inner.delete_session(session_id).await
        }

        async fn try_acquire_lease(
            &self,
            session_id: i64,
            owner_id: &str,
            now_ms: i64,
        ) -> Result<LeaseAcquire, StoreError> {
            self.inner
                .try_acquire_lease(session_id, owner_id, now_ms)
                .await
        }

        async fn heartbeat_lease(
            &self,
            session_id: i64,
            owner_id: &str,
            now_ms: i64,
        ) -> Result<bool, StoreError> {
            self.inner
                .heartbeat_lease(session_id, owner_id, now_ms)
                .await
        }

        async fn release_lease(&self, session_id: i64, owner_id: &str) -> Result<bool, StoreError> {
            self.inner.release_lease(session_id, owner_id).await
        }

        async fn current_lease(&self, session_id: i64) -> Result<Option<LeaseState>, StoreError> {
            self.inner.current_lease(session_id).await
        }

        async fn reap_stale_leases(&self, stale_before_ms: i64) -> Result<Vec<i64>, StoreError> {
            self.inner.reap_stale_leases(stale_before_ms).await
        }

        async fn submit_user_run(
            &self,
            session_id: i64,
            owner_id: &str,
            parts: Vec<NewPart>,
            idempotency_key: Option<String>,
            now_ms: i64,
        ) -> Result<SubmitOutcome, StoreError> {
            self.inner
                .submit_user_run(session_id, owner_id, parts, idempotency_key, now_ms)
                .await
        }

        async fn append_parts(
            &self,
            session_id: i64,
            owner_id: &str,
            run_id: i64,
            parts: Vec<NewPart>,
            now_ms: i64,
        ) -> Result<Vec<Part>, StoreError> {
            self.inner
                .append_parts(session_id, owner_id, run_id, parts, now_ms)
                .await
        }

        async fn update_part(
            &self,
            session_id: i64,
            owner_id: &str,
            part_id: i64,
            delta: PartDelta,
            now_ms: i64,
        ) -> Result<Part, StoreError> {
            self.inner
                .update_part(session_id, owner_id, part_id, delta, now_ms)
                .await
        }

        async fn complete_run(
            &self,
            session_id: i64,
            owner_id: &str,
            run_id: i64,
            outcome: RunOutcome,
            now_ms: i64,
        ) -> Result<Part, StoreError> {
            self.inner
                .complete_run(session_id, owner_id, run_id, outcome, now_ms)
                .await
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
            self.inner
                .start_run(
                    session_id,
                    owner_id,
                    run_kind,
                    content,
                    idempotency_key,
                    now_ms,
                )
                .await
        }

        async fn cancel_run(
            &self,
            session_id: i64,
            owner_id: &str,
            run_id: i64,
            now_ms: i64,
        ) -> Result<Vec<Part>, StoreError> {
            self.inner
                .cancel_run(session_id, owner_id, run_id, now_ms)
                .await
        }

        async fn answer_interaction(
            &self,
            session_id: i64,
            owner_id: &str,
            interaction_part_id: i64,
            reply: NewPart,
            now_ms: i64,
        ) -> Result<InteractionAnswerOutcome, StoreError> {
            self.inner
                .answer_interaction(session_id, owner_id, interaction_part_id, reply, now_ms)
                .await
        }

        async fn fork_session(
            &self,
            session_id: i64,
            at_part_id: i64,
            title: String,
            rewind: bool,
            now_ms: i64,
        ) -> Result<SessionMeta, StoreError> {
            self.inner
                .fork_session(session_id, at_part_id, title, rewind, now_ms)
                .await
        }

        async fn reconcile(
            &self,
            session_id: i64,
            now_ms: i64,
        ) -> Result<ReconcileOutcome, StoreError> {
            self.inner.reconcile(session_id, now_ms).await
        }

        async fn maintenance(&self, now_ms: i64) -> Result<MaintenanceOutcome, StoreError> {
            self.inner.maintenance(now_ms).await
        }

        async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError> {
            self.inner.record_usage(record).await
        }

        async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError> {
            self.inner.usage_stats(query).await
        }

        async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError> {
            self.inner.export_session_jsonl(session_id).await
        }

        async fn import_session_jsonl(
            &self,
            workspace_id: i64,
            bundle: &str,
            now_ms: i64,
        ) -> Result<i64, StoreError> {
            self.inner
                .import_session_jsonl(workspace_id, bundle, now_ms)
                .await
        }
    }

    #[tokio::test]
    async fn writes_seed_cache_without_engine_reload() {
        let clock = Clock::new(1_000_000);
        let engine = CountingEngine::new();
        let facade = SessionFacade::with_clock(
            engine,
            "owner-a",
            MemoryLayer::new(16),
            NotificationBus::new(),
            {
                let clock = clock.clone();
                move || clock.get()
            },
        );
        let session_id = ready_session(&facade, 1, "t").await;

        // Warm the cache: the first `load` must hit the engine exactly once.
        // This is also the positive control that the counter is live.
        facade.load(session_id).await.expect("warm load");
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            1,
            "first load of an unwritten session goes to the engine"
        );
        facade
            .engine()
            .load_session_calls
            .store(0, Ordering::SeqCst);

        // a. submit_user_run: apply_committed seeds the cache with the
        //    committed marker + parts, so the next load is a hit.
        let outcome = facade
            .submit_user_run(
                session_id,
                "owner-a",
                vec![NewPart::pending(
                    "text",
                    PartRole::User,
                    json!({"text": "hello"}),
                )],
                None,
            )
            .await
            .expect("submit");
        let run_id = outcome.run_id;
        let view = facade.load(session_id).await.expect("load after submit");
        assert_eq!(view.parts.len(), 2, "marker + user part");
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            0,
            "submit must not force an engine reload"
        );

        // b. append_parts: committed parts merged into the cached view.
        let appended = facade
            .append_parts(
                session_id,
                "owner-a",
                run_id,
                vec![NewPart::pending(
                    "text",
                    PartRole::Assistant,
                    json!({"text": "reply"}),
                )],
            )
            .await
            .expect("append");
        let part_id = appended[0].part_id;
        facade.load(session_id).await.expect("load after append");
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            0,
            "append must not force an engine reload"
        );

        // c. update_part with a plain (non-streaming) content replacement.
        facade
            .update_part(
                session_id,
                "owner-a",
                part_id,
                PartDelta {
                    content: Some(json!({"text": "edited"})),
                    ..Default::default()
                },
            )
            .await
            .expect("update part");
        let view = facade.load(session_id).await.expect("load after update");
        assert_eq!(
            view.parts
                .iter()
                .find(|part| part.part_id == part_id)
                .expect("updated part present")
                .content["text"],
            "edited"
        );
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            0,
            "plain update must not force an engine reload"
        );

        // d. complete_run: the terminal marker is merged in place.
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
            .expect("complete run");
        facade.load(session_id).await.expect("load after complete");
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            0,
            "complete_run must not force an engine reload"
        );

        // e. Negative control: a fresh, never-loaded session triggers exactly
        //    one engine load, proving the counter is not permanently zero.
        let fresh_id = ready_session(&facade, 1, "fresh").await;
        facade.load(fresh_id).await.expect("fresh session load");
        assert_eq!(
            facade.engine().load_session_calls.load(Ordering::SeqCst),
            1,
            "a cache-miss read of a fresh session hits the engine once"
        );
    }
}
