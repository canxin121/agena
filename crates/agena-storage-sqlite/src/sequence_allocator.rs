//! Database-backed implementation of [`SequenceAllocator`].
//!
//! All allocation happens atomically inside the SQLite database, so multiple
//! processes sharing one database file can never hand out a duplicate
//! `seq_global`, per-session `seq_session`, or projected message/part id.

use std::sync::Arc;

use agena_storage::{EventStoreError, SequenceAllocator};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};

const GLOBAL_TABLE: &str = "agena_sequences";
const SESSION_TABLE: &str = "agena_session_sequences";
const EVENTS: &str = "agena_events";

/// SQLite implementation of [`SequenceAllocator`].
#[derive(Debug)]
pub struct SqliteSequenceAllocator {
    db: Arc<DatabaseConnection>,
}

impl SqliteSequenceAllocator {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

/// Allocate the next value of a named sequence. `next_val` stores the next
/// value to hand out, so the upsert bumps it by one and returns the previous
/// value.
async fn allocate_sequence(db: &DatabaseConnection, name: &str) -> Result<i64, EventStoreError> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {GLOBAL_TABLE} (seq_name, next_val) VALUES (?, 1) \
                 ON CONFLICT(seq_name) DO UPDATE SET next_val = {GLOBAL_TABLE}.next_val + 1 \
                 RETURNING next_val - 1 AS allocated"
            ),
            [Value::from(name)],
        ))
        .await
        .map_err(backend_error)?;
    let row = row.ok_or_else(|| {
        EventStoreError::Backend("sequence allocation returned no row".to_owned())
    })?;
    row.try_get::<i64>("", "allocated").map_err(backend_error)
}

/// Allocate a block of `count` consecutive values from a named sequence.
/// Returns `first - 1` so callers that keep the existing "first id offset"
/// semantics can assign ids `first..first+count`.
async fn allocate_sequence_block(
    db: &DatabaseConnection,
    name: &str,
    count: i64,
) -> Result<i64, EventStoreError> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {GLOBAL_TABLE} (seq_name, next_val) VALUES (?, 1) \
                 ON CONFLICT(seq_name) DO UPDATE SET next_val = {GLOBAL_TABLE}.next_val + ? \
                 RETURNING next_val - ? - 1 AS first_minus_one"
            ),
            [Value::from(name), count.into(), count.into()],
        ))
        .await
        .map_err(backend_error)?;
    let row = row.ok_or_else(|| {
        EventStoreError::Backend("sequence block allocation returned no row".to_owned())
    })?;
    row.try_get::<i64>("", "first_minus_one")
        .map_err(backend_error)
}

/// Raise a named sequence floor to at least `target` (idempotent).
async fn seed_sequence(
    db: &DatabaseConnection,
    name: &str,
    target: i64,
) -> Result<(), EventStoreError> {
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        format!(
            "INSERT INTO {GLOBAL_TABLE} (seq_name, next_val) VALUES (?, ?) \
             ON CONFLICT(seq_name) DO UPDATE SET next_val = MAX({GLOBAL_TABLE}.next_val, excluded.next_val)"
        ),
        [Value::from(name), target.into()],
    ))
    .await
    .map_err(backend_error)?;
    Ok(())
}

/// Allocate the next per-session sequence, seeding the session row from the
/// event high watermark on first use.
async fn allocate_session_sequence(
    db: &DatabaseConnection,
    session_id: i64,
) -> Result<i64, EventStoreError> {
    // Seed (idempotent): never below the maximum seq_session already persisted.
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        format!(
            "INSERT INTO {SESSION_TABLE} (session_id, next_val) \
             VALUES (?, (SELECT COALESCE(MAX(seq_session), 0) + 1 FROM {EVENTS} WHERE session_id = ?)) \
             ON CONFLICT(session_id) DO UPDATE SET next_val = MAX({SESSION_TABLE}.next_val, excluded.next_val)"
        ),
        [session_id.into(), session_id.into()],
    ))
    .await
    .map_err(backend_error)?;
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {SESSION_TABLE} (session_id, next_val) VALUES (?, 1) \
                 ON CONFLICT(session_id) DO UPDATE SET next_val = {SESSION_TABLE}.next_val + 1 \
                 RETURNING next_val - 1 AS allocated"
            ),
            [session_id.into()],
        ))
        .await
        .map_err(backend_error)?;
    let row = row.ok_or_else(|| {
        EventStoreError::Backend("session sequence allocation returned no row".to_owned())
    })?;
    row.try_get::<i64>("", "allocated").map_err(backend_error)
}

fn backend_error(error: impl std::fmt::Display) -> EventStoreError {
    EventStoreError::Backend(error.to_string())
}

#[async_trait]
impl SequenceAllocator for SqliteSequenceAllocator {
    async fn next_seq_global(&self) -> Result<i64, EventStoreError> {
        allocate_sequence(self.db.as_ref(), "seq_global").await
    }
    async fn next_seq_session(&self, session_id: i64) -> Result<i64, EventStoreError> {
        allocate_session_sequence(self.db.as_ref(), session_id).await
    }
    async fn next_message_id(&self) -> Result<i64, EventStoreError> {
        allocate_sequence(self.db.as_ref(), "message_id").await
    }
    async fn next_part_id(&self) -> Result<i64, EventStoreError> {
        allocate_sequence(self.db.as_ref(), "part_id").await
    }
    async fn reserve_message_id_block(&self, count: i64) -> Result<i64, EventStoreError> {
        allocate_sequence_block(self.db.as_ref(), "message_id", count).await
    }
    async fn reserve_part_id_block(&self, count: i64) -> Result<i64, EventStoreError> {
        allocate_sequence_block(self.db.as_ref(), "part_id", count).await
    }
    async fn seed_global(&self, high: i64) -> Result<(), EventStoreError> {
        seed_sequence(self.db.as_ref(), "seq_global", high + 1).await
    }
    async fn seed_message_id(&self, high: i64) -> Result<(), EventStoreError> {
        seed_sequence(self.db.as_ref(), "message_id", high + 1).await
    }
    async fn seed_part_id(&self, high: i64) -> Result<(), EventStoreError> {
        seed_sequence(self.db.as_ref(), "part_id", high + 1).await
    }
}
