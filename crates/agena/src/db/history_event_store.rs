use std::sync::Arc;

use async_trait::async_trait;

use crate::event::{EventFilter, EventKind, EventStore, EventStoreError, StoreRange};
use crate::event::envelope::DomainEvent;
use crate::db::SeaEventStore;

/// Wraps `SeaEventStore<EventKind>` and silently skips UI-only events (those
/// where `EventKind::is_persistent()` returns `false`). History events are
/// written normally; the bus still receives every event regardless.
pub struct HistoryEventStore {
    inner: SeaEventStore<EventKind>,
}

impl HistoryEventStore {
    pub fn new(db: Arc<sea_orm::DatabaseConnection>) -> Self {
        Self {
            inner: SeaEventStore::new(db),
        }
    }
}

#[async_trait]
impl EventStore<EventKind> for HistoryEventStore {
    async fn append_batch(&self, events: &[DomainEvent<EventKind>]) -> Result<(), EventStoreError> {
        let persistent: Vec<_> = events
            .iter()
            .filter(|e| e.kind.is_persistent())
            .cloned()
            .collect();
        if persistent.is_empty() {
            return Ok(());
        }
        self.inner.append_batch(&persistent).await
    }

    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<DomainEvent<EventKind>>, EventStoreError> {
        self.inner.range(filter, range).await
    }

    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError> {
        self.inner.high_watermark().await
    }

    async fn session_high_watermark(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, EventStoreError> {
        self.inner.session_high_watermark(session_id).await
    }
}
