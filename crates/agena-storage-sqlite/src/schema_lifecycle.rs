//! SQLite schema lifecycle markers.
//!
//! Agena's schema is versioned with `PRAGMA user_version`. Version zero means
//! "not yet created"; a fresh database is created in one DDL transaction by
//! `crates/agena-storage-sqlite/src/schema.rs`. Older incompatible schemas are
//! rejected rather than migrated. Compatible invariant-trigger corrections
//! retain the table-layout version and are refreshed in one transaction.

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
/// Compatible trigger corrections preserve version 13 and existing rows;
/// initialization verifies the concrete tables and required indexes before
/// refreshing those triggers. No table/column migration is performed.
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
        let db = database_with_version(0).await;
        initialize_schema(&db).await.expect("create current schema");
        let before = schema_objects(&db).await;
        initialize_schema(&db)
            .await
            .expect("initialize current schema");
        assert_eq!(read_schema_version(&db).await, CURRENT_SCHEMA_VERSION);
        assert_eq!(schema_objects(&db).await, before);
    }

    async fn schema_objects(db: &sea_orm::DatabaseConnection) -> Vec<(String, String, String)> {
        db.query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type, name",
        ))
        .await
        .expect("schema objects")
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "type").expect("object type"),
                row.try_get("", "name").expect("object name"),
                row.try_get("", "sql").expect("object definition"),
            )
        })
        .collect()
    }

    #[tokio::test]
    async fn current_version_marker_without_schema_is_rejected() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION).await;
        let before = schema_objects(&db).await;
        let error = initialize_schema(&db)
            .await
            .expect_err("version alone does not establish compatibility");
        assert!(error.to_string().contains("incompatible"), "{error}");
        assert_eq!(read_schema_version(&db).await, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            schema_objects(&db).await,
            before,
            "rejection must not create missing objects"
        );
    }

    #[tokio::test]
    async fn modified_current_schema_is_rejected_without_repairing_it() {
        for mutation in [
            "ALTER TABLE agena_parts ADD COLUMN legacy_rendering TEXT",
            "DROP INDEX uq_agena_workspace_path",
            "CREATE TABLE agena_obsolete_content (id INTEGER PRIMARY KEY)",
        ] {
            let db = database_with_version(0).await;
            initialize_schema(&db).await.expect("create schema");
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, mutation))
                .await
                .expect("simulate incompatible structure");
            let before = schema_objects(&db).await;
            let error = initialize_schema(&db)
                .await
                .expect_err("reject modified current schema");
            assert!(
                error.to_string().contains("incompatible"),
                "{mutation}: {error}"
            );
            assert_eq!(schema_objects(&db).await, before, "{mutation}");
            assert_eq!(read_schema_version(&db).await, CURRENT_SCHEMA_VERSION);
        }
    }

    #[tokio::test]
    async fn nonempty_unversioned_database_is_rejected_without_claiming_it() {
        let db = database_with_version(0).await;
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE marker (value TEXT NOT NULL)",
        ))
        .await
        .expect("preexisting data schema");
        let before = schema_objects(&db).await;
        initialize_schema(&db)
            .await
            .expect_err("unversioned is not necessarily fresh");
        assert_eq!(read_schema_version(&db).await, 0);
        assert_eq!(schema_objects(&db).await, before);
    }

    #[tokio::test]
    async fn incompatible_schema_rejection_preserves_connection_pragmas() {
        for version in [7, CURRENT_SCHEMA_VERSION + 1] {
            let db = database_with_version(version).await;
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA synchronous = FULL",
            ))
            .await
            .expect("set durability");
            initialize_schema(&db).await.expect_err("reject version");
            let mode: i64 = db
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "PRAGMA synchronous",
                ))
                .await
                .expect("durability")
                .expect("row")
                .try_get("", "synchronous")
                .expect("mode");
            assert_eq!(mode, 2, "rejection must precede durability changes");
        }
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
