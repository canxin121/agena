//! SQLite schema lifecycle markers.
//!
//! Agena's schema is versioned with `PRAGMA user_version`. Version zero means
//! "not yet created"; a fresh database is created in one DDL transaction by
//! `crates/agena-storage-sqlite/src/schema.rs`. Older incompatible schemas are
//! rejected rather than migrated, so this module is deliberately minimal.

/// Current SQLite schema version written to `PRAGMA user_version`.
///
/// Version 2 adds the `agena_content_nodes.title` column for tool Activities,
/// letting a running title be updated with a tiny column UPDATE instead of a
/// full payload rewrite. Version 1 introduced the database-backed
/// `agena_sequences` / `agena_session_sequences` tables, the cross-process
/// `agena_execution_leases` table, the user-message
/// `agena_user_message_idempotency` table, and the scheduler `delivery_key` /
/// `claimed_at_ms` columns. The schema evolves in place (create-if-not-exists);
/// older databases are rejected rather than migrated.
/// Version 3 adds the `agena_session_messages` membership table: a fork (or
/// rewind branch) references the parent's terminal message rows instead of
/// physically copying them, so `/fork` and `/side` stay cheap and compact.
pub const CURRENT_SCHEMA_VERSION: i64 = 3;

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
