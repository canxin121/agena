//! Schema lifecycle for the dedicated scheduler SQLite database.
//!
//! The scheduler keeps its own SQLite database (`~/.agena/scheduler.db` by
//! default) rather than sharing the chat database, so its tables and version
//! marker live here instead of `agena-storage-sqlite`. Like the chat schema,
//! versioning uses `PRAGMA user_version`; version zero means "not yet created"
//! and a fresh database is created in one DDL transaction. Incompatible older
//! versions are rejected rather than migrated.
//!
//! The scheduler's version space is independent from the chat schema's and
//! starts at 1.

use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait};

/// Current scheduler SQLite schema version written to `PRAGMA user_version`.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// How long `initialize_schema` waits for a concurrent process to finish
/// building the schema before giving up.
const SCHEMA_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Serializes schema creation across processes.
///
/// This mirrors the lock in `agena-storage-sqlite::schema`: SQLite's
/// `PRAGMA journal_mode = WAL` needs an exclusive lock that the busy timeout
/// does not wait on, so two processes cold-starting the same database file
/// would otherwise race and one would fail with `SQLITE_BUSY`. A filesystem
/// lock on a sibling `.schema-lock` file serializes the whole create path.
/// In-memory databases have no backing file and skip the lock.
///
/// The lock is held for the lifetime of this guard: dropping it releases the
/// advisory file lock.
struct SchemaLock {
    // Held only so the file (and its lock) outlives the guard.
    _file: std::fs::File,
}

impl SchemaLock {
    async fn acquire(db: &DatabaseConnection) -> Result<Option<SchemaLock>, DbErr> {
        let Some(lock_path) = schema_lock_path(db).await? else {
            return Ok(None);
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                DbErr::Custom(format!(
                    "open schema lock file {}: {error}",
                    lock_path.display()
                ))
            })?;
        let started = std::time::Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(SchemaLock { _file: file })),
                Err(_) if started.elapsed() < SCHEMA_LOCK_TIMEOUT => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(error) => {
                    return Err(DbErr::Custom(format!(
                        "timed out acquiring schema lock {}: {error}",
                        lock_path.display()
                    )));
                }
            }
        }
    }
}

/// Resolve the `<db>.schema-lock` path for a SQLite connection, or `None` for
/// in-memory databases. Uses `PRAGMA database_list` which reports the absolute
/// backing-file path of the main database.
async fn schema_lock_path(db: &DatabaseConnection) -> Result<Option<PathBuf>, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA database_list".to_owned(),
        ))
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let file: String = row.try_get("", "file")?;
    if file.is_empty() || file == ":memory:" {
        return Ok(None);
    }
    let path = Path::new(&file);
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".schema-lock");
    Ok(Some(PathBuf::from(lock_path)))
}

/// Creates the scheduler schema and applies its version marker atomically.
///
/// Serialized across processes by a filesystem lock so concurrent cold starts
/// of the same database file cannot race the WAL switch or the DDL transaction.
/// A version-0 database is created from scratch; a database already at
/// [`CURRENT_SCHEMA_VERSION`] is left untouched; anything else is rejected
/// (Agena does not migrate incompatible databases).
pub async fn initialize_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let _lock = SchemaLock::acquire(db).await?;
    // Connection hardening: WAL journal (no-op for in-memory databases),
    // bounded busy timeout, and NORMAL durability — identical to the chat
    // schema so the scheduler database behaves the same under concurrency.
    for pragma in [
        "PRAGMA journal_mode = WAL",
        "PRAGMA busy_timeout = 15000",
        "PRAGMA synchronous = NORMAL",
    ] {
        db.execute(Statement::from_string(
            db.get_database_backend(),
            pragma.to_owned(),
        ))
        .await?;
    }
    let current_version = read_schema_version(db).await?;
    match current_version {
        0 => {
            let txn = db.begin().await?;
            for statement in TABLES.iter().chain(INDEXES) {
                txn.execute(Statement::from_string(
                    txn.get_database_backend(),
                    (*statement).to_owned(),
                ))
                .await?;
            }
            txn.execute(Statement::from_string(
                txn.get_database_backend(),
                format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
            ))
            .await?;
            txn.commit().await
        }
        v if v == CURRENT_SCHEMA_VERSION => Ok(()),
        v => Err(DbErr::Custom(format!(
            "scheduler database schema version {v} is incompatible with the supported version \
             {CURRENT_SCHEMA_VERSION}; Agena does not migrate incompatible databases, so create a \
             fresh database"
        ))),
    }
}

async fn read_schema_version(db: &DatabaseConnection) -> Result<i64, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA user_version".to_owned(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("SQLite did not return user_version".to_owned()))?;
    row.try_get("", "user_version")
}

/// `agena_scheduler_jobs` mirrors the hot scheduling fields of `ScheduledJob`
/// (`retry_at_ms`, `paused`, `completed`) as columns alongside
/// `next_fire_at_ms` so the scheduler's due scan can filter in SQL instead of
/// decoding every job JSON every tick. `delivery_key` / `claimed_at_ms` are the
/// cross-process claim lock. `job_json` remains the source of truth for the
/// full in-memory state; the columns are derived copies written together.
const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, retry_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, paused INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)",
];

const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_next_fire ON agena_scheduler_jobs(next_fire_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_finished ON agena_scheduler_history(finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_job_finished ON agena_scheduler_history(job_id, finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_jobs_delivery ON agena_scheduler_jobs(delivery_key) WHERE delivery_key IS NOT NULL",
];

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    use super::*;

    async fn read_version(db: &DatabaseConnection) -> i64 {
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
    async fn fresh_scheduler_database_initializes_at_current_version() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        initialize_schema(&db)
            .await
            .expect("initialize scheduler schema");
        assert_eq!(read_version(&db).await, CURRENT_SCHEMA_VERSION);
        for table in ["agena_scheduler_jobs", "agena_scheduler_history"] {
            let count: i64 = db
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    format!(
                        "SELECT COUNT(*) AS count FROM sqlite_master \
                         WHERE type = 'table' AND name = '{table}'"
                    ),
                ))
                .await
                .expect("query table")
                .expect("table row")
                .try_get("", "count")
                .expect("count value");
            assert_eq!(count, 1, "scheduler table {table} must exist");
        }
    }

    #[tokio::test]
    async fn incompatible_scheduler_database_is_rejected() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA user_version = 99".to_owned(),
        ))
        .await
        .expect("set schema version");

        let error = initialize_schema(&db)
            .await
            .expect_err("reject incompatible scheduler schema");
        assert!(error.to_string().contains("does not migrate"));
    }
}
