use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::event::bus::EventBus;
use crate::event::error::PublishError;
use agena_domain::{
    EVENT_ENVELOPE_SCHEMA_VERSION, EventEnvelope as DomainEvent, EventMeta, KindPersistence,
};
use agena_storage::{EventStoreError, SequenceAllocator};

/// Optional routing context attached to a publish call. The publisher fills
/// in the rest of the envelope (id, seq, timestamp).
#[derive(Debug, Clone, Default)]
pub struct PublishContext {
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
}

impl PublishContext {
    pub fn for_session(session_id: i64) -> Self {
        Self {
            session_id: Some(session_id),
            ..Default::default()
        }
    }
}

/// Composes a [`SequenceAllocator`], an [`EventStore`] and an [`EventBus`]
/// into the canonical write path: persist first, then broadcast. This is what
/// `agena` core depends on; it does not depend on the bus or store impls
/// directly.
pub struct EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    seq: Arc<SequenceAllocator>,
    store: Arc<dyn agena_storage::EventStore<K>>,
    bus: Arc<dyn EventBus<K>>,
    session_sequences: Arc<Mutex<HashMap<i64, Arc<Mutex<Option<i64>>>>>>,
}

impl<K> EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    pub fn new(
        seq: Arc<SequenceAllocator>,
        store: Arc<dyn agena_storage::EventStore<K>>,
        bus: Arc<dyn EventBus<K>>,
    ) -> Self {
        Self {
            seq,
            store,
            bus,
            session_sequences: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn store(&self) -> &Arc<dyn agena_storage::EventStore<K>> {
        &self.store
    }

    pub fn bus(&self) -> &Arc<dyn EventBus<K>> {
        &self.bus
    }

    /// Build an envelope with both globally and per-session monotonic
    /// sequences. Session sequence allocation is lazy and resumes from the
    /// durable store high watermark on first use after process start.
    pub async fn build(
        &self,
        ctx: PublishContext,
        kind: K,
    ) -> Result<DomainEvent<K>, EventStoreError> {
        let seq_session = if let Some(session_id) = ctx.session_id {
            let allocator = self.session_allocator(session_id).await;
            let mut next = allocator.lock().await;
            let allocated = match *next {
                Some(next) => next,
                None => self
                    .store
                    .session_high_watermark(session_id)
                    .await?
                    .unwrap_or(0)
                    .saturating_add(1)
                    .max(1),
            };
            *next = Some(allocated.saturating_add(1));
            Some(allocated)
        } else {
            None
        };
        Ok(DomainEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: self.seq.next(),
                seq_session,
                session_id: ctx.session_id,
                workspace_id: ctx.workspace_id,
                created_at: Utc::now(),
                causation_id: ctx.causation_id,
                correlation_id: ctx.correlation_id,
                envelope_schema: EVENT_ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        })
    }

    /// Resolve (and lazily create) the per-session sequence allocator.
    async fn session_allocator(&self, session_id: i64) -> Arc<Mutex<Option<i64>>> {
        let mut sequences = self.session_sequences.lock().await;
        Arc::clone(
            sequences
                .entry(session_id)
                .or_insert_with(|| Arc::new(Mutex::new(None))),
        )
    }

    /// Reallocate both the global and the per-session sequence numbers of a
    /// batch after a duplicate-sequence conflict.
    ///
    /// Global sequences simply continue from the allocator. Per-session
    /// sequences resume from the durable store high watermark so a batch that
    /// was built before another writer (for example the bootstrap reconcile
    /// pass after a process restart) persisted its events can never collide
    /// with those already-persisted rows. The allocator is never rolled back:
    /// if a concurrent publisher already advanced it past the watermark, the
    /// higher value wins.
    async fn resequence_events_from_store(
        &self,
        events: Vec<DomainEvent<K>>,
    ) -> Result<Vec<DomainEvent<K>>, EventStoreError> {
        // Read fresh watermarks before taking any per-session allocator lock
        // so a store round-trip never happens while holding a session lock.
        let mut sessions: Vec<i64> = events
            .iter()
            .filter_map(|event| event.meta.session_id)
            .collect();
        sessions.sort_unstable();
        sessions.dedup();
        let mut bases: HashMap<i64, i64> = HashMap::with_capacity(sessions.len());
        for session_id in sessions {
            let hw = self
                .store
                .session_high_watermark(session_id)
                .await?
                .unwrap_or(0);
            bases.insert(session_id, hw.saturating_add(1).max(1));
        }

        let mut initialized: HashSet<i64> = HashSet::new();
        let mut out = Vec::with_capacity(events.len());
        for mut event in events {
            event.meta.seq_global = self.seq.next();
            if let Some(session_id) = event.meta.session_id {
                let allocator = self.session_allocator(session_id).await;
                let mut next = allocator.lock().await;
                if initialized.insert(session_id) {
                    let base = bases
                        .get(&session_id)
                        .copied()
                        .unwrap_or(1)
                        .max(next.unwrap_or(1));
                    *next = Some(base);
                }
                let allocated = next.unwrap_or(1);
                *next = Some(allocated.saturating_add(1));
                event.meta.seq_session = Some(allocated);
            }
            out.push(event);
        }
        Ok(out)
    }

    async fn persist_with_retry(
        &self,
        mut events: Vec<DomainEvent<K>>,
    ) -> Result<Vec<DomainEvent<K>>, PublishError> {
        const MAX_DUPLICATE_SEQ_RETRIES: usize = 4;

        if events.is_empty() {
            return Ok(events);
        }

        let mut attempts = 0usize;
        loop {
            let persistent: Vec<DomainEvent<K>> = events
                .iter()
                .filter(|event| event.kind.is_persistent())
                .cloned()
                .collect();
            if persistent.is_empty() {
                return Ok(events);
            }

            match self.store.append_batch(&persistent).await {
                Ok(()) => return Ok(events),
                Err(
                    EventStoreError::DuplicateSeq(_) | EventStoreError::DuplicateSessionSeq { .. },
                ) if attempts < MAX_DUPLICATE_SEQ_RETRIES => {
                    attempts += 1;
                    self.resume_from_store().await?;
                    events = self.resequence_events_from_store(events).await?;
                }
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Persist + broadcast a single event.
    pub async fn publish(
        &self,
        ctx: PublishContext,
        kind: K,
    ) -> Result<DomainEvent<K>, PublishError> {
        let event = self.build(ctx, kind).await?;
        self.publish_built(event).await
    }

    pub async fn publish_built(
        &self,
        event: DomainEvent<K>,
    ) -> Result<DomainEvent<K>, PublishError> {
        let mut events = self.persist_with_retry(vec![event]).await?;
        let event = events
            .pop()
            .expect("persist_with_retry should preserve single-event batches");
        self.bus.publish(event.clone()).await?;
        Ok(event)
    }

    /// Persist + broadcast a batch atomically (with respect to the store).
    /// Non-persistent kinds are forwarded to the bus only.
    pub async fn publish_batch(
        &self,
        events: Vec<DomainEvent<K>>,
    ) -> Result<Vec<DomainEvent<K>>, PublishError> {
        let events = self.persist_with_retry(events).await?;
        for event in &events {
            self.bus.publish(event.clone()).await?;
        }
        Ok(events)
    }

    /// Persist a batch without broadcasting to the bus. Used by replay-only
    /// flows (session fork, JSONL import) where the events are historical
    /// reconstructions, not live activity that subscribers should react to.
    pub async fn append_batch_silent(
        &self,
        events: Vec<DomainEvent<K>>,
    ) -> Result<Vec<DomainEvent<K>>, PublishError> {
        self.persist_with_retry(events).await
    }

    /// Re-initialise the sequence allocator from the store's high watermark.
    /// Call this once at startup, before any events are produced.
    pub async fn resume_from_store(&self) -> Result<(), EventStoreError> {
        if let Some(hw) = self.store.high_watermark().await? {
            self.seq.init_from(hw);
        }
        Ok(())
    }
}

impl<K> Clone for EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    fn clone(&self) -> Self {
        Self {
            seq: self.seq.clone(),
            store: self.store.clone(),
            bus: self.bus.clone(),
            session_sequences: Arc::clone(&self.session_sequences),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, InProcessEventBus};
    use agena_domain::ExecutionId;
    use agena_domain::ExecutionSource;
    use agena_domain::ExecutionStartedEvent;
    use agena_storage::WorkspaceRepository;
    use sea_orm::Database;

    async fn test_publisher() -> (
        Arc<EventPublisher<EventKind>>,
        Arc<dyn agena_storage::EventStore<EventKind>>,
        i64,
    ) {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/workspace")
            .await
            .expect("workspace");
        let session =
            crate::db::crud::session::create_session(&db, workspace_id, None, "publisher test")
                .await
                .expect("session");
        let store: Arc<dyn agena_storage::EventStore<EventKind>> = Arc::new(
            agena_storage_sqlite::SeaEventStore::<EventKind>::new(Arc::new(db)),
        );
        let bus: Arc<dyn EventBus<EventKind>> = Arc::new(InProcessEventBus::new(8));
        (
            Arc::new(EventPublisher::new(
                Arc::new(SequenceAllocator::new()),
                Arc::clone(&store),
                bus,
            )),
            store,
            session.id,
        )
    }

    fn execution_started(session_id: i64) -> EventKind {
        EventKind::ExecutionStarted(ExecutionStartedEvent {
            session_id,
            execution_id: ExecutionId::new(),
            turn_id: agena_domain::TurnId::new(),
            reply_id: agena_domain::AssistantReplyId::new(),
            source: ExecutionSource::User,
            ts_ms: 1,
        })
    }

    #[tokio::test]
    async fn concurrent_builds_allocate_unique_ordered_session_sequences() {
        let (publisher, _, session_id) = test_publisher().await;
        let (first, second) = tokio::join!(
            publisher.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
            publisher.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
        );
        let mut sequences = [
            first.expect("first event").meta.seq_session,
            second.expect("second event").meta.seq_session,
        ];
        sequences.sort();
        assert_eq!(sequences, [Some(1), Some(2)]);
    }

    #[tokio::test]
    async fn duplicate_session_seq_conflict_is_retried_and_resequenced() {
        let (_, store, session_id) = test_publisher().await;
        let bus: Arc<dyn EventBus<EventKind>> = Arc::new(InProcessEventBus::new(8));

        // Two independent publishers share one store. The second publisher
        // builds its batch before the first persists, so both allocate
        // `seq_session = 1`. When the second batch is appended after the
        // first, the store reports a duplicate session sequence and the
        // publisher must resequence the whole batch instead of failing.
        let publisher_a = Arc::new(EventPublisher::new(
            Arc::new(SequenceAllocator::new()),
            Arc::clone(&store),
            Arc::clone(&bus),
        ));
        let publisher_b = Arc::new(EventPublisher::new(
            Arc::new(SequenceAllocator::new()),
            Arc::clone(&store),
            Arc::clone(&bus),
        ));

        let built_b = publisher_b
            .build(
                PublishContext::for_session(session_id),
                execution_started(session_id),
            )
            .await
            .expect("publisher b build");
        assert_eq!(built_b.meta.seq_session, Some(1));
        let built_a = publisher_a
            .build(
                PublishContext::for_session(session_id),
                execution_started(session_id),
            )
            .await
            .expect("publisher a build");
        assert_eq!(built_a.meta.seq_session, Some(1));

        publisher_a
            .append_batch_silent(vec![built_a])
            .await
            .expect("publisher a persists first");
        let persisted_b = publisher_b
            .append_batch_silent(vec![built_b])
            .await
            .expect("publisher b retries and resequences instead of failing");
        let persisted_b = persisted_b.into_iter().next().expect("one event returned");
        assert_eq!(persisted_b.meta.seq_session, Some(2));

        // Both events are durable and the per-session sequence is unique.
        let rows = store
            .range(
                &agena_domain::EventFilter {
                    scope: agena_domain::EventScope::Session { session_id },
                    kinds: None,
                    since_seq_global: None,
                },
                agena_storage::StoreRange {
                    after_seq_global: 0,
                    limit: 100,
                },
            )
            .await
            .expect("read session events");
        let mut sequences = rows
            .iter()
            .map(|event| event.meta.seq_session.expect("session seq"))
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        assert_eq!(sequences, [1, 2]);
    }
}
