//! Job persistence, optimistic edits, and renewable delivery ownership.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use uuid::Uuid;

use crate::error::{SchedulerError, SchedulerResult};
use crate::job::{ScheduledJob, SchedulerHistoryEntry};

static JOB_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

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
    async fn list_filtered(
        &self,
        session_id: Option<i64>,
        active_only: bool,
    ) -> SchedulerResult<Vec<JobSnapshot>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|entry| {
                session_id.is_none_or(|id| entry.job.owner_session_id == Some(id))
                    && (!active_only || !entry.job.completed)
            })
            .collect())
    }
    async fn pending_jobs_for_session(
        &self,
        session_id: i64,
    ) -> SchedulerResult<Vec<ScheduledJob>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .map(|snapshot| snapshot.job)
            .filter(|job| {
                job.owner_session_id == Some(session_id)
                    && !job.paused
                    && !job.completed
                    && (job.next_fire_at.is_some() || job.pending_delivery.is_some())
            })
            .collect())
    }
    async fn session_has_pending_jobs(&self, session_id: i64) -> SchedulerResult<bool> {
        Ok(!self.pending_jobs_for_session(session_id).await?.is_empty())
    }
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

    async fn list_filtered(
        &self,
        session_id: Option<i64>,
        active_only: bool,
    ) -> SchedulerResult<Vec<JobSnapshot>> {
        let mut jobs: Vec<_> = self
            .inner
            .read()
            .jobs
            .values()
            .filter(|entry| {
                session_id.is_none_or(|id| entry.job.owner_session_id == Some(id))
                    && (!active_only || !entry.job.completed)
            })
            .cloned()
            .collect();
        jobs.sort_by_key(|entry| {
            (
                entry.job.next_fire_at.is_none(),
                entry.job.next_fire_at,
                entry.job.id,
            )
        });
        Ok(jobs)
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
    writer: Arc<tokio::sync::OnceCell<Arc<agena_async::WriteQueue>>>,
}

impl SqliteJobStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            writer: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    async fn write_permit(&self) -> SchedulerResult<agena_async::WritePermit> {
        let queue = self
            .writer
            .get_or_try_init(|| async {
                let row = self
                    .db
                    .query_one(Statement::from_string(
                        DatabaseBackend::Sqlite,
                        "PRAGMA database_list",
                    ))
                    .await?
                    .ok_or_else(|| {
                        sea_orm::DbErr::Custom("SQLite main database is missing".into())
                    })?;
                let file: String = row.try_get("", "file")?;
                if file.is_empty() {
                    Ok::<_, sea_orm::DbErr>(agena_async::WriteQueue::named(
                        format!(
                            "sqlite-memory:{}",
                            self.db
                                .get_sqlite_connection_pool()
                                .connect_options()
                                .get_filename()
                                .display()
                        )
                        .into(),
                    ))
                } else {
                    agena_async::WriteQueue::for_file(std::path::Path::new(&file))
                        .await
                        .map_err(|error| {
                            sea_orm::DbErr::Custom(format!(
                                "resolve scheduler write queue: {error}"
                            ))
                        })
                }
            })
            .await?;
        queue
            .acquire()
            .await
            .map_err(|error| SchedulerError::Persistence(sea_orm::DbErr::Custom(error.to_string())))
    }

    async fn encode_job(job: ScheduledJob) -> SchedulerResult<(ScheduledJob, String)> {
        JOB_CODECS
            .run(move || {
                let json = serde_json::to_string(&job)?;
                Ok((job, json))
            })
            .await
            .map_err(|error| {
                SchedulerError::Persistence(sea_orm::DbErr::Custom(format!(
                    "job encode worker failed: {error}"
                )))
            })?
    }

    async fn encode_job_update(
        expected: &JobSnapshot,
        job: ScheduledJob,
    ) -> SchedulerResult<(ScheduledJob, String)> {
        if expected.job.owner_session_id != job.owner_session_id
            || expected.job.owner_workspace != job.owner_workspace
            || expected.job.id != job.id
        {
            return Err(SchedulerError::InvalidUpdate(
                "job id and ownership are immutable".into(),
            ));
        }
        Self::encode_job(job).await
    }

    async fn decode_rows(rows: Vec<sea_orm::QueryResult>) -> SchedulerResult<Vec<JobSnapshot>> {
        JOB_CODECS
            .run(move || rows.iter().map(Self::decode).collect())
            .await
            .map_err(|error| {
                SchedulerError::Persistence(sea_orm::DbErr::Custom(format!(
                    "job decode worker failed: {error}"
                )))
            })?
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

    async fn prepare_history(entry: SchedulerHistoryEntry) -> SchedulerResult<Statement> {
        JOB_CODECS.run(move || {
            Ok(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
                "INSERT INTO agena_scheduler_history (job_id, owner_workspace, owner_session_id, run_json, finished_at_ms) VALUES (?, ?, ?, ?, ?)",
                [entry.job_id.to_string().into(), entry.owner_workspace.into(), entry.owner_session_id.into(),
                 serde_json::to_string(&entry.record)?.into(), entry.record.finished_at.timestamp_millis().into()],
            ))
        }).await.map_err(|error| SchedulerError::Persistence(sea_orm::DbErr::Custom(format!("history encode worker failed: {error}"))))?
    }

    async fn write_history(db: &impl ConnectionTrait, statement: Statement) -> SchedulerResult<()> {
        db.execute(statement).await?;
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
        let history = match new_history(expected, job) {
            Some(entry) => Some(Self::prepare_history(entry).await?),
            None => None,
        };
        let _permit = self.write_permit().await?;
        let txn = self.db.begin().await?;
        let changed = txn.execute(statement).await?.rows_affected() > 0;
        if changed && let Some(statement) = history {
            Self::write_history(&txn, statement).await?;
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
        let (job, json) = Self::encode_job(job).await?;
        let mut values = Self::job_values(&job, json);
        values.push(job.id.to_string().into());
        let _permit = self.write_permit().await?;
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
        let _permit = self.write_permit().await?;
        Ok(self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "DELETE FROM agena_scheduler_jobs WHERE id = ? AND job_json = ? AND delivery_key IS ?",
            [expected.job.id.to_string().into(), expected.json.clone().into(), expected.claim_key.clone().into()],
        )).await?.rows_affected() > 0)
    }

    async fn remove(&self, id: Uuid) -> SchedulerResult<bool> {
        let _permit = self.write_permit().await?;
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
        let rows = self.db.query_all(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs ORDER BY next_fire_at_ms IS NULL, next_fire_at_ms, id",
        )).await?;
        Self::decode_rows(rows).await
    }

    async fn list_filtered(
        &self,
        session_id: Option<i64>,
        active_only: bool,
    ) -> SchedulerResult<Vec<JobSnapshot>> {
        // Keep corruption visible. The invalid-JSON partial index makes this
        // check constant-sized on healthy stores.
        if let Some(row) = self.db.query_one(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE NOT json_valid(job_json) LIMIT 1")).await? {
            Self::decode_rows(vec![row]).await?;
        }
        let mut sql = String::from(
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE json_valid(job_json)",
        );
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(id) = session_id {
            sql.push_str(" AND json_extract(job_json, '$.owner_session_id') = ?");
            values.push(id.into());
        }
        if active_only {
            sql.push_str(" AND completed = 0");
        }
        sql.push_str(" ORDER BY next_fire_at_ms IS NULL, next_fire_at_ms, id");
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?;
        Self::decode_rows(rows).await
    }

    async fn pending_jobs_for_session(
        &self,
        session_id: i64,
    ) -> SchedulerResult<Vec<ScheduledJob>> {
        let rows = self.db.query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE json_extract(job_json, '$.owner_session_id') = ? AND paused = 0 AND completed = 0 AND (next_fire_at_ms IS NOT NULL OR json_extract(job_json, '$.pending_delivery') IS NOT NULL) ORDER BY id",
            [session_id.into()],
        )).await?;
        Ok(Self::decode_rows(rows)
            .await?
            .into_iter()
            .map(|snapshot| snapshot.job)
            .collect())
    }

    async fn session_has_pending_jobs(&self, session_id: i64) -> SchedulerResult<bool> {
        let row = self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT EXISTS(SELECT 1 FROM agena_scheduler_jobs WHERE json_extract(job_json, '$.owner_session_id') = ? AND paused = 0 AND completed = 0 AND (next_fire_at_ms IS NOT NULL OR json_extract(job_json, '$.pending_delivery') IS NOT NULL)) AS present",
            [session_id.into()],
        )).await?;
        Ok(row
            .map(|row| row.try_get("", "present"))
            .transpose()?
            .unwrap_or(false))
    }

    async fn get(&self, id: Uuid) -> SchedulerResult<Option<JobSnapshot>> {
        let row = self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE id = ?", [id.to_string().into()],
        )).await?;
        Ok(Self::decode_rows(row.into_iter().collect()).await?.pop())
    }

    async fn list_due(&self, now_ms: i64) -> SchedulerResult<Vec<JobSnapshot>> {
        let rows = self
            .db
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
            .await?;
        Self::decode_rows(rows).await
    }

    async fn replace(&self, expected: &JobSnapshot, job: ScheduledJob) -> SchedulerResult<bool> {
        let (job, json) = Self::encode_job_update(expected, job).await?;
        let mut values = Self::job_values(&job, json);
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
        let (job, json) = Self::encode_job_update(expected, job).await?;
        let mut values = Self::job_values(&job, json);
        values.extend([
            claim_key.into(),
            now_ms.into(),
            job.id.to_string().into(),
            expected.json.clone().into(),
            expected.claim_key.clone().into(),
            lease_cutoff(now_ms).into(),
        ]);
        let _permit = self.write_permit().await?;
        Ok(self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_scheduler_jobs SET job_json = ?, next_fire_at_ms = ?, retry_at_ms = ?, paused = ?, completed = ?, updated_at_ms = ?, delivery_key = ?, claimed_at_ms = ? \
             WHERE id = ? AND job_json = ? AND delivery_key IS ? \
               AND (delivery_key IS NULL OR claimed_at_ms IS NULL OR claimed_at_ms <= ?)", values,
        )).await?.rows_affected() > 0)
    }

    async fn renew(&self, id: Uuid, claim_key: &str, now_ms: i64) -> SchedulerResult<bool> {
        let _permit = self.write_permit().await?;
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
        let (job, json) = Self::encode_job_update(expected, job).await?;
        let mut values = Self::job_values(&job, json);
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
        let statement = Self::prepare_history(entry).await?;
        let _permit = self.write_permit().await?;
        let txn = self.db.begin().await?;
        Self::write_history(&txn, statement).await?;
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
                "SELECT job_id, owner_workspace, owner_session_id, run_json FROM agena_scheduler_history WHERE job_id = ? ORDER BY finished_at_ms DESC, id DESC LIMIT ?",
                vec![job_id.to_string().into(), limit.into()],
            )
        } else {
            (
                "SELECT job_id, owner_workspace, owner_session_id, run_json FROM agena_scheduler_history ORDER BY finished_at_ms DESC, id DESC LIMIT ?",
                vec![limit.into()],
            )
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?;
        JOB_CODECS
            .run(move || {
                rows.iter()
                    .map(|row| {
                        let id: String = row.try_get("", "job_id")?;
                        let job_id = id.parse().map_err(|error| {
                            SchedulerError::Persistence(sea_orm::DbErr::Custom(format!(
                                "invalid scheduler history job id: {error}"
                            )))
                        })?;
                        let json: String = row.try_get("", "run_json")?;
                        Ok(SchedulerHistoryEntry {
                            job_id,
                            owner_workspace: row.try_get("", "owner_workspace")?,
                            owner_session_id: row.try_get("", "owner_session_id")?,
                            record: serde_json::from_str(&json)?,
                        })
                    })
                    .collect()
            })
            .await
            .map_err(|error| {
                SchedulerError::Persistence(sea_orm::DbErr::Custom(format!(
                    "history decode worker failed: {error}"
                )))
            })?
    }
}

#[cfg(test)]
mod tests;
