//! The `Scheduler` runtime loop.

mod owned;

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
    handle: parking_lot::Mutex<Option<RunningScheduler>>,
}

struct RunningScheduler {
    stop: CancellationToken,
    handle: JoinHandle<()>,
}

// Own only the worker's dependencies. Holding an Arc<Scheduler> across a
// delivery would prevent its Drop from ever cancelling a wedged sink.
struct SchedulerWorker {
    store: Arc<dyn JobStore>,
    sink: Arc<dyn JobSink>,
    tick: Duration,
}

impl Scheduler {
    pub fn new(store: Arc<dyn JobStore>, sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Self> {
        Arc::new(Self {
            store,
            sink,
            tick,
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
        };
        let stop = CancellationToken::new();
        *g = Some(RunningScheduler {
            stop: stop.clone(),
            handle: tokio::spawn(worker.run_loop(stop)),
        });
    }

    /// Request a graceful stop. Already claimed deliveries are finalized
    /// before the task exits; no later poll is admitted. `start` can launch
    /// another loop after this task has exited.
    pub fn stop(&self) {
        if let Some(running) = self.handle.lock().as_ref() {
            running.stop.cancel();
        }
    }

    pub async fn add(&self, job: ScheduledJob) -> SchedulerResult<()> {
        self.store.put(job).await
    }

    pub async fn remove(&self, id: uuid::Uuid) -> SchedulerResult<bool> {
        self.store.remove(id).await
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
        if !self.store.replace(expected, job.clone()).await? {
            return Err(SchedulerError::Conflict(job.id));
        }
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

struct ClaimedDelivery {
    job: ScheduledJob,
    delivery: JobDeliveryAttempt,
    claim_key: String,
}

impl SchedulerWorker {
    async fn run_loop(self, stop: CancellationToken) {
        while !stop.is_cancelled() {
            match self.store.list_due(Utc::now().timestamp_millis()).await {
                Ok(candidates) => {
                    for candidate in candidates {
                        if stop.is_cancelled() {
                            break;
                        }
                        let id = candidate.job.id;
                        // Claim just before delivery. Later candidates must not
                        // lose their lease while queued behind a slow sink.
                        let result = match self.claim_candidate(candidate, Utc::now()).await {
                            Ok(Some(claim)) => {
                                self.deliver_with_lease(claim, CLAIM_HEARTBEAT).await
                            }
                            Ok(None) => Ok(()),
                            Err(error) => Err(error),
                        };
                        if let Err(error) = result {
                            // Preserve the durable row. An abandoned claim is
                            // recoverable; deleting it here silently loses work.
                            tracing::error!(target: "agena_scheduler", job_id = %id, %error, "scheduled delivery failed; durable state retained");
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(target: "agena_scheduler", %error, "failed to poll scheduled jobs")
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(self.tick) => {}
                _ = stop.cancelled() => break,
            }
        }
    }

    async fn claim_candidate(
        &self,
        expected: JobSnapshot,
        now: chrono::DateTime<Utc>,
    ) -> SchedulerResult<Option<ClaimedDelivery>> {
        let mut job = expected.job.clone();
        // Expired ownership is selected by the store. A pending delivery
        // deliberately fails ordinary due(), but is recoverable with its key.
        if expected.claim_key().is_none() && !job.due(now) {
            return Ok(None);
        }
        match job.claim_due_delivery(now)? {
            crate::job::ClaimDueDelivery::NotDue => Ok(None),
            crate::job::ClaimDueDelivery::StateUpdated => {
                self.store.replace(&expected, job).await?;
                Ok(None)
            }
            crate::job::ClaimDueDelivery::Deliver(delivery) => {
                let claim_key = uuid::Uuid::new_v4().to_string();
                if self
                    .store
                    .claim(
                        &expected,
                        job.clone(),
                        claim_key.clone(),
                        now.timestamp_millis(),
                    )
                    .await?
                {
                    Ok(Some(ClaimedDelivery {
                        job,
                        delivery,
                        claim_key,
                    }))
                } else {
                    Ok(None) // A concurrent edit or claimant won the comparison.
                }
            }
        }
    }

    async fn deliver_with_lease(
        &self,
        claim: ClaimedDelivery,
        heartbeat: Duration,
    ) -> SchedulerResult<()> {
        let id = claim.job.id;
        if !self
            .store
            .renew(id, &claim.claim_key, Utc::now().timestamp_millis())
            .await?
        {
            return Err(SchedulerError::Conflict(id));
        }
        let work = async {
            let result = self.sink.deliver(&claim.job, &claim.delivery).await;
            self.persist_completed_delivery(&claim, result, Utc::now())
                .await
        };
        let renewals = async {
            loop {
                tokio::time::sleep(heartbeat).await;
                let renewed = tokio::time::timeout(
                    CLAIM_HEARTBEAT,
                    self.store
                        .renew(id, &claim.claim_key, Utc::now().timestamp_millis()),
                )
                .await;
                match renewed {
                    Ok(Ok(true)) => {}
                    Ok(Err(error)) => return Err(error),
                    Ok(Ok(false)) | Err(_) => return Err(SchedulerError::Conflict(id)),
                }
            }
        };
        // Poll both futures while renewal waits for a connection: work may
        // own the very transaction it needs. A finished commit takes priority
        // over a renewal that now sees the deliberately released claim.
        // A lost lease drops work, including any in-flight sink future.
        tokio::select! {
            biased;
            result = work => result,
            result = renewals => result,
        }
    }

    async fn persist_completed_delivery(
        &self,
        claim: &ClaimedDelivery,
        result: JobDeliveryResult,
        finished_at: chrono::DateTime<Utc>,
    ) -> SchedulerResult<()> {
        let id = claim.job.id;
        // Merge against current configuration, not the pre-delivery copy.
        // Bounded retries tolerate ordinary pause/update races without making
        // a continuously edited job monopolize the scheduler.
        for _ in 0..8 {
            let Some(expected) = self.store.get(id).await? else {
                return Ok(());
            };
            if expected.claim_key() != Some(claim.claim_key.as_str()) {
                return Err(SchedulerError::Conflict(id));
            }
            let mut job = expected.job.clone();
            job.finish_delivery(finished_at, &claim.delivery, result.clone())?;
            if self
                .store
                .finish(
                    &expected,
                    &claim.claim_key,
                    job,
                    Utc::now().timestamp_millis(),
                )
                .await?
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err(SchedulerError::Conflict(id))
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
