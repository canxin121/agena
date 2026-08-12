//! SQLite schema lifecycle markers.
//!
//! Agena's schema is versioned with `PRAGMA user_version`. Version zero means
//! "not yet created"; a fresh database is created in one DDL transaction by
//! `crates/agena-storage-sqlite/src/schema.rs`. Older incompatible schemas are
//! rejected rather than migrated, so this module is deliberately minimal.

/// Current SQLite schema version written to `PRAGMA user_version`.
///
/// Version 8 adds a stored generated `is_subagent` column to `agena_sessions`
/// (derived from `relation_kind = 'subagent'`), so any session row reports in
/// O(1) whether it is a task child — the session switcher filters on it to
/// show only parent sessions. This schema owns the nine chat tables —
/// `agena_parts`, `agena_session_parts`, `agena_sessions`,
/// `agena_execution_leases`, `agena_sequences`, `agena_workspaces`,
/// `agena_permission_rules`, `agena_usage`, `agena_idempotency` — plus the
/// model-catalog infrastructure tables. Parts are the only chat-content
/// entity; runs are `kind='run'` marker parts; session state is derived from
/// parts + leases; there is no event log, no projection watermark, and no
/// runtime-state JSON. v1 databases are NOT migrated: they are discarded
/// (decision D3), so a database at any version other than 0 or 8 is rejected
/// outright.
///
/// Version history:
/// - 5: the v2 "everything is a part" schema (design
///   `docs/database-design-v2.md`).
/// - 6: `agena_scheduler_jobs` gains `retry_at_ms`, `paused`, `completed`
///   columns so the scheduler due scan filters in SQL (no full-table JSON
///   decode every tick).
/// - 7: scheduler tables move out of this database entirely. The scheduler
///   now owns a dedicated SQLite database with its own schema and version
///   (`agena-scheduler::schema`), so this database no longer holds
///   `agena_scheduler_jobs` / `agena_scheduler_history`.
/// - 8: `agena_sessions` gains the stored generated `is_subagent` column for
///   O(1) task-child detection used by the `/session` switcher filter.
pub const CURRENT_SCHEMA_VERSION: i64 = 8;

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    use super::*;
    use crate::initialize_schema;

    async fn database_with_version(version: i64) -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("PRAGMA user_version = {version}"),
        ))
        .await
        .expect("set schema version");
        db
    }

    async fn read_schema_version(db: &sea_orm::DatabaseConnection) -> i64 {
        db.query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA user_version".to_owned(),
        ))
        .await
        .expect("query user_version")
        .expect("user_version row")
        .try_get("", "user_version")
        .expect("user_version value")
    }

    #[tokio::test]
    async fn fresh_database_is_initialized_at_current_version() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        initialize_schema(&db)
            .await
            .expect("initialize fresh schema");
        assert_eq!(read_schema_version(&db).await, CURRENT_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn current_database_is_accepted_without_rewriting_version() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION).await;
        initialize_schema(&db)
            .await
            .expect("initialize current schema");
        assert_eq!(read_schema_version(&db).await, CURRENT_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn incompatible_older_database_is_rejected_without_mutation() {
        // Simulate a pre-refactor (legacy) schema version.
        let db = database_with_version(11).await;
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE legacy_marker (value TEXT NOT NULL)".to_owned(),
        ))
        .await
        .expect("create legacy marker table");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO legacy_marker (value) VALUES ('preserved')".to_owned(),
        ))
        .await
        .expect("insert legacy marker");

        let error = initialize_schema(&db)
            .await
            .expect_err("reject legacy schema");

        assert!(error.to_string().contains("does not migrate"));
        assert_eq!(read_schema_version(&db).await, 11);
        let marker = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT value FROM legacy_marker".to_owned(),
            ))
            .await
            .expect("read marker")
            .expect("marker row");
        assert_eq!(
            marker.try_get::<String>("", "value").expect("marker value"),
            "preserved"
        );
    }

    #[tokio::test]
    async fn newer_database_is_rejected() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION + 1).await;
        let error = initialize_schema(&db)
            .await
            .expect_err("reject newer schema");
        assert!(error.to_string().contains("does not migrate"));
    }
}
