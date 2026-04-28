use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::envelope::DomainEvent;
use crate::error::BusError;
use crate::filter::{EventFilter, KindMatcher};

/// Item delivered to a subscriber. `Lagged(n)` means the underlying broadcast
/// channel dropped `n` events because this subscriber was too slow; transports
/// should surface this and may resume from the store using
/// `EventStore::range`.
#[derive(Debug, Clone)]
pub enum SubscriptionItem<K> {
    Event(Arc<DomainEvent<K>>),
    Lagged(u64),
}

/// Live subscription handle. Subscribers drive it by calling `recv` in a
/// loop; dropping the handle unsubscribes.
pub struct Subscription<K> {
    rx: broadcast::Receiver<Arc<DomainEvent<K>>>,
    filter: EventFilter,
}

impl<K> Subscription<K>
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    pub fn filter(&self) -> &EventFilter {
        &self.filter
    }

    /// Receive the next item that matches the filter. Returns `None` when the
    /// bus is closed.
    pub async fn recv(&mut self) -> Option<SubscriptionItem<K>> {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if !self.filter.matches_meta(&event.meta) {
                        continue;
                    }
                    let tag = event.kind.tag();
                    if !self.filter.matches_kind(&tag) {
                        continue;
                    }
                    return Some(SubscriptionItem::Event(event));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    return Some(SubscriptionItem::Lagged(n));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Pub/sub abstraction. Production uses [`InProcessEventBus`]; tests can
/// substitute custom implementations.
#[async_trait]
pub trait EventBus<K>: Send + Sync
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    async fn publish(&self, event: DomainEvent<K>) -> Result<(), BusError>;
    fn subscribe(&self, filter: EventFilter) -> Subscription<K>;
    /// Capacity of the underlying broadcast channel; useful for transports
    /// that want to estimate when to fall back to store replay.
    fn capacity(&self) -> usize;
}

/// In-process tokio broadcast bus. Storage is the caller's responsibility:
/// the recommended pattern is to compose this with [`crate::EventStore`] via
/// [`crate::EventPublisher`].
pub struct InProcessEventBus<K> {
    tx: broadcast::Sender<Arc<DomainEvent<K>>>,
    capacity: usize,
}

impl<K> InProcessEventBus<K>
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx, capacity }
    }
}

#[async_trait]
impl<K> EventBus<K> for InProcessEventBus<K>
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    async fn publish(&self, event: DomainEvent<K>) -> Result<(), BusError> {
        // `send` only errors when there are no receivers — that is not a
        // failure for fire-and-forget event flow, so we ignore the count.
        let _ = self.tx.send(Arc::new(event));
        Ok(())
    }

    fn subscribe(&self, filter: EventFilter) -> Subscription<K> {
        let rx = self.tx.subscribe();
        Subscription { rx, filter }
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}

// Re-export so transports can convert `Subscription` into a Stream if they
// prefer.
pub use tokio_stream::wrappers::BroadcastStream;
pub use tokio_stream::Stream;

/// Helper: classify a `BroadcastStreamRecvError` for transport layers that
/// build their own streams.
pub fn classify_recv_error(err: BroadcastStreamRecvError) -> u64 {
    match err {
        BroadcastStreamRecvError::Lagged(n) => n,
    }
}
