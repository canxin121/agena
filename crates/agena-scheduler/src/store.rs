//! Job persistence, optimistic edits, and renewable delivery ownership.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use uuid::Uuid;

use crate::error::{SchedulerError, SchedulerResult};
use crate::job::{ScheduledJob, SchedulerHistoryEntry};

pub const MAX_RETAINED_HISTORY_ENTRIES: usize = 1_000;

/// A worker renews every 30 seconds. An abandoned claim becomes recoverable
/// after 90 seconds, retaining its business delivery key for sink deduplication.
/// `claimed_at_ms` in schema v1 holds the most recent renewal; the original
/// attempt start time remains in `job_json.pending_delivery.claimed_at`.
pub const CLAIM_LEASE_MILLIS: i64 = 90_000;
pub(crate) const CLAIM_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

/// A read snapshot with an opaque compare token. The exact stored JSON and
/// claim owner detect concurrent edits, including changes that do not alter
/// next_fire_at. Heartbeats do not invalidate a configuration edit.
#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub job: ScheduledJob,
    json: String,
    claim_key: Option<String>,
    renewed_at_ms: Option<i64>,
}

impl JobSnapshot {
    pub fn claim_key(&self) -> Option<&str> {
        self.claim_key.as_deref()
    }

    fn available(&self, now_ms: i64) -> bool {
        self.claim_key.is_none()
            || self
                .renewed_at_ms
                .is_none_or(|time| time <= lease_cutoff(now_ms))
    }

    fn matches(&self, expected: &Self) -> bool {
        self.json == expected.json && self.claim_key == expected.claim_key
    }
}

fn lease_cutoff(now_ms: i64) -> i64 {
    now_ms.saturating_sub(CLAIM_LEASE_MILLIS)
}

/// A failed database operation is distinct from an absent job or a lost
/// optimistic race. State changes that produce a new last_run commit that
/// record to the bounded history ledger in the same transaction.
#[async_trait::async_trait]
pub trait JobStore: Send + Sync {
    /// Insert a new job. Existing IDs are rejected, never silently overwritten.
    async fn put(&self, job: ScheduledJob) -> SchedulerResult<()>;
    async fn remove(&self, id: Uuid) -> SchedulerResult<bool>;
    /// Delete only the snapshot whose immutable owner was authorized.
    async fn remove_checked(&self, _expected: &JobSnapshot) -> SchedulerResult<bool> {
        Err(SchedulerError::InvalidUpdate(
            "conditional deletion unsupported by this store".into(),
        ))
    }
    async fn list_history_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        job_id: Option<Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>> {
        Ok(self
            .list_history(job_id, MAX_RETAINED_HISTORY_ENTRIES)
            .await?
            .into_iter()
            .filter(|entry| {
                entry.owner_workspace.as_deref() == Some(workspace)
                    && entry.owner_session_id == session
            })
            .take(limit.clamp(1, MAX_RETAINED_HISTORY_ENTRIES))
            .collect())
    }
    async fn list(&self) -> SchedulerResult<Vec<JobSnapshot>>;
    async fn get(&self, id: Uuid) -> SchedulerResult<Option<JobSnapshot>>;
    /// Due unclaimed jobs and abandoned, unpaused claims. Callers claim each
    /// candidate immediately before delivery, never a batch ahead of time.
    async fn list_due(&self, now_ms: i64) -> SchedulerResult<Vec<JobSnapshot>>;
    async fn replace(&self, expected: &JobSnapshot, job: ScheduledJob) -> SchedulerResult<bool>;
    async fn claim(
        &self,
        expected: &JobSnapshot,
        job: ScheduledJob,
        claim_key: String,
        now_ms: i64,
    ) -> SchedulerResult<bool>;
    /// Renew only an unexpired claim still owned by this attempt. A worker
    /// that loses ownership must drop its sink future and cannot finalize.
    async fn renew(&self, id: Uuid, claim_key: &str, now_ms: i64) -> SchedulerResult<bool>;
    async fn finish(
        &self,
        expected: &JobSnapshot,
        claim_key: &str,
        job: ScheduledJob,
        now_ms: i64,
    ) -> SchedulerResult<bool>;
    async fn append_history(&self, entry: SchedulerHistoryEntry) -> SchedulerResult<()>;
    async fn list_history(
        &self,
        job_id: Option<Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>>;
}

#[derive(Default, Clone)]
pub struct InMemoryJobStore {
    inner: Arc<RwLock<MemoryState>>,
}

#[derive(Default)]
struct MemoryState {
    jobs: HashMap<Uuid, JobSnapshot>,
    // One lock makes job updates and their history entry observable together.
    history: Vec<SchedulerHistoryEntry>,
}

impl MemoryState {
    fn append_history(&mut self, entry: SchedulerHistoryEntry) {
        self.history.push(entry);
        // Stable sorting keeps insertion order as the tie breaker, like SQL id.
        self.history.sort_by_key(|entry| entry.record.finished_at);
        let excess = self
            .history
            .len()
            .saturating_sub(MAX_RETAINED_HISTORY_ENTRIES);
        self.history.drain(..excess);
    }

    fn save(&mut self, expected: &JobSnapshot, next: JobSnapshot) {
        if let Some(entry) = new_history(expected, &next.job) {
            self.append_history(entry);
        }
        self.jobs.insert(next.job.id, next);
    }
}

fn new_history(expected: &JobSnapshot, job: &ScheduledJob) -> Option<SchedulerHistoryEntry> {
    (expected.job.last_run != job.last_run)
        .then(|| job.last_run.clone())
        .flatten()
        .map(|record| SchedulerHistoryEntry {
            job_id: job.id,
            owner_session_id: job.owner_session_id,
            owner_workspace: job.owner_workspace.clone(),
            record,
        })
}

fn encode_update(expected: &JobSnapshot, job: &ScheduledJob) -> SchedulerResult<String> {
    if expected.job.owner_session_id != job.owner_session_id
        || expected.job.owner_workspace != job.owner_workspace
    {
        return Err(SchedulerError::InvalidUpdate(
            "job ownership is immutable".into(),
        ));
    }
    if expected.job.id != job.id {
        return Err(SchedulerError::InvalidUpdate("job id cannot change".into()));
    }
    Ok(serde_json::to_string(job)?)
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl JobStore for InMemoryJobStore {
    async fn put(&self, job: ScheduledJob) -> SchedulerResult<()> {
        let json = serde_json::to_string(&job)?;
        let mut state = self.inner.write();
        if state.jobs.contains_key(&job.id) {
            return Err(SchedulerError::Conflict(job.id));
        }
        state.jobs.insert(
            job.id,
            JobSnapshot {
                job,
                json,
                claim_key: None,
                renewed_at_ms: None,
            },
        );
        Ok(())
    }

    async fn remove_checked(&self, expected: &JobSnapshot) -> SchedulerResult<bool> {
        let mut state = self.inner.write();
        if state
            .jobs
            .get(&expected.job.id)
            .is_none_or(|actual| !actual.matches(expected))
        {
            return Ok(false);
        }
        state.jobs.remove(&expected.job.id);
        Ok(true)
    }

    async fn remove(&self, id: Uuid) -> SchedulerResult<bool> {
        Ok(self.inner.write().jobs.remove(&id).is_some())
    }

    async fn list(&self) -> SchedulerResult<Vec<JobSnapshot>> {
        let mut jobs: Vec<_> = self.inner.read().jobs.values().cloned().collect();
        jobs.sort_by_key(|entry| {
            (
                entry.job.next_fire_at.is_none(),
                entry.job.next_fire_at,
                entry.job.id,
            )
        });
        Ok(jobs)
    }

    async fn get(&self, id: Uuid) -> SchedulerResult<Option<JobSnapshot>> {
        Ok(self.inner.read().jobs.get(&id).cloned())
    }

    async fn list_due(&self, now_ms: i64) -> SchedulerResult<Vec<JobSnapshot>> {
        let mut jobs: Vec<_> = self
            .inner
            .read()
            .jobs
            .values()
            .filter(|entry| {
                !entry.job.paused
                    && !entry.job.completed
                    && entry.available(now_ms)
                    && (entry.claim_key.is_some()
                        || entry
                            .job
                            .retry_at
                            .or_else(|| {
                                entry
                                    .job
                                    .pending_delivery
                                    .is_none()
                                    .then_some(entry.job.next_fire_at)
                                    .flatten()
                            })
                            .is_some_and(|time| time.timestamp_millis() <= now_ms))
            })
            .cloned()
            .collect();
        jobs.sort_by_key(|entry| (entry.job.retry_at.or(entry.job.next_fire_at), entry.job.id));
        Ok(jobs)
    }

    async fn replace(&self, expected: &JobSnapshot, job: ScheduledJob) -> SchedulerResult<bool> {
        let json = encode_update(expected, &job)?;
        let mut state = self.inner.write();
        let Some(current) = state
            .jobs
            .get(&job.id)
            .filter(|entry| entry.matches(expected))
        else {
            return Ok(false);
        };
        let next = JobSnapshot {
            job,
            json,
            claim_key: current.claim_key.clone(),
            renewed_at_ms: current.renewed_at_ms,
        };
        state.save(expected, next);
        Ok(true)
    }

    async fn claim(
        &self,
        expected: &JobSnapshot,
        job: ScheduledJob,
        claim_key: String,
        now_ms: i64,
    ) -> SchedulerResult<bool> {
        let json = encode_update(expected, &job)?;
        let mut state = self.inner.write();
        if !state
            .jobs
            .get(&job.id)
            .is_some_and(|entry| entry.matches(expected) && entry.available(now_ms))
        {
            return Ok(false);
        }
        state.jobs.insert(
            job.id,
            JobSnapshot {
                job,
                json,
                claim_key: Some(claim_key),
                renewed_at_ms: Some(now_ms),
            },
        );
        Ok(true)
    }

    async fn renew(&self, id: Uuid, claim_key: &str, now_ms: i64) -> SchedulerResult<bool> {
        let mut state = self.inner.write();
        let Some(entry) = state.jobs.get_mut(&id) else {
            return Ok(false);
        };
        if entry.claim_key() != Some(claim_key) || entry.available(now_ms) {
            return Ok(false);
        }
        entry.renewed_at_ms = Some(entry.renewed_at_ms.unwrap_or(now_ms).max(now_ms));
        Ok(true)
    }

    async fn finish(
        &self,
        expected: &JobSnapshot,
        claim_key: &str,
        job: ScheduledJob,
        now_ms: i64,
    ) -> SchedulerResult<bool> {
        let json = encode_update(expected, &job)?;
        let mut state = self.inner.write();
        if !state.jobs.get(&job.id).is_some_and(|entry| {
            entry.matches(expected)
                && entry.claim_key() == Some(claim_key)
                && !entry.available(now_ms)
        }) {
            return Ok(false);
        }
        state.save(
            expected,
            JobSnapshot {
                job,
                json,
                claim_key: None,
                renewed_at_ms: None,
            },
        );
        Ok(true)
    }

    async fn append_history(&self, entry: SchedulerHistoryEntry) -> SchedulerResult<()> {
        self.inner.write().append_history(entry);
        Ok(())
    }

    async fn list_history(
        &self,
        job_id: Option<Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>> {
        Ok(self
            .inner
            .read()
            .history
            .iter()
            .rev()
            .filter(|entry| job_id.is_none_or(|id| entry.job_id == id))
            .take(limit.clamp(1, MAX_RETAINED_HISTORY_ENTRIES))
            .cloned()
            .collect())
    }
}

#[derive(Clone)]
pub struct SqliteJobStore {
    db: DatabaseConnection,
}

impl SqliteJobStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    fn decode(row: &sea_orm::QueryResult) -> SchedulerResult<JobSnapshot> {
        let json: String = row.try_get("", "job_json")?;
        Ok(JobSnapshot {
            job: serde_json::from_str(&json)?,
            json,
            claim_key: row.try_get("", "delivery_key")?,
            renewed_at_ms: row.try_get("", "claimed_at_ms")?,
        })
    }

    fn job_values(job: &ScheduledJob, json: String) -> Vec<sea_orm::Value> {
        vec![
            json.into(),
            job.next_fire_at.map(|time| time.timestamp_millis()).into(),
            job.retry_at.map(|time| time.timestamp_millis()).into(),
            i64::from(job.paused).into(),
            i64::from(job.completed).into(),
            chrono::Utc::now().timestamp_millis().into(),
        ]
    }

    async fn write_history(
        db: &impl ConnectionTrait,
        entry: &SchedulerHistoryEntry,
    ) -> SchedulerResult<()> {
        let mut record = serde_json::to_value(&entry.record)?;
        record["owner_workspace"] = serde_json::json!(entry.owner_workspace);
        record["owner_session_id"] = serde_json::json!(entry.owner_session_id);
        db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO agena_scheduler_history (job_id, run_json, finished_at_ms) VALUES (?, ?, ?)",
            [entry.job_id.to_string().into(), serde_json::to_string(&record)?.into(), entry.record.finished_at.timestamp_millis().into()],
        )).await?;
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "DELETE FROM agena_scheduler_history \
             WHERE id IN (SELECT id FROM agena_scheduler_history \
                          ORDER BY finished_at_ms ASC, id ASC LIMIT \
                            (SELECT MAX(0, COUNT(*) - ?) FROM agena_scheduler_history))",
            [(MAX_RETAINED_HISTORY_ENTRIES as i64).into()],
        ))
        .await?;
        Ok(())
    }

    async fn update_with_history(
        &self,
        statement: Statement,
        expected: &JobSnapshot,
        job: &ScheduledJob,
    ) -> SchedulerResult<bool> {
        let txn = self.db.begin().await?;
        let changed = txn.execute(statement).await?.rows_affected() > 0;
        if changed && let Some(entry) = new_history(expected, job) {
            Self::write_history(&txn, &entry).await?;
        }
        // Any statement failure returns early, dropping/rolling back the
        // transaction. Never commit an update without its audit record.
        txn.commit().await?;
        Ok(changed)
    }
}

#[async_trait::async_trait]
impl JobStore for SqliteJobStore {
    async fn put(&self, job: ScheduledJob) -> SchedulerResult<()> {
        let mut values = Self::job_values(&job, serde_json::to_string(&job)?);
        values.push(job.id.to_string().into());
        let result = self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO agena_scheduler_jobs (job_json, next_fire_at_ms, retry_at_ms, paused, completed, updated_at_ms, id) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING", values,
        )).await?;
        if result.rows_affected() == 0 {
            return Err(SchedulerError::Conflict(job.id));
        }
        Ok(())
    }

    async fn remove_checked(&self, expected: &JobSnapshot) -> SchedulerResult<bool> {
        Ok(self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "DELETE FROM agena_scheduler_jobs WHERE id = ? AND job_json = ? AND delivery_key IS ?",
            [expected.job.id.to_string().into(), expected.json.clone().into(), expected.claim_key.clone().into()],
        )).await?.rows_affected() > 0)
    }

    async fn remove(&self, id: Uuid) -> SchedulerResult<bool> {
        Ok(self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "DELETE FROM agena_scheduler_jobs WHERE id = ?",
                [id.to_string().into()],
            ))
            .await?
            .rows_affected()
            > 0)
    }

    async fn list(&self) -> SchedulerResult<Vec<JobSnapshot>> {
        self.db.query_all(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs ORDER BY next_fire_at_ms IS NULL, next_fire_at_ms, id",
        )).await?.iter().map(Self::decode).collect()
    }

    async fn get(&self, id: Uuid) -> SchedulerResult<Option<JobSnapshot>> {
        self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE id = ?", [id.to_string().into()],
        )).await?.as_ref().map(Self::decode).transpose()
    }

    async fn list_due(&self, now_ms: i64) -> SchedulerResult<Vec<JobSnapshot>> {
        self.db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs \
             WHERE paused = 0 AND completed = 0 AND ( \
               delivery_key IS NULL AND (retry_at_ms IS NOT NULL AND retry_at_ms <= ? \
                 OR retry_at_ms IS NULL AND next_fire_at_ms IS NOT NULL AND next_fire_at_ms <= ?) \
               OR delivery_key IS NOT NULL AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?)) \
             ORDER BY COALESCE(retry_at_ms, next_fire_at_ms), id",
                [now_ms.into(), now_ms.into(), lease_cutoff(now_ms).into()],
            ))
            .await?
            .iter()
            .map(Self::decode)
            .collect()
    }

    async fn replace(&self, expected: &JobSnapshot, job: ScheduledJob) -> SchedulerResult<bool> {
        let mut values = Self::job_values(&job, encode_update(expected, &job)?);
        values.extend([
            job.id.to_string().into(),
            expected.json.clone().into(),
            expected.claim_key.clone().into(),
        ]);
        self.update_with_history(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_scheduler_jobs SET job_json = ?, next_fire_at_ms = ?, retry_at_ms = ?, paused = ?, completed = ?, updated_at_ms = ? \
             WHERE id = ? AND job_json = ? AND delivery_key IS ?", values,
        ), expected, &job).await
    }

    async fn claim(
        &self,
        expected: &JobSnapshot,
        job: ScheduledJob,
        claim_key: String,
        now_ms: i64,
    ) -> SchedulerResult<bool> {
        let mut values = Self::job_values(&job, encode_update(expected, &job)?);
        values.extend([
            claim_key.into(),
            now_ms.into(),
            job.id.to_string().into(),
            expected.json.clone().into(),
            expected.claim_key.clone().into(),
            lease_cutoff(now_ms).into(),
        ]);
        Ok(self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_scheduler_jobs SET job_json = ?, next_fire_at_ms = ?, retry_at_ms = ?, paused = ?, completed = ?, updated_at_ms = ?, delivery_key = ?, claimed_at_ms = ? \
             WHERE id = ? AND job_json = ? AND delivery_key IS ? \
               AND (delivery_key IS NULL OR claimed_at_ms IS NULL OR claimed_at_ms <= ?)", values,
        )).await?.rows_affected() > 0)
    }

    async fn renew(&self, id: Uuid, claim_key: &str, now_ms: i64) -> SchedulerResult<bool> {
        Ok(self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_scheduler_jobs SET claimed_at_ms = MAX(claimed_at_ms, ?) \
             WHERE id = ? AND delivery_key = ? AND claimed_at_ms > ?",
                [
                    now_ms.into(),
                    id.to_string().into(),
                    claim_key.into(),
                    lease_cutoff(now_ms).into(),
                ],
            ))
            .await?
            .rows_affected()
            > 0)
    }

    async fn finish(
        &self,
        expected: &JobSnapshot,
        claim_key: &str,
        job: ScheduledJob,
        now_ms: i64,
    ) -> SchedulerResult<bool> {
        if expected.claim_key() != Some(claim_key) {
            return Ok(false);
        }
        let mut values = Self::job_values(&job, encode_update(expected, &job)?);
        values.extend([
            job.id.to_string().into(),
            expected.json.clone().into(),
            claim_key.into(),
            lease_cutoff(now_ms).into(),
        ]);
        self.update_with_history(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_scheduler_jobs SET job_json = ?, next_fire_at_ms = ?, retry_at_ms = ?, paused = ?, completed = ?, updated_at_ms = ?, delivery_key = NULL, claimed_at_ms = NULL \
             WHERE id = ? AND job_json = ? AND delivery_key = ? AND claimed_at_ms > ?", values,
        ), expected, &job).await
    }

    async fn append_history(&self, entry: SchedulerHistoryEntry) -> SchedulerResult<()> {
        let txn = self.db.begin().await?;
        Self::write_history(&txn, &entry).await?;
        txn.commit().await?;
        Ok(())
    }

    async fn list_history(
        &self,
        job_id: Option<Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>> {
        let limit = limit.clamp(1, MAX_RETAINED_HISTORY_ENTRIES) as i64;
        let (sql, values) = if let Some(job_id) = job_id {
            (
                "SELECT job_id, run_json FROM agena_scheduler_history WHERE job_id = ? ORDER BY finished_at_ms DESC, id DESC LIMIT ?",
                vec![job_id.to_string().into(), limit.into()],
            )
        } else {
            (
                "SELECT job_id, run_json FROM agena_scheduler_history ORDER BY finished_at_ms DESC, id DESC LIMIT ?",
                vec![limit.into()],
            )
        };
        self.db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?
            .iter()
            .map(|row| {
                let id: String = row.try_get("", "job_id")?;
                let job_id = id.parse().map_err(|error| {
                    SchedulerError::Persistence(sea_orm::DbErr::Custom(format!(
                        "invalid scheduler history job id: {error}"
                    )))
                })?;
                let json: String = row.try_get("", "run_json")?;
                let value: serde_json::Value = serde_json::from_str(&json)?;
                Ok(SchedulerHistoryEntry {
                    job_id,
                    owner_workspace: value["owner_workspace"].as_str().map(str::to_owned),
                    owner_session_id: value["owner_session_id"].as_i64(),
                    record: serde_json::from_value(value)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
