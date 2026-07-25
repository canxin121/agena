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
    async fn replace(&self, id: Uuid, job: ScheduledJob) -> bool;
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
    async fn replace(&self, id: Uuid, job: ScheduledJob) -> bool {
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

    async fn replace(&self, id: Uuid, job: ScheduledJob) -> bool {
        if self.get(id).await.is_none() {
            return false;
        }
        self.upsert(&job).await.is_ok()
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
        updated.prompt = "verify again".to_string();
        assert!(reconstructed.replace(id, updated).await);
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
                    error_message: None,
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
                        error_message: None,
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
}
