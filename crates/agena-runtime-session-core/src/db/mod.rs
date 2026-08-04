pub mod crud;
pub mod entities;
#[cfg(test)]
pub mod event_entity;
pub mod leases;

#[cfg(test)]
mod usage_adapter_tests {
    use std::sync::Arc;

    use agena_storage::UsageRepository;
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    use agena_storage_sqlite::initialize_schema;

    #[tokio::test]
    async fn sqlite_usage_adapter_reads_the_real_assistant_role_encoding() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        initialize_schema(&db).await.expect("schema");
        for sql in [
            "INSERT INTO agena_workspaces (path, created_at_ms, updated_at_ms) VALUES ('/usage', 1, 1)",
            "INSERT INTO agena_sessions (parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, creation_failure_json, runtime_state_json, created_at_ms, updated_at_ms) VALUES (NULL, 0, 0, 1, 'usage session', 1, 'ready', NULL, '{}', 1, 1)",
            "INSERT INTO agena_model_messages (message_id, session_id, model_turn_id, execution_id, run_id, role, state, created_at_ms, updated_at_ms, metadata, provider_state, usage, part_count) VALUES (101, 1, NULL, NULL, NULL, 2, 3, 10, 10, '{\"model_provider_id\":\"provider\",\"model_id\":\"model\"}', NULL, '{\"input_tokens\":1,\"output_tokens\":2,\"reasoning_tokens\":3,\"cache_write_tokens\":4,\"cache_read_tokens\":5,\"total_cost\":0.1}', 0)",
        ] {
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                sql.to_owned(),
            ))
            .await
            .expect("fixture row");
        }
        let repository = agena_storage_sqlite::SeaUsageRepository::new(Arc::new(db));
        let records = repository
            .list(1, &[], false, None, None)
            .await
            .expect("usage records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider_id, "provider");
        assert_eq!(records[0].usage.value["output_tokens"], 2);
    }
}

#[cfg(test)]
mod session_summary_adapter_tests {
    use std::sync::Arc;

    use agena_storage::{
        SessionMutationRepository, SessionSummaryListQuery, SessionSummaryRepository,
    };
    use sea_orm::{ConnectionTrait, Database};

    use agena_storage_sqlite::initialize_schema;

    #[tokio::test]
    async fn sqlite_summary_adapter_preserves_root_child_and_simple_mutation_semantics() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        initialize_schema(&db).await.expect("schema");
        db.execute_unprepared(
            "INSERT INTO agena_workspaces (path, created_at_ms, updated_at_ms) VALUES ('/summary', 1, 1)",
        )
        .await
        .expect("workspace");
        let repository = agena_storage_sqlite::SeaSessionSummaryRepository::new(Arc::new(db));
        let root = repository
            .create(1, None, "root".to_owned())
            .await
            .expect("root");
        assert_eq!(root.root_id, root.id);
        let child = repository
            .create(1, Some(root.id), "child".to_owned())
            .await
            .expect("child");
        assert_eq!(child.depth, 1);
        assert_eq!(child.root_id, root.id);
        assert_eq!(repository.list_tree(root.id).await.expect("tree").len(), 2);
        let renamed = repository
            .rename(child.id, "renamed".to_owned())
            .await
            .expect("rename")
            .expect("existing child");
        assert_eq!(renamed.title, "renamed");
        assert_eq!(
            repository
                .list(SessionSummaryListQuery {
                    workspace_id: Some(1),
                    roots_only: true,
                    include_subagents: true,
                    limit: 10,
                    ..Default::default()
                })
                .await
                .expect("list roots")
                .len(),
            1
        );
        assert_eq!(repository.delete(root.id).await.expect("delete root"), 1);
        assert!(repository.get(child.id).await.expect("get child").is_none());
    }
}

#[cfg(test)]
mod event_store_adapter_tests {
    use std::sync::Arc;

    use agena_domain::{EventEnvelope, EventFilter, EventMeta, KindMatcher};
    use agena_storage::{EventStore, StoreRange};
    use chrono::Utc;
    use sea_orm::Database;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use agena_storage_sqlite::initialize_schema;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct TestEvent {
        value: String,
    }
    impl KindMatcher for TestEvent {
        fn tag(&self) -> agena_domain::EventKindTag {
            "storage_test".into()
        }
    }

    #[tokio::test]
    async fn sqlite_event_store_round_trips_envelopes_and_empty_watermarks() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        initialize_schema(&db).await.expect("schema");
        let store = agena_storage_sqlite::SeaEventStore::<TestEvent>::new(Arc::new(db));
        assert_eq!(store.high_watermark().await.expect("empty watermark"), None);
        let event = EventEnvelope {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: 1,
                seq_session: None,
                session_id: None,
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            kind: TestEvent {
                value: "payload".to_owned(),
            },
        };
        store
            .append_batch(std::slice::from_ref(&event))
            .await
            .expect("append");
        assert_eq!(store.high_watermark().await.expect("watermark"), Some(1));
        let returned = store
            .range(
                &EventFilter::new(agena_domain::EventScope::Global),
                StoreRange {
                    after_seq_global: 0,
                    limit: 10,
                },
            )
            .await
            .expect("range");
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].meta.id, event.meta.id);
        assert_eq!(returned[0].kind, event.kind);
        assert_eq!(
            returned[0].meta.created_at.timestamp_millis(),
            event.meta.created_at.timestamp_millis()
        );
    }
}

#[cfg(test)]
mod lease_tests {
    use std::sync::Arc;

    use agena_storage::WorkspaceRepository;
    use sea_orm::{ConnectionTrait, Database};

    use crate::db::leases::{
        LeaseAcquireOutcome, lease_now_ms, reap_stale_leases, release_lease, try_acquire_lease,
    };
    use agena_storage_sqlite::initialize_schema;

    async fn session_db() -> (sea_orm::DatabaseConnection, i64) {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        initialize_schema(&db).await.expect("schema");
        // Workspace + session for the lease FK.
        let ws = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/lease-test")
            .await
            .expect("workspace");
        let row = db
            .query_one(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT INTO agena_sessions (parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, created_at_ms, updated_at_ms) \
                 VALUES (NULL, 0, 0, ?, 'lease', 1, 'ready', 1, 1) RETURNING id",
                [ws.into()],
            ))
            .await
            .expect("insert session")
            .expect("session row");
        let session_id: i64 = row.try_get("", "id").expect("session id");
        (db, session_id)
    }

    #[tokio::test]
    async fn lease_is_exclusive_across_owners_and_releasable() {
        let (db, session_id) = session_db().await;
        let now = lease_now_ms();

        let first = try_acquire_lease(&db, session_id, "owner-a", None, now)
            .await
            .expect("acquire a");
        assert!(matches!(first, LeaseAcquireOutcome::Acquired));

        let second = try_acquire_lease(&db, session_id, "owner-b", None, now + 1)
            .await
            .expect("acquire b");
        assert!(matches!(second, LeaseAcquireOutcome::HeldBy { .. }));

        assert!(
            release_lease(&db, session_id, "owner-a")
                .await
                .expect("release a")
        );
        let third = try_acquire_lease(&db, session_id, "owner-b", None, now + 2)
            .await
            .expect("acquire b again");
        assert!(matches!(third, LeaseAcquireOutcome::Acquired));
    }

    #[tokio::test]
    async fn stale_leases_are_reclaimed() {
        let (db, session_id) = session_db().await;
        let now = lease_now_ms();
        try_acquire_lease(&db, session_id, "owner-crashed", None, now - 60_000)
            .await
            .expect("acquire stale");

        let reclaimed = reap_stale_leases(&db, now - 30_000).await.expect("reap");
        assert!(reclaimed.contains(&session_id));

        // After reclaim the lease is gone and a new owner can acquire.
        let fresh = try_acquire_lease(&db, session_id, "owner-new", None, now)
            .await
            .expect("acquire after reap");
        assert!(matches!(fresh, LeaseAcquireOutcome::Acquired));
    }

    #[tokio::test]
    async fn stale_lease_is_stolen_atomically_at_acquire_time() {
        let (db, session_id) = session_db().await;
        let now = lease_now_ms();
        // A crashed owner's lease whose heartbeat is well past the threshold.
        try_acquire_lease(&db, session_id, "owner-crashed", None, now - 60_000)
            .await
            .expect("acquire stale");

        // A new owner takes over immediately — no separate reap step needed.
        let taken = try_acquire_lease(&db, session_id, "owner-new", None, now)
            .await
            .expect("steal stale lease");
        assert!(matches!(taken, LeaseAcquireOutcome::Acquired));

        // The new owner now exclusively holds a fresh lease.
        let held = try_acquire_lease(&db, session_id, "owner-third", None, now + 1)
            .await
            .expect("owner-third attempts");
        assert!(matches!(held, LeaseAcquireOutcome::HeldBy { .. }));
        let row = crate::db::leases::lease(&db, session_id)
            .await
            .expect("read lease")
            .expect("lease row");
        assert_eq!(row.owner_id, "owner-new");
    }

    #[tokio::test]
    async fn fresh_lease_is_never_stolen_at_acquire_time() {
        let (db, session_id) = session_db().await;
        let now = lease_now_ms();
        try_acquire_lease(&db, session_id, "owner-live", None, now)
            .await
            .expect("acquire live");

        // A fresh heartbeat must keep the lease with its current owner.
        let held = try_acquire_lease(&db, session_id, "owner-try", None, now + 1)
            .await
            .expect("attempt on fresh lease");
        let LeaseAcquireOutcome::HeldBy { owner_id, .. } = held else {
            panic!("fresh lease must not be stolen");
        };
        assert_eq!(owner_id, "owner-live");
    }
}
