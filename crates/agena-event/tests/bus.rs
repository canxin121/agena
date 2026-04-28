//! In-memory `EventStore` impl + integration tests covering publish/subscribe,
//! filtering, lagged subscribers, and store-backed resume.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use agena_event::{
    DomainEvent, EventBus, EventFilter, EventKindTag, EventPublisher, EventStore, EventStoreError,
    InProcessEventBus, KindMatcher, Scope, SequenceAllocator, StoreRange,
    bus::SubscriptionItem, publisher::PublishContext,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum TestKind {
    Hello { text: String },
    World { value: i64 },
}

impl KindMatcher for TestKind {
    fn tag(&self) -> EventKindTag {
        match self {
            TestKind::Hello { .. } => "hello".into(),
            TestKind::World { .. } => "world".into(),
        }
    }
}

#[derive(Default)]
struct MemStore {
    events: Mutex<Vec<DomainEvent<TestKind>>>,
}

#[async_trait]
impl EventStore<TestKind> for MemStore {
    async fn append_batch(
        &self,
        events: &[DomainEvent<TestKind>],
    ) -> Result<(), EventStoreError> {
        let mut guard = self.events.lock().await;
        for ev in events {
            if guard.iter().any(|e| e.meta.seq_global == ev.meta.seq_global) {
                return Err(EventStoreError::DuplicateSeq(ev.meta.seq_global));
            }
            guard.push(ev.clone());
        }
        guard.sort_by_key(|e| e.meta.seq_global);
        Ok(())
    }

    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<DomainEvent<TestKind>>, EventStoreError> {
        let guard = self.events.lock().await;
        let mut out: Vec<_> = guard
            .iter()
            .filter(|e| e.meta.seq_global > range.after_seq_global)
            .filter(|e| filter.scope.matches(&e.meta))
            .filter(|e| filter.matches_kind(&e.kind.tag()))
            .cloned()
            .collect();
        out.truncate(range.limit);
        Ok(out)
    }

    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError> {
        Ok(self.events.lock().await.iter().map(|e| e.meta.seq_global).max())
    }

    async fn session_high_watermark(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, EventStoreError> {
        Ok(self
            .events
            .lock()
            .await
            .iter()
            .filter(|e| e.meta.session_id == Some(session_id))
            .filter_map(|e| e.meta.seq_session)
            .max())
    }
}

fn make_publisher(
    capacity: usize,
) -> (
    EventPublisher<TestKind>,
    Arc<MemStore>,
    Arc<InProcessEventBus<TestKind>>,
) {
    let store = Arc::new(MemStore::default());
    let bus = Arc::new(InProcessEventBus::<TestKind>::new(capacity));
    let seq = Arc::new(SequenceAllocator::new());
    let publisher = EventPublisher::new(seq, store.clone(), bus.clone());
    (publisher, store, bus)
}

#[tokio::test]
async fn publish_then_subscribe_round_trip() {
    let (publisher, _store, bus) = make_publisher(16);
    let mut sub = bus.subscribe(EventFilter::new(Scope::Global));

    let task = tokio::spawn(async move {
        match sub.recv().await.unwrap() {
            SubscriptionItem::Event(ev) => ev,
            other => panic!("unexpected: {other:?}"),
        }
    });

    let ctx = PublishContext::for_session(1);
    let event = publisher
        .publish(ctx, TestKind::Hello { text: "hi".into() })
        .await
        .unwrap();
    let received = task.await.unwrap();
    assert_eq!(received.meta.seq_global, event.meta.seq_global);
    assert_eq!(received.meta.session_id, Some(1));
}

#[tokio::test]
async fn filter_drops_off_scope_and_off_kind() {
    let (publisher, _store, bus) = make_publisher(16);
    let mut sub = bus.subscribe(
        EventFilter::new(Scope::Session { session_id: 7 }).with_kinds(["hello"]),
    );

    publisher
        .publish(
            PublishContext::for_session(99),
            TestKind::Hello { text: "wrong scope".into() },
        )
        .await
        .unwrap();
    publisher
        .publish(
            PublishContext::for_session(7),
            TestKind::World { value: 1 },
        )
        .await
        .unwrap();
    let kept = publisher
        .publish(
            PublishContext::for_session(7),
            TestKind::Hello { text: "kept".into() },
        )
        .await
        .unwrap();

    let item = sub.recv().await.unwrap();
    let SubscriptionItem::Event(ev) = item else {
        panic!("expected event");
    };
    assert_eq!(ev.meta.seq_global, kept.meta.seq_global);
}

#[tokio::test]
async fn lagged_subscriber_is_notified() {
    let (publisher, _store, bus) = make_publisher(2);
    let mut sub = bus.subscribe(EventFilter::new(Scope::Global));

    for i in 0..10 {
        publisher
            .publish(PublishContext::default(), TestKind::World { value: i })
            .await
            .unwrap();
    }

    let item = sub.recv().await.unwrap();
    matches!(item, SubscriptionItem::Lagged(_));
}

#[tokio::test]
async fn resume_from_store_restores_sequence() {
    let (publisher, store, _bus) = make_publisher(4);
    for _ in 0..5 {
        publisher
            .publish(PublishContext::default(), TestKind::World { value: 1 })
            .await
            .unwrap();
    }
    assert_eq!(store.high_watermark().await.unwrap(), Some(5));

    // Simulate restart: new allocator, same store.
    let new_bus = Arc::new(InProcessEventBus::<TestKind>::new(4));
    let new_seq = Arc::new(SequenceAllocator::new());
    let new_pub = EventPublisher::new(new_seq, store.clone(), new_bus);
    new_pub.resume_from_store().await.unwrap();

    let next = new_pub
        .publish(PublishContext::default(), TestKind::World { value: 2 })
        .await
        .unwrap();
    assert_eq!(next.meta.seq_global, 6);
}

#[tokio::test]
async fn store_range_returns_after_cursor_in_order() {
    let (publisher, store, _bus) = make_publisher(8);
    for i in 0..6 {
        publisher
            .publish(
                PublishContext::for_session(1),
                TestKind::World { value: i },
            )
            .await
            .unwrap();
    }

    let filter = EventFilter::new(Scope::Session { session_id: 1 });
    let range = StoreRange {
        after_seq_global: 2,
        limit: 10,
    };
    let out = store.range(&filter, range).await.unwrap();
    let seqs: Vec<_> = out.iter().map(|e| e.meta.seq_global).collect();
    assert_eq!(seqs, vec![3, 4, 5, 6]);
}
