//! Cross-process execution lease primitives.
//!
//! Multiple agena processes share one SQLite database. A per-session lease
//! (an `INSERT ... ON CONFLICT DO NOTHING` on `agena_execution_leases`)
//! guarantees that only one process executes a given session at a time, and
//! lets a process that owns a lease distinguish "another process is actively
//! running this session" from "this session's run crashed and needs
//! reconciliation". Ownership is identified by a per-process `owner_id`.

use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement};

const TABLE: &str = "agena_execution_leases";

/// Outcome of attempting to acquire a session's execution lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquireOutcome {
    /// This caller now owns the lease and may execute the session.
    Acquired,
    /// Another process owns the lease and is (or was recently) active.
    HeldBy { owner_id: String, heartbeat_at_ms: i64 },
}

/// A single lease row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRow {
    pub session_id: i64,
    pub owner_id: String,
    pub run_id: Option<String>,
    pub lease_started_at_ms: i64,
    pub heartbeat_at_ms: i64,
}

/// Try to acquire the execution lease for `session_id`.
///
/// Returns `Acquired` when this caller won the insert, or `HeldBy` with the
/// current owner when another process holds the lease.
pub async fn try_acquire_lease<C>(
    db: &C,
    session_id: i64,
    owner_id: &str,
    run_id: Option<&str>,
    now_ms: i64,
) -> Result<LeaseAcquireOutcome, DbErr>
where
    C: ConnectionTrait,
{
    let result = db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {TABLE} (session_id, owner_id, run_id, lease_started_at_ms, heartbeat_at_ms) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT(session_id) DO NOTHING"
            ),
            [
                session_id.into(),
                owner_id.to_owned().into(),
                run_id.map(str::to_owned).into(),
                now_ms.into(),
                now_ms.into(),
            ],
        ))
        .await?;
    if result.rows_affected() == 1 {
        return Ok(LeaseAcquireOutcome::Acquired);
    }
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT owner_id, heartbeat_at_ms FROM {TABLE} WHERE session_id = ?"
            ),
            [session_id.into()],
        ))
        .await?
        .ok_or_else(|| {
            DbErr::Custom(format!("lease row vanished after failed acquire for session {session_id}"))
        })?;
    Ok(LeaseAcquireOutcome::HeldBy {
        owner_id: row.try_get("", "owner_id")?,
        heartbeat_at_ms: row.try_get("", "heartbeat_at_ms")?,
    })
}

/// Release the lease for `session_id` only if this caller still owns it.
/// Returns `true` when a row was released.
pub async fn release_lease<C>(
    db: &C,
    session_id: i64,
    owner_id: &str,
) -> Result<bool, DbErr>
where
    C: ConnectionTrait,
{
    let result = db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("DELETE FROM {TABLE} WHERE session_id = ? AND owner_id = ?"),
            [session_id.into(), owner_id.to_owned().into()],
        ))
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Refresh the lease heartbeat. Returns `true` when this caller still owns
/// the lease (and the heartbeat was updated).
pub async fn heartbeat<C>(
    db: &C,
    session_id: i64,
    owner_id: &str,
    now_ms: i64,
) -> Result<bool, DbErr>
where
    C: ConnectionTrait,
{
    let result = db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "UPDATE {TABLE} SET heartbeat_at_ms = ? WHERE session_id = ? AND owner_id = ?"
            ),
            [now_ms.into(), session_id.into(), owner_id.to_owned().into()],
        ))
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Read the current lease row for `session_id`, if any.
pub async fn lease<C>(db: &C, session_id: i64) -> Result<Option<LeaseRow>, DbErr>
where
    C: ConnectionTrait,
{
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT session_id, owner_id, run_id, lease_started_at_ms, heartbeat_at_ms \
                 FROM {TABLE} WHERE session_id = ?"
            ),
            [session_id.into()],
        ))
        .await?;
    row.map(|row| {
        Ok(LeaseRow {
            session_id: row.try_get("", "session_id")?,
            owner_id: row.try_get("", "owner_id")?,
            run_id: row.try_get("", "run_id")?,
            lease_started_at_ms: row.try_get("", "lease_started_at_ms")?,
            heartbeat_at_ms: row.try_get("", "heartbeat_at_ms")?,
        })
    })
    .transpose()
}

/// Delete every lease whose heartbeat is older than `stale_before_ms` and
/// return the session ids that were reclaimed. The caller may then reconcile
/// those sessions' interrupted runs.
pub async fn reap_stale_leases<C>(
    db: &C,
    stale_before_ms: i64,
) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "DELETE FROM {TABLE} WHERE heartbeat_at_ms < ? \
                 RETURNING session_id"
            ),
            [stale_before_ms.into()],
        ))
        .await?;
    let mut session_ids = Vec::with_capacity(rows.len());
    for row in rows {
        session_ids.push(row.try_get("", "session_id")?);
    }
    Ok(session_ids)
}

/// A future timestamp that is guaranteed to be "fresh" (used as the stale
/// threshold for a lease whose owner never sent a heartbeat).
pub fn lease_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
