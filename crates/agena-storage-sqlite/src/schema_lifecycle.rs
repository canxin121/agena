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
pub const CURRENT_SCHEMA_VERSION: i64 = 5;

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
    if current_version != 0 && current_version != CURRENT_SCHEMA_VERSION {
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
    if from_version == 0 {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
        ))
        .await?;
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
    async fn incompatible_older_database_is_rejected_without_mutation() {
        let db = database_with_version(CURRENT_SCHEMA_VERSION - 1).await;
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
            CURRENT_SCHEMA_VERSION - 1
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
