use std::sync::Arc;

use agena_event::{
    DomainEvent, EventFilter, EventKindTag, EventStore, KindMatcher, Scope, StoreRange,
    envelope::{ENVELOPE_SCHEMA_VERSION, EventMeta},
};
use agena_event_store_sea::{Migrator, SeaEventStore};
use chrono::Utc;
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum DemoKind {
    Hello { text: String },
    World { value: i64 },
}

impl KindMatcher for DemoKind {
    fn tag(&self) -> EventKindTag {
        match self {
            DemoKind::Hello { .. } => "hello".into(),
            DemoKind::World { .. } => "world".into(),
        }
    }
}

async fn setup_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory");
    Migrator::up(&db, None).await.expect("migrate up");
    db
}

fn evt(seq: i64, session_id: Option<i64>, kind: DemoKind) -> DomainEvent<DemoKind> {
    DomainEvent {
        meta: EventMeta {
            id: Uuid::new_v4(),
            seq_global: seq,
            seq_session: Some(seq),
            session_id,
            workspace_id: session_id.map(|s| s * 10),
            created_at: Utc::now(),
            causation_id: None,
            correlation_id: None,
            envelope_schema: ENVELOPE_SCHEMA_VERSION,
        },
        kind,
    }
}

#[tokio::test]
async fn append_and_high_watermark() {
    let db = Arc::new(setup_db().await);
    let store = SeaEventStore::<DemoKind>::new(db.clone());

    assert_eq!(store.high_watermark().await.unwrap(), None);

    let events = vec![
        evt(1, Some(7), DemoKind::Hello { text: "a".into() }),
        evt(2, Some(7), DemoKind::World { value: 9 }),
        evt(3, Some(8), DemoKind::Hello { text: "c".into() }),
    ];
    store.append_batch(&events).await.unwrap();

    assert_eq!(store.high_watermark().await.unwrap(), Some(3));
    assert_eq!(store.session_high_watermark(7).await.unwrap(), Some(2));
    assert_eq!(store.session_high_watermark(8).await.unwrap(), Some(3));
}

#[tokio::test]
async fn range_filters_by_scope_and_cursor() {
    let db = Arc::new(setup_db().await);
    let store = SeaEventStore::<DemoKind>::new(db.clone());

    let events = vec![
        evt(1, Some(1), DemoKind::Hello { text: "a".into() }),
        evt(2, Some(2), DemoKind::World { value: 1 }),
        evt(3, Some(1), DemoKind::World { value: 2 }),
        evt(4, Some(1), DemoKind::Hello { text: "b".into() }),
        evt(5, Some(2), DemoKind::Hello { text: "c".into() }),
    ];
    store.append_batch(&events).await.unwrap();

    let filter = EventFilter::new(Scope::Session { session_id: 1 });
    let out = store
        .range(
            &filter,
            StoreRange {
                after_seq_global: 1,
                limit: 100,
            },
        )
        .await
        .unwrap();
    let seqs: Vec<_> = out.iter().map(|e| e.meta.seq_global).collect();
    assert_eq!(seqs, vec![3, 4]);
}

#[tokio::test]
async fn range_filters_by_kind() {
    let db = Arc::new(setup_db().await);
    let store = SeaEventStore::<DemoKind>::new(db.clone());

    let events = vec![
        evt(1, Some(1), DemoKind::Hello { text: "a".into() }),
        evt(2, Some(1), DemoKind::World { value: 9 }),
        evt(3, Some(1), DemoKind::Hello { text: "b".into() }),
    ];
    store.append_batch(&events).await.unwrap();

    let filter = EventFilter::new(Scope::Global).with_kinds(["world"]);
    let out = store
        .range(
            &filter,
            StoreRange {
                after_seq_global: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].meta.seq_global, 2);
}

#[tokio::test]
async fn duplicate_seq_global_rejected() {
    let db = Arc::new(setup_db().await);
    let store = SeaEventStore::<DemoKind>::new(db.clone());

    let dup = vec![
        evt(1, Some(1), DemoKind::Hello { text: "a".into() }),
        evt(1, Some(1), DemoKind::Hello { text: "b".into() }),
    ];
    let err = store.append_batch(&dup).await;
    assert!(err.is_err(), "expected duplicate seq_global to fail");

    // The first row should not be inserted because the batch is transactional.
    assert_eq!(store.high_watermark().await.unwrap(), None);
}

#[tokio::test]
async fn migration_drops_legacy_tables_idempotently() {
    let db = Arc::new(setup_db().await);
    // Re-run the migrator on a freshly migrated DB. Should be a no-op (and
    // the `if_exists` drops should not error).
    Migrator::up(db.as_ref(), None).await.unwrap();
    let store = SeaEventStore::<DemoKind>::new(db.clone());
    assert_eq!(store.high_watermark().await.unwrap(), None);
}
