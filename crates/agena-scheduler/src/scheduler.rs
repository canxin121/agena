//! The `Scheduler` runtime loop.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::SchedulerResult;
use crate::job::{JobDeliveryAttempt, JobOutcome, JobSink, ScheduledJob, SchedulerHistoryEntry};
use crate::store::{JobStore, MAX_RETAINED_HISTORY_ENTRIES};

/// Background scheduler that fires jobs on their schedule.
pub struct Scheduler {
    store: Arc<dyn JobStore>,
    sink: Arc<dyn JobSink>,
    tick: Duration,
    stop: Arc<Notify>,
    handle: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn new(store: Arc<dyn JobStore>, sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Self> {
        Arc::new(Self {
            store,
            sink,
            tick,
            stop: Arc::new(Notify::new()),
            handle: parking_lot::Mutex::new(None),
        })
    }

    /// Spawn the background task; idempotent.
    pub fn start(self: &Arc<Self>) {
        let mut g = self.handle.lock();
        if g.is_some() {
            return;
        }
        let scheduler = Arc::downgrade(self);
        *g = Some(tokio::spawn(async move { Self::run_loop(scheduler).await }));
    }

    pub fn stop(&self) {
        self.stop.notify_waiters();
    }

    pub async fn add(&self, job: ScheduledJob) {
        self.store.put(job).await;
    }

    pub async fn remove(&self, id: uuid::Uuid) -> bool {
        self.store.remove(id).await
    }

    pub async fn list(&self) -> Vec<ScheduledJob> {
        self.store.list().await
    }

    pub async fn get(&self, id: uuid::Uuid) -> Option<ScheduledJob> {
        self.store.get(id).await
    }

    /// Return the globally retained scheduler audit ledger, newest first.
    /// Unlike `ScheduledJob::run_history`, these records survive deletion of
    /// the job that produced them.  The backing store bounds retention, so
    /// callers must treat this as diagnostics/export history rather than an
    /// unbounded event archive.
    ///
    /// The per-job histories are merged as a read-only compatibility fallback
    /// for databases created before the ledger table existed.  Entries already
    /// persisted in the ledger are deterministically deduplicated; after new
    /// deliveries complete the central table is the authoritative copy.
    pub async fn history(
        &self,
        job_id: Option<uuid::Uuid>,
        limit: usize,
    ) -> Vec<SchedulerHistoryEntry> {
        let mut entries = self
            .store
            .list_history(job_id, MAX_RETAINED_HISTORY_ENTRIES)
            .await;
        for job in self.store.list().await {
            if job_id.is_some_and(|id| id != job.id) {
                continue;
            }
            entries.extend(
                job.run_history
                    .into_iter()
                    .map(|record| SchedulerHistoryEntry {
                        job_id: job.id,
                        record,
                    }),
            );
        }
        entries.sort_by_key(|entry| Reverse(entry.record.finished_at));
        let mut seen = HashSet::new();
        entries.retain(|entry| scheduler_history_identity(entry, &mut seen));
        entries.truncate(limit.clamp(1, MAX_RETAINED_HISTORY_ENTRIES));
        entries
    }

    pub async fn pause(&self, id: uuid::Uuid) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(mut job) = self.store.get(id).await else {
            return Ok(None);
        };
        let token = job.next_fire_at.map(|value| value.timestamp_millis());
        if job.pause() {
            self.store.replace(id, token, job.clone()).await;
        }
        Ok(Some(job))
    }

    pub async fn resume(&self, id: uuid::Uuid) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(mut job) = self.store.get(id).await else {
            return Ok(None);
        };
        let token = job.next_fire_at.map(|value| value.timestamp_millis());
        if job.resume(Utc::now())? {
            self.store.replace(id, token, job.clone()).await;
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
        let Some(mut job) = self.store.get(id).await else {
            return Ok(None);
        };
        let token = job.next_fire_at.map(|value| value.timestamp_millis());
        if job.update(
            prompt,
            expression,
            max_age_days,
            misfire_policy,
            retry_policy,
            Utc::now(),
        )? {
            self.store.replace(id, token, job.clone()).await;
        }
        Ok(Some(job))
    }

    async fn run_loop(scheduler: std::sync::Weak<Self>) {
        loop {
            let Some(current) = scheduler.upgrade() else {
                break;
            };
            let due = current.claim_due().await;
            for (mut job, delivery, token, delivery_key) in due {
                let now = Utc::now();
                let result = current.sink.deliver(&job, &delivery).await;
                match job.finish_delivery(now, &delivery, result) {
                    Ok(JobOutcome::Continued) => {
                        current
                            .persist_completed_delivery(job, token, &delivery_key)
                            .await;
                    }
                    Ok(JobOutcome::Expired) => {
                        // Keep terminal jobs, including their bounded run
                        // history, until a user explicitly deletes them.
                        current
                            .persist_completed_delivery(job, token, &delivery_key)
                            .await;
                    }
                    Ok(JobOutcome::RetryScheduled) => {
                        // The failed attempt and retry deadline are durable;
                        // a later poll will reclaim the same delivery key.
                        current
                            .persist_completed_delivery(job, token, &delivery_key)
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "agena_scheduler",
                            "delivery finalization failed for job {}: {e}",
                            job.id
                        );
                        current.store.remove(job.id).await;
                    }
                }
            }
            let tick = current.tick;
            let stop = Arc::clone(&current.stop);
            drop(current);
            tokio::select! {
                _ = tokio::time::sleep(tick) => {}
                _ = stop.notified() => break,
            }
        }
    }

    /// Persist every due-state transition before invoking the delivery sink.
    /// A returned `Deliver` item is therefore already claimed durably; a
    /// process loss before the sink finishes can be recovered as a retry.
    ///
    /// The returned `Option<i64>` is the job's `next_fire_at` (in millis) as
    /// observed before claiming — the optimistic-lock token for the follow-up
    /// `store.claim` so a concurrent process cannot double-claim. The trailing
    /// `String` is the delivery key used to finalize (or requeue) the claim.
    async fn claim_due(&self) -> Vec<(ScheduledJob, JobDeliveryAttempt, Option<i64>, String)> {
        let now = Utc::now();
        let mut deliveries = Vec::new();
        for mut job in self.store.list().await {
            // Capture the pre-claim next_fire_at as the optimistic token.
            let token = job.next_fire_at.map(|value| value.timestamp_millis());
            if !job.due(now) {
                continue;
            }
            match job.claim_due_delivery(now) {
                Ok(crate::job::ClaimDueDelivery::NotDue) => {}
                Ok(crate::job::ClaimDueDelivery::StateUpdated) => {
                    let record = job.last_run.clone();
                    if self.store.replace(job.id, token, job.clone()).await {
                        if let Some(record) = record {
                            self.store
                                .append_history(SchedulerHistoryEntry {
                                    job_id: job.id,
                                    record,
                                })
                                .await;
                        }
                    } else {
                        tracing::warn!(
                            target: "agena_scheduler",
                            "job disappeared while recording scheduler state transition"
                        );
                    }
                }
                Ok(crate::job::ClaimDueDelivery::Deliver(delivery)) => {
                    let delivery_key = format!("{}:{}", job.id, delivery.attempt);
                    if self
                        .store
                        .claim(
                            job.id,
                            token,
                            job.clone(),
                            delivery_key.clone(),
                            delivery.claimed_at.timestamp_millis(),
                        )
                        .await
                    {
                        deliveries.push((job, delivery, token, delivery_key));
                    } else {
                        tracing::warn!(
                            target: "agena_scheduler",
                            "job was already claimed by another process"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "agena_scheduler",
                        job_id = %job.id,
                        %error,
                        "failed to prepare due scheduled job"
                    );
                }
            }
        }
        deliveries
    }

    /// Preserve a run in the central ledger only after the updated job state
    /// itself has been durably replaced.  That ordering prevents an audit
    /// entry from claiming a completed delivery whose claim/finalization did
    /// not reach the same store.
    async fn persist_completed_delivery(
        &self,
        job: ScheduledJob,
        _token: Option<i64>,
        delivery_key: &str,
    ) {
        let record = job.last_run.clone();
        let id = job.id;
        let next_fire_at_ms = job.next_fire_at.map(|value| value.timestamp_millis());
        if !self
            .store
            .finish(id, delivery_key.to_owned(), job, next_fire_at_ms)
            .await
        {
            tracing::warn!(
                target: "agena_scheduler",
                job_id = %id,
                "job disappeared while persisting scheduler delivery finalization"
            );
            return;
        }
        if let Some(record) = record {
            self.store
                .append_history(SchedulerHistoryEntry { job_id: id, record })
                .await;
        }
    }
}

fn scheduler_history_identity(entry: &SchedulerHistoryEntry, seen: &mut HashSet<String>) -> bool {
    let key = format!(
        "{}|{}|{}|{}|{:?}",
        entry.job_id,
        entry
            .record
            .triggered_at
            .timestamp_nanos_opt()
            .unwrap_or_default(),
        entry
            .record
            .finished_at
            .timestamp_nanos_opt()
            .unwrap_or_default(),
        entry.record.delivery_key.as_deref().unwrap_or_default(),
        entry.record.status,
    );
    seen.insert(key)
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.stop.notify_waiters();
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

/// Convenience constructor: scheduler + in-memory store + custom sink.
pub fn build_in_memory(sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Scheduler> {
    let store = Arc::new(crate::store::InMemoryJobStore::new()) as Arc<dyn JobStore>;
    Scheduler::new(store, sink, tick)
}

/// Runtime constructor backed by the shared Agena SQLite connection.
pub fn build_persistent(
    database: sea_orm::DatabaseConnection,
    sink: Arc<dyn JobSink>,
    tick: Duration,
) -> Arc<Scheduler> {
    let store = Arc::new(crate::store::SqliteJobStore::new(database)) as Arc<dyn JobStore>;
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
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::{JobDeliveryAttempt, JobDeliveryResult, JobSink, ScheduledJob};

    use super::build_in_memory;

    struct NoopSink;

    #[async_trait::async_trait]
    impl JobSink for NoopSink {
        async fn deliver(
            &self,
            _job: &ScheduledJob,
            _delivery: &JobDeliveryAttempt,
        ) -> JobDeliveryResult {
            JobDeliveryResult::submitted(None)
        }
    }

    #[tokio::test]
    async fn background_loop_does_not_keep_scheduler_alive() {
        let scheduler = build_in_memory(Arc::new(NoopSink), Duration::from_secs(60));
        scheduler.start();
        let weak = Arc::downgrade(&scheduler);

        drop(scheduler);
        tokio::task::yield_now().await;

        assert!(weak.upgrade().is_none());
    }
}
