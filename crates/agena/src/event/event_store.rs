use async_trait::async_trait;

use crate::event::envelope::DomainEvent;
use crate::event::error::EventStoreError;
use crate::event::filter::{EventFilter, KindMatcher};

/// Inclusive lower bound, exclusive upper bound by `seq_global`.
#[derive(Debug, Clone, Copy)]
pub struct StoreRange {
    pub after_seq_global: i64,
    pub limit: usize,
}

/// Persistent event log abstraction.
///
/// Implementations must:
/// - Append events in `seq_global` order (strict monotonic).
/// - Reject duplicate `seq_global` with [`EventStoreError::DuplicateSeq`].
/// - Provide a stable cursor over `seq_global`.
#[async_trait]
pub trait EventStore<K>: Send + Sync
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    /// Append a batch atomically. Either all events land or none do.
    async fn append_batch(&self, events: &[DomainEvent<K>]) -> Result<(), EventStoreError>;

    /// Read events matching `filter`, after `range.after_seq_global`, up to
    /// `range.limit`. Results are returned in ascending `seq_global` order.
    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<DomainEvent<K>>, EventStoreError>;

    /// Highest `seq_global` currently persisted, or `None` if empty.
    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError>;

    /// Highest `seq_session` for a given session, or `None`.
    async fn session_high_watermark(&self, session_id: i64)
    -> Result<Option<i64>, EventStoreError>;
}
