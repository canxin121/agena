//! Database access layer shared by session persistence.

pub mod crud;
pub mod entities;
pub mod leases;

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
