use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::event::bus::EventBus;
use crate::event::envelope::{DomainEvent, ENVELOPE_SCHEMA_VERSION, EventMeta};
use crate::event::error::{EventStoreError, PublishError};
use crate::event::event_store::EventStore;
use crate::event::filter::KindPersistence;
use crate::event::sequence::SequenceAllocator;

/// Optional routing context attached to a publish call. The publisher fills
/// in the rest of the envelope (id, seq, timestamp).
#[derive(Debug, Clone, Default)]
pub struct PublishContext {
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub seq_session: Option<i64>,
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
    store: Arc<dyn EventStore<K>>,
    bus: Arc<dyn EventBus<K>>,
}

impl<K> EventPublisher<K>
where
    K: KindPersistence + Send + Sync + Clone + 'static,
{
    pub fn new(
        seq: Arc<SequenceAllocator>,
        store: Arc<dyn EventStore<K>>,
        bus: Arc<dyn EventBus<K>>,
    ) -> Self {
        Self { seq, store, bus }
    }

    pub fn store(&self) -> &Arc<dyn EventStore<K>> {
        &self.store
    }

    pub fn bus(&self) -> &Arc<dyn EventBus<K>> {
        &self.bus
    }

    pub fn sequence(&self) -> &Arc<SequenceAllocator> {
        &self.seq
    }

    /// Build an envelope without publishing — useful when callers want to
    /// inspect the assigned `seq_global` (e.g. for logging) before sending.
    pub fn build(&self, ctx: PublishContext, kind: K) -> DomainEvent<K> {
        DomainEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: self.seq.next(),
                seq_session: ctx.seq_session,
                session_id: ctx.session_id,
                workspace_id: ctx.workspace_id,
                created_at: Utc::now(),
                causation_id: ctx.causation_id,
                correlation_id: ctx.correlation_id,
                envelope_schema: ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        }
    }

    fn resequence_events(&self, events: &[DomainEvent<K>]) -> Vec<DomainEvent<K>> {
        events
            .iter()
            .cloned()
            .map(|mut event| {
                event.meta.seq_global = self.seq.next();
                event
            })
            .collect()
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
                Err(EventStoreError::DuplicateSeq(_)) if attempts < MAX_DUPLICATE_SEQ_RETRIES => {
                    attempts += 1;
                    self.resume_from_store().await?;
                    events = self.resequence_events(&events);
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
        let event = self.build(ctx, kind);
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
        }
    }
}
