//! Current schema for the dedicated scheduler SQLite database.
//!
//! The scheduler has no database migration layer. Empty databases are created
//! from the current declarations; non-empty databases must match exactly.

use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait};

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

/// Create the current scheduler schema or validate an existing one exactly.
pub async fn initialize_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let _lock = SchemaLock::acquire(db).await?;
    let objects = schema_objects(db).await?;
    let fresh = objects.is_empty();
    if !fresh {
        validate_existing_schema(&objects)?;
    }
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
    if !fresh {
        return Ok(());
    }
    let txn = db.begin().await?;
    for statement in TABLES.iter().chain(INDEXES) {
        txn.execute(Statement::from_string(
            txn.get_database_backend(),
            (*statement).to_owned(),
        ))
        .await?;
    }
    txn.commit().await
}

fn declaration(sql: &str) -> Result<(String, String, String), DbErr> {
    for (prefix, kind) in [
        ("CREATE TABLE IF NOT EXISTS ", "table"),
        ("CREATE INDEX IF NOT EXISTS ", "index"),
        ("CREATE UNIQUE INDEX IF NOT EXISTS ", "index"),
    ] {
        if let Some(rest) = sql.strip_prefix(prefix) {
            let name = rest.split([' ', '(']).next().unwrap_or_default();
            if !name.is_empty() {
                return Ok((
                    kind.into(),
                    name.into(),
                    sql.replacen(" IF NOT EXISTS ", " ", 1),
                ));
            }
        }
    }
    Err(DbErr::Custom(
        "invalid internal scheduler schema declaration".into(),
    ))
}

async fn schema_objects<C: ConnectionTrait>(
    db: &C,
) -> Result<std::collections::BTreeMap<(String, String), String>, DbErr> {
    db.query_all(Statement::from_string(
        db.get_database_backend(),
        "SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
    ))
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            (row.try_get("", "type")?, row.try_get("", "name")?),
            row.try_get("", "sql")?,
        ))
    })
    .collect()
}

fn validate_existing_schema(
    actual: &std::collections::BTreeMap<(String, String), String>,
) -> Result<(), DbErr> {
    let mut expected = std::collections::BTreeMap::new();
    for sql in TABLES.iter().chain(INDEXES) {
        let (kind, name, stored) = declaration(sql)?;
        expected.insert((kind, name), stored);
    }
    if actual == &expected {
        Ok(())
    } else {
        Err(DbErr::Custom("scheduler database schema does not match this Agena build; delete it and start with a fresh database".into()))
    }
}

/// `agena_scheduler_jobs` mirrors the hot scheduling fields of `ScheduledJob`
/// (`retry_at_ms`, `paused`, `completed`) as columns alongside
/// `next_fire_at_ms` so the scheduler's due scan can filter in SQL instead of
/// decoding every job JSON every tick. `delivery_key` / `claimed_at_ms` are the
/// cross-process claim lease: `delivery_key` identifies one worker attempt,
/// and `claimed_at_ms` is refreshed by its heartbeat. The stable business key
/// and original claim time remain in `job_json.pending_delivery`. Other hot
/// columns are derived copies of the JSON, written together with it.
const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, retry_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, paused INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, owner_workspace TEXT NULL, owner_session_id INTEGER NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)",
];

const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_next_fire ON agena_scheduler_jobs(next_fire_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_finished ON agena_scheduler_history(finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_job_finished ON agena_scheduler_history(job_id, finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_jobs_delivery ON agena_scheduler_jobs(delivery_key) WHERE delivery_key IS NOT NULL",
];

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    #[tokio::test]
    async fn fresh_scheduler_database_is_created_and_reopens_exactly() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize_schema(&db).await.unwrap();
        let before = schema_objects(&db).await.unwrap();
        validate_existing_schema(&before).unwrap();
        initialize_schema(&db).await.unwrap();
        assert_eq!(schema_objects(&db).await.unwrap(), before);
    }

    #[tokio::test]
    async fn modified_scheduler_database_is_rejected_without_repair() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        initialize_schema(&db).await.unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE agena_scheduler_jobs ADD COLUMN obsolete TEXT",
        ))
        .await
        .unwrap();
        let before = schema_objects(&db).await.unwrap();
        initialize_schema(&db)
            .await
            .expect_err("modified schema must be rejected");
        assert_eq!(schema_objects(&db).await.unwrap(), before);
    }
}
