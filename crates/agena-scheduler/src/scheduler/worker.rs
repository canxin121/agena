//! Deadline-driven, bounded delivery admission and lease maintenance.

use super::*;
use crate::store::DueJobQuery;
use std::collections::HashMap;
use tokio::task::{Id, JoinSet};
use tokio::time::Instant;

const MAX_CONCURRENT_DELIVERIES: usize = 4;
const ADMISSION_RETRY_DELAY: Duration = Duration::from_millis(100);
const ERROR_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_DEFERRED_JOBS: usize = 32;

// Keep only worker dependencies: Arc<Scheduler> would prevent Drop from
// cancelling a wedged sink. Dropping the JoinSet aborts every delivery task.
pub(super) struct SchedulerWorker {
    pub(super) store: Arc<dyn JobStore>,
    pub(super) sink: Arc<dyn JobSink>,
    pub(super) tick: Duration,
    pub(super) changes: Arc<SchedulerChanges>,
}

#[derive(Clone, Copy)]
struct Admission {
    id: uuid::Uuid,
    session_id: Option<i64>,
}

enum DeliveryProgress {
    Applied,
    Idle,
}

pub(super) struct ClaimedDelivery {
    pub(super) job: ScheduledJob,
    pub(super) delivery: JobDeliveryAttempt,
    pub(super) claim_key: String,
}

impl SchedulerWorker {
    pub(super) async fn run_loop(self, stop: CancellationToken) {
        let worker = Arc::new(self);
        let mut wake = worker.changes.wake.subscribe();
        let mut deliveries = JoinSet::<SchedulerResult<DeliveryProgress>>::new();
        let mut active = HashMap::<Id, Admission>::new();
        let mut deferred = HashMap::<uuid::Uuid, Instant>::new();
        let mut next_read = Instant::now();
        let mut reads_allowed_at = next_read;
        let mut failures = 0u32;
        // Reconciliation covers other processes and wall-clock corrections;
        // ordinary in-process changes interrupt the wait immediately.
        let reconciliation = worker
            .tick
            .clamp(Duration::from_millis(100), Duration::from_secs(60));

        loop {
            if stop.is_cancelled() {
                break;
            }
            // Coalesce completions that became ready together before doing
            // another store read; refill their slots in one bounded batch.
            while let Some(completion) = deliveries.try_join_next_with_id() {
                record_completion(completion, &mut active, &mut deferred);
                next_read = Instant::now().max(reads_allowed_at);
            }
            deferred.retain(|_, until| *until > Instant::now());
            if active.len() < MAX_CONCURRENT_DELIVERIES && Instant::now() >= next_read {
                // Observe before I/O: a mutation racing either read remains
                // pending, and cannot be swallowed by arming the next wait.
                wake.borrow_and_update();
                let query = admission_query(&active, &deferred);
                let deadline = tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    result = worker.store.next_wake_at_ms(&query) => result,
                };
                let mut admitted = 0;
                let read_result = match deadline {
                    Err(error) => Err(error),
                    Ok(deadline)
                        if deadline.is_none_or(|at| at > Utc::now().timestamp_millis()) =>
                    {
                        Ok(deadline)
                    }
                    Ok(_) => {
                        let candidates = tokio::select! {
                            biased;
                            _ = stop.cancelled() => break,
                            result = worker.store.list_due_batch(Utc::now().timestamp_millis(), &query) => result,
                        };
                        match candidates {
                            Err(error) => Err(error),
                            Ok(candidates) => {
                                for candidate in candidates {
                                    if stop.is_cancelled()
                                        || active.len() >= MAX_CONCURRENT_DELIVERIES
                                    {
                                        break;
                                    }
                                    let admission = Admission {
                                        id: candidate.job.id,
                                        session_id: candidate.job.owner_session_id,
                                    };
                                    if admission.session_id.is_some_and(|id| {
                                        active.values().any(|entry| entry.session_id == Some(id))
                                    }) {
                                        continue;
                                    }
                                    let delivery_worker = Arc::clone(&worker);
                                    // Reserve a slot, then claim immediately in
                                    // its task, never while queued behind a sink.
                                    let task = deliveries.spawn(async move {
                                        match delivery_worker
                                            .claim_candidate(candidate, Utc::now())
                                            .await?
                                        {
                                            Some(claim) => {
                                                delivery_worker
                                                    .deliver_with_lease(claim, CLAIM_HEARTBEAT)
                                                    .await?;
                                                Ok(DeliveryProgress::Applied)
                                            }
                                            None => Ok(DeliveryProgress::Idle),
                                        }
                                    });
                                    active.insert(task.id(), admission);
                                    admitted += 1;
                                }
                                if active.len() < MAX_CONCURRENT_DELIVERIES {
                                    let query = admission_query(&active, &deferred);
                                    tokio::select! {
                                        biased;
                                        _ = stop.cancelled() => break,
                                        result = worker.store.next_wake_at_ms(&query) => result,
                                    }
                                } else {
                                    Ok(None)
                                }
                            }
                        }
                    }
                };
                let now = Instant::now();
                match read_result {
                    Ok(deadline) => {
                        failures = 0;
                        reads_allowed_at = now;
                        let delay = deadline.map_or(reconciliation, |at| {
                            Duration::from_millis(
                                at.saturating_sub(Utc::now().timestamp_millis()).max(0) as u64,
                            )
                            .min(reconciliation)
                        });
                        // A lost optimistic race or malformed state must not
                        // create a zero-delay database loop. Useful backlog
                        // can refill unused slots without an artificial tick.
                        next_read = now
                            + if delay.is_zero() && admitted == 0 {
                                ADMISSION_RETRY_DELAY
                            } else {
                                delay
                            };
                        if let Some(until) = deferred.values().min() {
                            next_read = next_read.min(*until);
                        }
                    }
                    Err(error) => {
                        failures = failures.saturating_add(1).min(5);
                        let delay = (ERROR_RETRY_DELAY * 2u32.pow(failures - 1))
                            .min(Duration::from_secs(60));
                        reads_allowed_at = now + delay;
                        next_read = reads_allowed_at;
                        tracing::error!(target: "agena_scheduler", %error, retry_ms = delay.as_millis(), "scheduled-job read failed; backing off");
                    }
                }
            }

            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                completion = deliveries.join_next_with_id(), if !deliveries.is_empty() => {
                    if let Some(completion) = completion {
                        record_completion(completion, &mut active, &mut deferred);
                        next_read = Instant::now().max(reads_allowed_at);
                    }
                }
                result = wake.changed() => {
                    if result.is_err() { break; }
                    next_read = Instant::now().max(reads_allowed_at);
                }
                _ = tokio::time::sleep_until(next_read), if active.len() < MAX_CONCURRENT_DELIVERIES => {}
            }
        }
        // Graceful stop closes admission but lets each admitted occurrence
        // finalize with its heartbeat. Drop aborts the whole JoinSet instead.
        while let Some(completion) = deliveries.join_next_with_id().await {
            record_completion(completion, &mut active, &mut deferred);
        }
    }

    pub(super) async fn claim_candidate(
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
        let _commit = self.changes.commit.lock().await;
        match job.claim_due_delivery(now)? {
            crate::job::ClaimDueDelivery::NotDue => Ok(None),
            crate::job::ClaimDueDelivery::StateUpdated => {
                if self.store.replace(&expected, job.clone()).await? {
                    self.changes.publish(SchedulerChange::Upsert(job), false);
                }
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
                    self.changes
                        .publish(SchedulerChange::Upsert(job.clone()), false);
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

    pub(super) async fn deliver_with_lease(
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

    pub(super) async fn persist_completed_delivery(
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
            let _commit = self.changes.commit.lock().await;
            let mut job = expected.job.clone();
            job.finish_delivery(finished_at, &claim.delivery, result.clone())?;
            if self
                .store
                .finish(
                    &expected,
                    &claim.claim_key,
                    job.clone(),
                    Utc::now().timestamp_millis(),
                )
                .await?
            {
                self.changes.publish(SchedulerChange::Upsert(job), false);
                return Ok(());
            }
            drop(_commit);
            tokio::task::yield_now().await;
        }
        Err(SchedulerError::Conflict(id))
    }
}

fn admission_query(
    active: &HashMap<Id, Admission>,
    deferred: &HashMap<uuid::Uuid, Instant>,
) -> DueJobQuery {
    let mut sessions: Vec<_> = active
        .values()
        .filter_map(|entry| entry.session_id)
        .collect();
    sessions.sort_unstable();
    sessions.dedup();
    DueJobQuery {
        limit: MAX_CONCURRENT_DELIVERIES.saturating_sub(active.len()),
        excluded_job_ids: active
            .values()
            .map(|entry| entry.id)
            .chain(deferred.keys().copied())
            .collect(),
        excluded_session_ids: sessions,
    }
}

fn record_completion(
    completion: Result<(Id, SchedulerResult<DeliveryProgress>), tokio::task::JoinError>,
    active: &mut HashMap<Id, Admission>,
    deferred: &mut HashMap<uuid::Uuid, Instant>,
) {
    let (task_id, result) = match completion {
        Ok((id, result)) => (id, result),
        Err(error) => (
            error.id(),
            Err(SchedulerError::Persistence(sea_orm::DbErr::Custom(
                format!("delivery worker failed: {error}"),
            ))),
        ),
    };
    let Some(admission) = active.remove(&task_id) else {
        return;
    };
    let delay = match result {
        Ok(DeliveryProgress::Applied) => return,
        Ok(DeliveryProgress::Idle) => ADMISSION_RETRY_DELAY,
        Err(error) => {
            tracing::error!(target: "agena_scheduler", job_id = %admission.id, %error, "scheduled delivery failed; durable state retained");
            ERROR_RETRY_DELAY
        }
    };
    if deferred.len() >= MAX_DEFERRED_JOBS
        && !deferred.contains_key(&admission.id)
        && let Some(id) = deferred
            .iter()
            .min_by_key(|(_, until)| **until)
            .map(|(id, _)| *id)
    {
        deferred.remove(&id);
    }
    deferred.insert(admission.id, Instant::now() + delay);
}
