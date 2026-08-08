use std::sync::Arc;

use chrono::Utc;
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
    seq: Arc<dyn SequenceAllocator>,
    store: Arc<dyn agena_storage::EventStore<K>>,
    bus: Arc<dyn EventBus<K>>,
}

impl<K> EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    pub fn new(
        seq: Arc<dyn SequenceAllocator>,
        store: Arc<dyn agena_storage::EventStore<K>>,
        bus: Arc<dyn EventBus<K>>,
    ) -> Self {
        Self { seq, store, bus }
    }

    pub fn store(&self) -> &Arc<dyn agena_storage::EventStore<K>> {
        &self.store
    }

    pub fn bus(&self) -> &Arc<dyn EventBus<K>> {
        &self.bus
    }

    /// Build an envelope with both globally and per-session monotonic
    /// sequences, allocated atomically by the backing store so concurrent
    /// processes can never produce a duplicate.
    pub async fn build(
        &self,
        ctx: PublishContext,
        kind: K,
    ) -> Result<DomainEvent<K>, EventStoreError> {
        let seq_session = match ctx.session_id {
            Some(session_id) => Some(self.seq.next_seq_session(session_id).await?),
            None => None,
        };
        Ok(DomainEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: self.seq.next_seq_global().await?,
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

    /// Reallocate both the global and the per-session sequence numbers of a
    /// batch after a duplicate-sequence conflict. With the database-backed
    /// allocator this is normally a no-op path: each allocation is atomic and
    /// unique, so a batch can only collide if it carried hand-built sequences
    /// (fork copy, JSONL import). Re-allocating through the same store keeps
    /// the retry loop a safe fallback.
    async fn resequence_events_from_store(
        &self,
        events: Vec<DomainEvent<K>>,
    ) -> Result<Vec<DomainEvent<K>>, EventStoreError> {
        let mut out = Vec::with_capacity(events.len());
        for mut event in events {
            event.meta.seq_global = self.seq.next_seq_global().await?;
            if let Some(session_id) = event.meta.session_id {
                event.meta.seq_session = Some(self.seq.next_seq_session(session_id).await?);
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

    /// Raise the sequence allocator floor to the store's high watermark.
    /// Call this once at startup, before any events are produced. Idempotent:
    /// the database-backed allocator seeds to `MAX(current, watermark + 1)`.
    pub async fn resume_from_store(&self) -> Result<(), EventStoreError> {
        let hw = self.store.high_watermark().await?.unwrap_or(0);
        self.seq.seed_global(hw).await
    }
}

impl<K> Clone for EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    fn clone(&self) -> Self {
        Self {
            seq: Arc::clone(&self.seq),
            store: Arc::clone(&self.store),
            bus: Arc::clone(&self.bus),
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
    use sea_orm::{Database, DatabaseConnection};

    async fn test_publisher() -> (
        Arc<EventPublisher<EventKind>>,
        Arc<dyn agena_storage::EventStore<EventKind>>,
        Arc<DatabaseConnection>,
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
        let db_arc = Arc::new(db);
        let store: Arc<dyn agena_storage::EventStore<EventKind>> = Arc::new(
            agena_storage_sqlite::SeaEventStore::<EventKind>::new(Arc::clone(&db_arc)),
        );
        let bus: Arc<dyn EventBus<EventKind>> = Arc::new(InProcessEventBus::new(8));
        let seq: Arc<dyn SequenceAllocator> = Arc::new(
            agena_storage_sqlite::SqliteSequenceAllocator::new(Arc::clone(&db_arc)),
        );
        (
            Arc::new(EventPublisher::new(seq, Arc::clone(&store), bus)),
            store,
            db_arc,
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
        let (publisher, _, _, session_id) = test_publisher().await;
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
    async fn independent_publishers_never_allocate_duplicate_session_sequences() {
        let (_, store, db_arc, session_id) = test_publisher().await;
        let bus: Arc<dyn EventBus<EventKind>> = Arc::new(InProcessEventBus::new(8));

        // Two independent publishers share one store but hold separate
        // database-backed allocator instances. The database allocates each
        // `seq_session` atomically, so neither publisher can collide with the
        // other even though both start from a fresh allocator.
        let publisher_a = Arc::new(EventPublisher::new(
            Arc::new(agena_storage_sqlite::SqliteSequenceAllocator::new(
                Arc::clone(&db_arc),
            )),
            Arc::clone(&store),
            Arc::clone(&bus),
        ));
        let publisher_b = Arc::new(EventPublisher::new(
            Arc::new(agena_storage_sqlite::SqliteSequenceAllocator::new(
                Arc::clone(&db_arc),
            )),
            Arc::clone(&store),
            Arc::clone(&bus),
        ));

        // Allocate four session sequences across both publishers. All four
        // must be distinct even though each allocator is independent.
        let (a1, b1) = tokio::join!(
            publisher_a.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
            publisher_b.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
        );
        let (a2, b2) = tokio::join!(
            publisher_a.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
            publisher_b.build(
                PublishContext::for_session(session_id),
                execution_started(session_id)
            ),
        );
        let built = [a1, b1, a2, b2]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("all builds succeed");
        let mut sequences = built
            .iter()
            .map(|event| event.meta.seq_session.expect("session seq"))
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        assert_eq!(sequences, [1, 2, 3, 4]);

        // Every event persists without any duplicate-sequence conflict.
        publisher_a
            .append_batch_silent(built)
            .await
            .expect("persist all events");

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
        assert_eq!(sequences, [1, 2, 3, 4]);
    }
}
