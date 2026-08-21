//! SQLite schema lifecycle markers.
//!
//! Agena's schema is versioned with `PRAGMA user_version`. Version zero means
//! "not yet created"; a fresh database is created in one DDL transaction by
//! `crates/agena-storage-sqlite/src/schema.rs`. Older incompatible schemas are
//! rejected rather than migrated, so this module is deliberately minimal.

/// Current SQLite schema version written to `PRAGMA user_version`.
///
/// Version 13 is the first schema for the canonical tool-result lifecycle.
/// It removes the durable human-rendering column from parts and requires the
/// current single-source content shape. This schema
/// owns the chat tables —
/// `agena_parts`, `agena_session_parts`, `agena_sessions`,
/// `agena_execution_leases`, `agena_sequences`, `agena_workspaces`,
/// `agena_permission_rules`, `agena_usage`, `agena_idempotency` — plus the
/// model-catalog infrastructure tables. Parts are the only chat-content
/// entity; runs are `kind='run'` marker parts; session state is derived from
/// parts + leases. Background-operation control state is deliberately
/// normalized rather than encoded only in transcript JSON. Incompatible
/// databases are rejected; a new database is created only from the current
/// schema.
///
pub const CURRENT_SCHEMA_VERSION: i64 = 13;

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
        // Simulate an incompatible older schema version.
        let db = database_with_version(7).await;
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE marker (value TEXT NOT NULL)".to_owned(),
        ))
        .await
        .expect("create marker table");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO marker (value) VALUES ('preserved')".to_owned(),
        ))
        .await
        .expect("insert marker");

        let error = initialize_schema(&db)
            .await
            .expect_err("reject incompatible schema");

        assert!(error.to_string().contains("does not migrate"));
        assert_eq!(read_schema_version(&db).await, 7);
        let marker = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT value FROM marker".to_owned(),
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
