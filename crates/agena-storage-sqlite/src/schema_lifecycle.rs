//! SQLite schema lifecycle independent of any particular SeaORM entity set.
//!
//! Entity-derived table/index definitions are supplied by the current
//! composition owner while the SQLite backend contract, version marker, and
//! migration transaction live with the concrete storage implementation.

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, DbErr, Statement,
    TransactionTrait,
};

/// Current SQLite schema version written to `PRAGMA user_version`.
pub const CURRENT_SCHEMA_VERSION: i64 = 9;

/// Schema versions that `apply_migrations` knows how to upgrade in place.
/// Older databases report an error instead of being migrated.
const MIGRATABLE_VERSIONS: &[i64] = &[8];

/// Validates SQLite invariants and opens the transaction that must contain
/// table/index/trigger creation plus the schema-version update.
pub async fn begin_schema_initialization(
    db: &DatabaseConnection,
) -> Result<(DatabaseTransaction, i64), DbErr> {
    if db.get_database_backend() != DatabaseBackend::Sqlite {
        return Err(DbErr::Custom("Agena currently requires SQLite".to_owned()));
    }
    ensure_sqlite_foreign_keys(db).await?;
    let current_version = read_schema_version(db).await?;
    if current_version != 0
        && current_version != CURRENT_SCHEMA_VERSION
        && !MIGRATABLE_VERSIONS.contains(&current_version)
    {
        let relation = if current_version < CURRENT_SCHEMA_VERSION {
            "older than"
        } else {
            "newer than"
        };
        return Err(DbErr::Custom(format!(
            "database schema version {current_version} is {relation} the supported version {CURRENT_SCHEMA_VERSION}; Agena does not migrate incompatible development databases, so create a fresh database"
        )));
    }
    Ok((db.begin().await?, current_version))
}

/// Writes pending version markers and commits the initialization transaction.
pub async fn complete_schema_initialization(
    transaction: DatabaseTransaction,
    from_version: i64,
) -> Result<(), DbErr> {
    apply_migrations(&transaction, from_version).await?;
    transaction.commit().await
}

async fn read_schema_version(db: &DatabaseConnection) -> Result<i64, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA user_version".to_owned(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("SQLite did not return user_version".to_owned()))?;
    row.try_get("", "user_version")
}

async fn ensure_sqlite_foreign_keys(db: &DatabaseConnection) -> Result<(), DbErr> {
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA foreign_keys".to_owned(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("SQLite did not return PRAGMA foreign_keys".to_owned()))?;
    let enabled: i64 = row.try_get("", "foreign_keys")?;
    if enabled == 1 {
        Ok(())
    } else {
        Err(DbErr::Custom(
            "SQLite foreign-key enforcement must be enabled for Agena".to_owned(),
        ))
    }
}

async fn apply_migrations<C: ConnectionTrait>(db: &C, from_version: i64) -> Result<(), DbErr> {
    match from_version {
        // Already current: nothing to do.
        CURRENT_SCHEMA_VERSION => {}
        // Fresh database: `schema.rs` already creates every table with the
        // current column set, so only the version marker is needed.
        0 => {
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
            ))
            .await?;
        }
        // 8 -> 9: assistant replies carry an optional structured failure
        // projection (`failure_json`) so clients can render a readable
        // failure summary with expandable detail. Existing failed replies
        // are backfilled from their last `execution_finished` event.
        8 => {
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "ALTER TABLE agena_assistant_replies ADD COLUMN failure_json JSON NULL",
            ))
            .await?;
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "UPDATE agena_assistant_replies AS r \
                 SET failure_json = ( \
                   SELECT json_extract(e.payload_json, '$.payload.outcome.failure') \
                   FROM agena_events e \
                   WHERE e.kind_tag = 'execution_finished' \
                     AND json_extract(e.payload_json, '$.payload.reply_id') = r.reply_id \
                     AND json_extract(e.payload_json, '$.payload.outcome.failure') IS NOT NULL \
                   ORDER BY e.seq_session DESC \
                   LIMIT 1 \
                 ) \
                 WHERE r.status = 'failed' AND r.failure_json IS NULL",
            ))
            .await?;
            db.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
            ))
            .await?;
        }
        version => {
            return Err(DbErr::Custom(format!(
                "unsupported schema migration path from version {version} to {CURRENT_SCHEMA_VERSION}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    use super::*;

    async fn database_with_version(version: i64) -> DatabaseConnection {
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

    #[tokio::test]
    async fn fresh_database_is_initialized_at_current_version() {
        let db = database_with_version(0).await;

        let (transaction, from_version) = begin_schema_initialization(&db)
            .await
            .expect("begin fresh schema initialization");
        complete_schema_initialization(transaction, from_version)
            .await
            .expect("complete fresh schema initialization");

        assert_eq!(
            read_schema_version(&db).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[tokio::test]
    async fn current_database_is_accepted_without_rewriting_version() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION).await;

        let (transaction, from_version) = begin_schema_initialization(&db)
            .await
            .expect("begin current schema initialization");
        complete_schema_initialization(transaction, from_version)
            .await
            .expect("complete current schema initialization");

        assert_eq!(
            read_schema_version(&db).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[tokio::test]
    async fn migratable_older_database_is_upgraded_in_place() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION - 1).await;
        // Simulate the v8 assistant-replies table (without failure_json).
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_assistant_replies (reply_id TEXT PRIMARY KEY, \
             turn_id TEXT NOT NULL UNIQUE, status TEXT NOT NULL, revision_seq INTEGER NOT NULL, \
             created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL)"
                .to_owned(),
        ))
        .await
        .unwrap();
                db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_events (id INTEGER PRIMARY KEY AUTOINCREMENT, event_uuid TEXT NOT NULL UNIQUE, \
             seq_global INTEGER NOT NULL UNIQUE, seq_session INTEGER NULL, session_id INTEGER NULL, \
             workspace_id INTEGER NULL, kind_tag TEXT NOT NULL, envelope_schema INTEGER NOT NULL, \
             payload_json JSON NOT NULL, causation_uuid TEXT NULL, correlation_uuid TEXT NULL, \
             created_at_ms INTEGER NOT NULL)"
                .to_owned(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_assistant_replies (reply_id, turn_id, status, revision_seq, created_at_ms) \
             VALUES ('reply-1', 'turn-1', 'failed', 1, 0)"
                .to_owned(),
        ))
        .await
        .unwrap();

        let (transaction, from_version) = begin_schema_initialization(&db)
            .await
            .expect("begin migratable schema initialization");
        complete_schema_initialization(transaction, from_version)
            .await
            .expect("complete schema migration");

        assert_eq!(
            read_schema_version(&db).await.unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        // The new column exists and pre-existing rows are preserved.
        let columns = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA table_info(agena_assistant_replies)".to_owned(),
            ))
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .any(|row| { row.try_get::<String>("", "name").unwrap() == "failure_json" })
        );
        let preserved = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT status FROM agena_assistant_replies WHERE reply_id = 'reply-1'".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(preserved.try_get::<String>("", "status").unwrap(), "failed");
    }

    #[tokio::test]
    async fn incompatible_older_database_is_rejected_without_mutation() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION - 2).await;
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE legacy_marker (value TEXT NOT NULL)".to_owned(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO legacy_marker (value) VALUES ('preserved')".to_owned(),
        ))
        .await
        .unwrap();

        let error = begin_schema_initialization(&db).await.unwrap_err();

        assert!(error.to_string().contains("does not migrate"));
        assert_eq!(
            read_schema_version(&db).await.unwrap(),
            CURRENT_SCHEMA_VERSION - 2
        );
        let marker = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT value FROM legacy_marker".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.try_get::<String>("", "value").unwrap(), "preserved");
    }

    #[tokio::test]
    async fn newer_database_is_rejected() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION + 1).await;

        let error = begin_schema_initialization(&db).await.unwrap_err();

        assert!(error.to_string().contains("newer than"));
    }
}
