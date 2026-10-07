//! The `Scheduler` runtime loop.

mod changes;
mod owned;
mod worker;

use changes::SchedulerChanges;
pub use changes::{SchedulerChange, SchedulerChangeObserver};
use worker::SchedulerWorker;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::{SchedulerError, SchedulerResult};
use crate::job::{
    JobDeliveryAttempt, JobDeliveryResult, JobSink, ScheduledJob, SchedulerHistoryEntry,
};
use crate::store::{CLAIM_HEARTBEAT, JobSnapshot, JobStore};

/// Background scheduler that fires jobs on their schedule.
pub struct Scheduler {
    store: Arc<dyn JobStore>,
    sink: Arc<dyn JobSink>,
    tick: Duration,
    changes: Arc<SchedulerChanges>,
    handle: parking_lot::Mutex<Option<RunningScheduler>>,
}

struct RunningScheduler {
    stop: CancellationToken,
    handle: JoinHandle<()>,
}

impl Scheduler {
    /// `tick` bounds low-frequency safety reconciliation. Ordinary changes
    /// wake immediately, and job deadlines are independent of this interval.
    pub fn new(store: Arc<dyn JobStore>, sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Self> {
        Arc::new(Self {
            store,
            sink,
            tick,
            changes: Arc::new(SchedulerChanges::default()),
            handle: parking_lot::Mutex::new(None),
        })
    }

    /// Spawn the background task; idempotent while it is running. A finished
    /// task can be started again with a fresh cancellation token.
    pub fn start(self: &Arc<Self>) {
        let mut g = self.handle.lock();
        if g.as_ref()
            .is_some_and(|running| !running.handle.is_finished())
        {
            return;
        }
        let worker = SchedulerWorker {
            store: Arc::clone(&self.store),
            sink: Arc::clone(&self.sink),
            tick: self.tick,
            changes: Arc::clone(&self.changes),
        };
        let stop = CancellationToken::new();
        *g = Some(RunningScheduler {
            stop: stop.clone(),
            handle: tokio::spawn(worker.run_loop(stop)),
        });
    }

    /// Request a graceful stop. Admitted deliveries are finalized before the
    /// task exits; no later delivery is admitted. `start` can launch
    /// another loop after this task has exited.
    pub fn stop(&self) {
        if let Some(running) = self.handle.lock().as_ref() {
            running.stop.cancel();
        }
    }

    /// Install the runtime projection before starting the scheduler.
    pub fn set_change_observer(&self, observer: SchedulerChangeObserver) {
        self.changes.set_observer(observer);
    }

    pub async fn add(&self, job: ScheduledJob) -> SchedulerResult<()> {
        let _commit = self.changes.commit.lock().await;
        self.store.put(job.clone()).await?;
        self.changes.publish(SchedulerChange::Upsert(job), true);
        Ok(())
    }

    pub async fn remove(&self, id: uuid::Uuid) -> SchedulerResult<bool> {
        let _commit = self.changes.commit.lock().await;
        let Some(expected) = self.store.get(id).await? else {
            return Ok(false);
        };
        if !self.store.remove_checked(&expected).await? {
            return Err(SchedulerError::Conflict(id));
        }
        self.changes
            .publish(SchedulerChange::Removed(expected.job), true);
        Ok(true)
    }

    pub async fn list(&self) -> SchedulerResult<Vec<ScheduledJob>> {
        Ok(self
            .store
            .list()
            .await?
            .into_iter()
            .map(|snapshot| snapshot.job)
            .collect())
    }

    pub async fn list_filtered(
        &self,
        session_id: Option<i64>,
        active_only: bool,
    ) -> SchedulerResult<Vec<ScheduledJob>> {
        Ok(self
            .store
            .list_filtered(session_id, active_only)
            .await?
            .into_iter()
            .map(|snapshot| snapshot.job)
            .collect())
    }

    /// Exact session-scoped snapshots, including claims not yet acknowledged by the sink.
    pub async fn pending_jobs_for_session(
        &self,
        session_id: i64,
    ) -> SchedulerResult<Vec<ScheduledJob>> {
        self.store.pending_jobs_for_session(session_id).await
    }

    /// Pending session-owned schedules, including future fires and claimed deliveries.
    pub async fn session_has_pending_jobs(&self, session_id: i64) -> SchedulerResult<bool> {
        self.store.session_has_pending_jobs(session_id).await
    }

    pub async fn get(&self, id: uuid::Uuid) -> SchedulerResult<Option<ScheduledJob>> {
        Ok(self.store.get(id).await?.map(|snapshot| snapshot.job))
    }

    /// Bounded global audit history, newest first, retained after job deletion.
    pub async fn history(
        &self,
        job_id: Option<uuid::Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>> {
        self.store.list_history(job_id, limit).await
    }

    async fn persist_edit(
        &self,
        expected: &JobSnapshot,
        job: ScheduledJob,
    ) -> SchedulerResult<ScheduledJob> {
        let _commit = self.changes.commit.lock().await;
        if !self.store.replace(expected, job.clone()).await? {
            return Err(SchedulerError::Conflict(job.id));
        }
        self.changes
            .publish(SchedulerChange::Upsert(job.clone()), true);
        Ok(job)
    }

    pub async fn pause(&self, id: uuid::Uuid) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self.store.get(id).await? else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        if job.pause() {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }

    pub async fn resume(&self, id: uuid::Uuid) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self.store.get(id).await? else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        let changed = if expected.claim_key().is_some() && job.paused && !job.completed {
            // A running (or abandoned) occurrence keeps its pending delivery.
            // Clearing it would invalidate completion or lose crash recovery.
            job.paused = false;
            true
        } else {
            job.resume(Utc::now())?
        };
        if changed {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }

    pub async fn update(
        &self,
        id: uuid::Uuid,
        prompt: Option<String>,
        expression: Option<String>,
        max_age_days: Option<u32>,
        misfire_policy: Option<crate::job::MisfirePolicy>,
        retry_policy: Option<crate::job::RetryPolicy>,
    ) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self.store.get(id).await? else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        if job.update(
            prompt,
            expression,
            max_age_days,
            misfire_policy,
            retry_policy,
            Utc::now(),
        )? {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        if let Some(running) = self.handle.get_mut().take() {
            running.stop.cancel();
            running.handle.abort();
        }
    }
}

/// Convenience constructor: scheduler + in-memory store + custom sink.
pub fn build_in_memory(sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Scheduler> {
    let store = Arc::new(crate::store::InMemoryJobStore::new()) as Arc<dyn JobStore>;
    Scheduler::new(store, sink, tick)
}

/// Runtime constructor backed by the dedicated scheduler SQLite connection.
/// When no scheduler database is available the scheduler degrades to the
/// in-memory store: scheduling still works, but jobs are not durable across
/// process restarts.
pub fn build_persistent(
    database: Option<std::sync::Arc<sea_orm::DatabaseConnection>>,
    sink: Arc<dyn JobSink>,
    tick: Duration,
) -> Arc<Scheduler> {
    let store: Arc<dyn JobStore> = match database {
        Some(database) => Arc::new(crate::store::SqliteJobStore::new(database.as_ref().clone())),
        None => Arc::new(crate::store::InMemoryJobStore::new()),
    };
    Scheduler::new(store, sink, tick)
}

// Returning an SchedulerResult helper for callers that want to bubble
// scheduler errors up consistently.
pub fn must<T>(r: SchedulerResult<T>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("scheduler invariant violated: {e}"),
    }
}

#[cfg(test)]
mod tests;
