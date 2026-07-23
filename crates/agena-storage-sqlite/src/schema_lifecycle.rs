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
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

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
    if current_version > CURRENT_SCHEMA_VERSION {
        return Err(DbErr::Custom(format!(
            "database schema version {current_version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
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
    if from_version < 1 {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA user_version = 1".to_owned(),
        ))
        .await?;
    }
    Ok(())
}
