use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::RwLock;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use uuid::Uuid;

use crate::job::{ScheduledJob, SchedulerHistoryEntry};

/// The global audit ledger is deliberately bounded independently from the
/// 50-record working history embedded in every retained job.  This protects
/// the shared SQLite database from unbounded scheduler growth while retaining
/// a useful cross-job, post-deletion diagnostic/export window.
pub const MAX_RETAINED_HISTORY_ENTRIES: usize = 1_000;

#[async_trait::async_trait]
pub trait JobStore: Send + Sync {
    async fn put(&self, job: ScheduledJob);
    async fn remove(&self, id: Uuid) -> bool;
    async fn list(&self) -> Vec<ScheduledJob>;
    async fn get(&self, id: Uuid) -> Option<ScheduledJob>;
    /// Optimistically replace `id`'s row only if it still holds
    /// `expected_next_fire_at_ms` (in milliseconds, as read by the caller).
    /// Returns `false` when the row was changed concurrently, so a caller can
    /// skip work another process already claimed.
    async fn replace(
        &self,
        id: Uuid,
        expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
    ) -> bool;
    /// Atomically claim a due job: mark it as claimed (via `delivery_key`) so
    /// no other process can also claim it. The claim only succeeds when the
    /// row is unclaimed (`delivery_key IS NULL`) and still due
    /// (`next_fire_at_ms` matches). Returns `false` when another process
    /// already claimed it or its schedule changed.
    async fn claim(
        &self,
        id: Uuid,
        expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
        delivery_key: String,
        claimed_at_ms: i64,
    ) -> bool;
    /// Atomically finish (or requeue) a claimed job: clear its `delivery_key`
    /// and set the next fire time. Returns `false` when the row is no longer
    /// held by `delivery_key` (a stale finalize from a crashed attempt).
    async fn finish(
        &self,
        id: Uuid,
        delivery_key: String,
        job: ScheduledJob,
        next_fire_at_ms: Option<i64>,
    ) -> bool;
    async fn append_history(&self, entry: SchedulerHistoryEntry);
    async fn list_history(&self, job_id: Option<Uuid>, limit: usize) -> Vec<SchedulerHistoryEntry>;
}

#[derive(Default, Clone)]
pub struct InMemoryJobStore {
    inner: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    history: Arc<RwLock<VecDeque<SchedulerHistoryEntry>>>,
}

/// Durable scheduler store backed by the shared Agena SQLite database. The
/// schema is created by `agena-storage-sqlite::initialize_schema`.
#[derive(Clone)]
pub struct SqliteJobStore {
    db: DatabaseConnection,
}

impl SqliteJobStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    async fn upsert(&self, job: &ScheduledJob) -> Result<(), sea_orm::DbErr> {
        let json = serde_json::to_string(job)
            .map_err(|error| sea_orm::DbErr::Custom(format!("serialize scheduled job: {error}")))?;
        let next_fire_at_ms = job.next_fire_at.map(|value| value.timestamp_millis());
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO agena_scheduler_jobs (id, job_json, next_fire_at_ms, updated_at_ms) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET job_json = excluded.job_json, next_fire_at_ms = excluded.next_fire_at_ms, updated_at_ms = excluded.updated_at_ms",
                [
                    job.id.to_string().into(),
                    json.into(),
                    next_fire_at_ms.into(),
                    chrono::Utc::now().timestamp_millis().into(),
                ],
            ))
            .await?;
        Ok(())
    }

    fn decode(row: &sea_orm::QueryResult) -> Option<ScheduledJob> {
        let json = match row.try_get::<String>("", "job_json") {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!(target: "agena_scheduler::store", %error, "invalid scheduler row");
                return None;
            }
        };
        match serde_json::from_str(&json) {
            Ok(job) => Some(job),
            Err(error) => {
                tracing::warn!(target: "agena_scheduler::store", %error, "invalid scheduler job JSON");
                None
            }
        }
    }
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl JobStore for InMemoryJobStore {
    async fn put(&self, job: ScheduledJob) {
        self.inner.write().insert(job.id, job);
    }
    async fn remove(&self, id: Uuid) -> bool {
        self.inner.write().remove(&id).is_some()
    }
    async fn list(&self) -> Vec<ScheduledJob> {
        self.inner.read().values().cloned().collect()
    }
    async fn get(&self, id: Uuid) -> Option<ScheduledJob> {
        self.inner.read().get(&id).cloned()
    }
    async fn replace(
        &self,
        id: Uuid,
        _expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
    ) -> bool {
        let mut g = self.inner.write();
        if !g.contains_key(&id) {
            return false;
        }
        g.insert(id, job);
        true
    }

    async fn claim(
        &self,
        id: Uuid,
        _expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
        _delivery_key: String,
        _claimed_at_ms: i64,
    ) -> bool {
        let mut g = self.inner.write();
        let Some(existing) = g.get(&id) else {
            return false;
        };
        if existing.pending_delivery.is_some() {
            return false;
        }
        g.insert(id, job);
        true
    }

    async fn finish(
        &self,
        id: Uuid,
        _delivery_key: String,
        job: ScheduledJob,
        _next_fire_at_ms: Option<i64>,
    ) -> bool {
        let mut g = self.inner.write();
        if !g.contains_key(&id) {
            return false;
        }
        g.insert(id, job);
        true
    }

    async fn append_history(&self, entry: SchedulerHistoryEntry) {
        let mut history = self.history.write();
        history.push_back(entry);
        while history.len() > MAX_RETAINED_HISTORY_ENTRIES {
            history.pop_front();
        }
    }

    async fn list_history(&self, job_id: Option<Uuid>, limit: usize) -> Vec<SchedulerHistoryEntry> {
        history_from_iter(self.history.read().iter(), job_id, limit)
    }
}

#[async_trait::async_trait]
impl JobStore for SqliteJobStore {
    async fn put(&self, job: ScheduledJob) {
        if let Err(error) = self.upsert(&job).await {
            tracing::error!(target: "agena_scheduler::store", job_id = %job.id, %error, "failed to persist scheduled job");
        }
    }

    async fn remove(&self, id: Uuid) -> bool {
        match self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "DELETE FROM agena_scheduler_jobs WHERE id = ?",
                [id.to_string().into()],
            ))
            .await
        {
            Ok(result) => result.rows_affected() > 0,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to delete scheduled job");
                false
            }
        }
    }

    async fn list(&self) -> Vec<ScheduledJob> {
        match self
            .db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT job_json FROM agena_scheduler_jobs ORDER BY next_fire_at_ms IS NULL, next_fire_at_ms, id".to_string(),
            ))
            .await
        {
            Ok(rows) => rows.iter().filter_map(Self::decode).collect(),
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", %error, "failed to list scheduled jobs");
                Vec::new()
            }
        }
    }

    async fn get(&self, id: Uuid) -> Option<ScheduledJob> {
        match self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT job_json FROM agena_scheduler_jobs WHERE id = ?",
                [id.to_string().into()],
            ))
            .await
        {
            Ok(row) => row.as_ref().and_then(Self::decode),
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to read scheduled job");
                None
            }
        }
    }

    async fn replace(
        &self,
        id: Uuid,
        expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
    ) -> bool {
        let json = match serde_json::to_string(&job) {
            Ok(json) => json,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to serialize scheduled job for replace");
                return false;
            }
        };
        let next_fire_at_ms = job.next_fire_at.map(|value| value.timestamp_millis());
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_scheduler_jobs \
                 SET job_json = ?, next_fire_at_ms = ?, updated_at_ms = ? \
                 WHERE id = ? AND next_fire_at_ms IS ?",
                [
                    json.into(),
                    next_fire_at_ms.into(),
                    chrono::Utc::now().timestamp_millis().into(),
                    id.to_string().into(),
                    expected_next_fire_at_ms.into(),
                ],
            ))
            .await;
        match result {
            Ok(result) => result.rows_affected() > 0,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to replace scheduled job");
                false
            }
        }
    }

    async fn claim(
        &self,
        id: Uuid,
        expected_next_fire_at_ms: Option<i64>,
        job: ScheduledJob,
        delivery_key: String,
        claimed_at_ms: i64,
    ) -> bool {
        let json = match serde_json::to_string(&job) {
            Ok(json) => json,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to serialize scheduled job for claim");
                return false;
            }
        };
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_scheduler_jobs \
                 SET job_json = ?, next_fire_at_ms = NULL, delivery_key = ?, claimed_at_ms = ?, updated_at_ms = ? \
                 WHERE id = ? AND delivery_key IS NULL AND next_fire_at_ms IS ?",
                [
                    json.into(),
                    delivery_key.into(),
                    claimed_at_ms.into(),
                    chrono::Utc::now().timestamp_millis().into(),
                    id.to_string().into(),
                    expected_next_fire_at_ms.into(),
                ],
            ))
            .await;
        match result {
            Ok(result) => result.rows_affected() > 0,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to claim scheduled job");
                false
            }
        }
    }

    async fn finish(
        &self,
        id: Uuid,
        delivery_key: String,
        job: ScheduledJob,
        next_fire_at_ms: Option<i64>,
    ) -> bool {
        let json = match serde_json::to_string(&job) {
            Ok(json) => json,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to serialize scheduled job for finish");
                return false;
            }
        };
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_scheduler_jobs \
                 SET job_json = ?, next_fire_at_ms = ?, delivery_key = NULL, claimed_at_ms = NULL, updated_at_ms = ? \
                 WHERE id = ? AND delivery_key = ?",
                [
                    json.into(),
                    next_fire_at_ms.into(),
                    chrono::Utc::now().timestamp_millis().into(),
                    id.to_string().into(),
                    delivery_key.into(),
                ],
            ))
            .await;
        match result {
            Ok(result) => result.rows_affected() > 0,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %id, %error, "failed to finalize scheduled job");
                false
            }
        }
    }

    async fn append_history(&self, entry: SchedulerHistoryEntry) {
        let json = match serde_json::to_string(&entry.record) {
            Ok(json) => json,
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", job_id = %entry.job_id, %error, "failed to serialize scheduler history entry");
                return;
            }
        };
        let finished_at_ms = entry.record.finished_at.timestamp_millis();
        if let Err(error) = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO agena_scheduler_history (job_id, run_json, finished_at_ms) VALUES (?, ?, ?)",
                [entry.job_id.to_string().into(), json.into(), finished_at_ms.into()],
            ))
            .await
        {
            tracing::error!(target: "agena_scheduler::store", job_id = %entry.job_id, %error, "failed to persist scheduler history entry");
            return;
        }
        if let Err(error) = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "DELETE FROM agena_scheduler_history WHERE id NOT IN (SELECT id FROM agena_scheduler_history ORDER BY finished_at_ms DESC, id DESC LIMIT ?)",
                [((MAX_RETAINED_HISTORY_ENTRIES as i64).into())],
            ))
            .await
        {
            tracing::error!(target: "agena_scheduler::store", %error, "failed to prune scheduler history ledger");
        }
    }

    async fn list_history(&self, job_id: Option<Uuid>, limit: usize) -> Vec<SchedulerHistoryEntry> {
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
        match self
            .db
            .query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values))
            .await
        {
            Ok(rows) => rows
                .iter()
                .filter_map(|row| {
                    let job_id = row.try_get::<String>("", "job_id").ok()?.parse().ok()?;
                    let json = row.try_get::<String>("", "run_json").ok()?;
                    match serde_json::from_str(&json) {
                        Ok(record) => Some(SchedulerHistoryEntry { job_id, record }),
                        Err(error) => {
                            tracing::warn!(target: "agena_scheduler::store", %error, "invalid scheduler history JSON");
                            None
                        }
                    }
                })
                .collect(),
            Err(error) => {
                tracing::error!(target: "agena_scheduler::store", %error, "failed to list scheduler history ledger");
                Vec::new()
            }
        }
    }
}

fn history_from_iter<'a>(
    entries: impl DoubleEndedIterator<Item = &'a SchedulerHistoryEntry>,
    job_id: Option<Uuid>,
    limit: usize,
) -> Vec<SchedulerHistoryEntry> {
    entries
        .rev()
        .filter(|entry| job_id.is_none_or(|id| entry.job_id == id))
        .take(limit.clamp(1, MAX_RETAINED_HISTORY_ENTRIES))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

    use super::*;

    #[tokio::test]
    async fn sqlite_store_survives_store_reconstruction() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler table");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler history table");

        let first = SqliteJobStore::new(db.clone());
        let job = ScheduledJob::new_once(Utc::now() + Duration::minutes(5), "verify");
        let id = job.id;
        first.put(job.clone()).await;

        let reconstructed = SqliteJobStore::new(db);
        assert_eq!(
            reconstructed.get(id).await.expect("persisted job").prompt,
            "verify"
        );
        assert_eq!(reconstructed.list().await.len(), 1);

        let mut updated = job;
        let token = updated.next_fire_at.map(|value| value.timestamp_millis());
        updated.prompt = "verify again".to_string();
        assert!(reconstructed.replace(id, token, updated).await);
        assert_eq!(
            reconstructed.get(id).await.expect("updated job").prompt,
            "verify again"
        );
        assert!(reconstructed.remove(id).await);
        assert!(reconstructed.list().await.is_empty());
    }

    #[tokio::test]
    async fn scheduler_wide_history_survives_job_deletion_and_store_reconstruction() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler table");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler history table");

        let store = SqliteJobStore::new(db.clone());
        let job = ScheduledJob::new_once(Utc::now() + Duration::minutes(5), "audit me");
        let job_id = job.id;
        store.put(job).await;
        let now = Utc::now();
        store
            .append_history(SchedulerHistoryEntry {
                job_id,
                record: crate::JobRunRecord {
                    triggered_at: now,
                    finished_at: now,
                    status: crate::JobRunStatus::Submitted,
                    scheduled_for: Some(now),
                    delivery_key: Some("delivery-key".to_string()),
                    attempt: Some(1),
                    session_id: Some(7),
                    failure: None,
                },
            })
            .await;
        assert!(store.remove(job_id).await);

        let reconstructed = SqliteJobStore::new(db);
        let history = reconstructed.list_history(Some(job_id), 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].job_id, job_id);
        assert_eq!(
            history[0].record.delivery_key.as_deref(),
            Some("delivery-key")
        );
    }

    #[tokio::test]
    async fn in_memory_history_retention_is_globally_bounded() {
        let store = InMemoryJobStore::new();
        let job_id = Uuid::new_v4();
        let now = Utc::now();
        for index in 0..=MAX_RETAINED_HISTORY_ENTRIES {
            store
                .append_history(SchedulerHistoryEntry {
                    job_id,
                    record: crate::JobRunRecord {
                        triggered_at: now + Duration::milliseconds(index as i64),
                        finished_at: now + Duration::milliseconds(index as i64),
                        status: crate::JobRunStatus::Submitted,
                        scheduled_for: None,
                        delivery_key: Some(index.to_string()),
                        attempt: Some(1),
                        session_id: None,
                        failure: None,
                    },
                })
                .await;
        }
        let history = store
            .list_history(Some(job_id), MAX_RETAINED_HISTORY_ENTRIES + 10)
            .await;
        assert_eq!(history.len(), MAX_RETAINED_HISTORY_ENTRIES);
        assert_eq!(
            history
                .last()
                .and_then(|entry| entry.record.delivery_key.as_deref()),
            Some("1")
        );
    }

    #[tokio::test]
    async fn sqlite_claim_is_exclusive_across_connections() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler table with delivery key");

        let store_a = SqliteJobStore::new(db.clone());
        let store_b = SqliteJobStore::new(db);
        let mut job = ScheduledJob::new_once(Utc::now() + Duration::minutes(5), "claim");
        job.next_fire_at = Some(Utc::now() - Duration::seconds(1)); // make it due
        let id = job.id;
        store_a.put(job.clone()).await;
        let token = job.next_fire_at.map(|value| value.timestamp_millis());

        // Two processes claim the same due job concurrently. The first wins;
        // the second sees the row already claimed (delivery_key IS NOT NULL).
        let (claim_a, claim_b) = tokio::join!(
            store_a.claim(id, token, job.clone(), "delivery-a".to_owned(), 1),
            store_b.claim(id, token, job.clone(), "delivery-b".to_owned(), 1),
        );
        assert!(claim_a ^ claim_b, "exactly one claim must succeed");
    }

    #[tokio::test]
    async fn sqlite_finish_clears_delivery_and_requeues() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)".to_string(),
        ))
        .await
        .expect("create scheduler table with delivery key");

        let store = SqliteJobStore::new(db);
        let mut job = ScheduledJob::new_once(Utc::now() + Duration::minutes(5), "finish");
        job.next_fire_at = Some(Utc::now() - Duration::seconds(1));
        let id = job.id;
        store.put(job.clone()).await;
        let token = job.next_fire_at.map(|value| value.timestamp_millis());

        assert!(
            store
                .claim(id, token, job.clone(), "delivery-1".to_owned(), 1)
                .await
        );
        // A second claim on the claimed job must fail.
        assert!(
            !store
                .claim(id, None, job.clone(), "delivery-2".to_owned(), 2)
                .await
        );
        // Finish requeues it; the delivery key is cleared.
        let next = job.next_fire_at.map(|value| value.timestamp_millis());
        assert!(
            store
                .finish(id, "delivery-1".to_owned(), job, next)
                .await
        );
        let reloaded = store.get(id).await.expect("reload");
        assert!(reloaded.pending_delivery.is_none());
    }
}
