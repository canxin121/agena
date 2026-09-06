use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Notify;

use crate::{JobDeliveryAttempt, JobDeliveryResult, JobSink, ScheduledJob};

use super::{Scheduler, build_in_memory};

#[derive(Default)]
struct ControlledSink {
    entered: Notify,
    release: Notify,
    dropped: Arc<AtomicBool>,
    finished_at: parking_lot::Mutex<Option<DateTime<Utc>>>,
}

struct DeliveryGuard(Arc<AtomicBool>);

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl JobSink for ControlledSink {
    async fn deliver(
        &self,
        _job: &ScheduledJob,
        _delivery: &JobDeliveryAttempt,
    ) -> JobDeliveryResult {
        let _guard = DeliveryGuard(Arc::clone(&self.dropped));
        self.entered.notify_one();
        self.release.notified().await;
        *self.finished_at.lock() = Some(Utc::now());
        JobDeliveryResult::submitted(None)
    }
}

async fn wait_until_stopped(scheduler: &Scheduler) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if scheduler
                .handle
                .lock()
                .as_ref()
                .is_none_or(|running| running.handle.is_finished())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the scheduler must honor a pending stop request");
}

fn due_job(prompt: &str) -> ScheduledJob {
    ScheduledJob::new_once(Utc::now() - chrono::Duration::seconds(1), prompt)
}

#[tokio::test]
async fn pause_during_sqlite_delivery_is_durable_and_survives_completion() {
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    crate::schema::initialize_schema(&db).await.unwrap();
    let sink = Arc::new(ControlledSink::default());
    let scheduler =
        super::build_persistent(Some(Arc::new(db)), sink.clone(), Duration::from_secs(60));
    let mut job = ScheduledJob::new_cron("0 * * * * * *", "pause in flight", 1).unwrap();
    job.next_fire_at = Some(Utc::now() - chrono::Duration::seconds(1));
    let id = job.id;
    scheduler.add(job).await.unwrap();
    scheduler.start();
    sink.entered.notified().await;
    assert!(scheduler.pause(id).await.unwrap().unwrap().paused);
    assert!(
        scheduler.get(id).await.unwrap().unwrap().paused,
        "pause must be persisted while claimed"
    );
    scheduler.stop();
    sink.release.notify_one();
    wait_until_stopped(&scheduler).await;
    assert!(
        scheduler.get(id).await.unwrap().unwrap().paused,
        "finalization must preserve the pause"
    );
}

#[tokio::test]
async fn scheduler_surfaces_sqlite_write_failures_to_callers() {
    use sea_orm::ConnectionTrait;
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    crate::schema::initialize_schema(&db).await.unwrap();
    let scheduler = super::build_persistent(
        Some(Arc::new(db.clone())),
        Arc::new(NoopSink),
        Duration::from_secs(60),
    );
    db.execute(sea_orm::Statement::from_string(sea_orm::DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_job BEFORE INSERT ON agena_scheduler_jobs BEGIN SELECT RAISE(ABORT, 'injected write failure'); END",
    )).await.unwrap();
    assert!(scheduler.add(due_job("cannot persist")).await.is_err());
    assert!(scheduler.list().await.unwrap().is_empty());
    db.execute(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "DROP TRIGGER reject_job",
    ))
    .await
    .unwrap();
    let job = due_job("editable");
    let id = job.id;
    scheduler.add(job).await.unwrap();
    db.execute(sea_orm::Statement::from_string(sea_orm::DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_edit BEFORE UPDATE ON agena_scheduler_jobs BEGIN SELECT RAISE(ABORT, 'injected edit failure'); END",
    )).await.unwrap();
    assert!(scheduler.pause(id).await.is_err());
    assert!(!scheduler.get(id).await.unwrap().unwrap().paused);
    db.execute(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "DROP TRIGGER reject_edit",
    ))
    .await
    .unwrap();
    db.execute(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "CREATE TRIGGER lose_edit BEFORE UPDATE ON agena_scheduler_jobs BEGIN SELECT RAISE(IGNORE); END",
    )).await.unwrap();
    assert!(matches!(
        scheduler.pause(id).await,
        Err(crate::SchedulerError::Conflict(_))
    ));
    assert!(!scheduler.get(id).await.unwrap().unwrap().paused);
}

#[tokio::test]
async fn resume_and_update_during_delivery_preserve_the_attempt_and_new_configuration() {
    let sink = Arc::new(ControlledSink::default());
    let scheduler = build_in_memory(sink.clone(), Duration::from_secs(60));
    let job = due_job("initial prompt");
    let id = job.id;
    scheduler.add(job).await.unwrap();
    scheduler.start();
    sink.entered.notified().await;
    let delivery = scheduler.get(id).await.unwrap().unwrap().pending_delivery;
    scheduler.pause(id).await.unwrap();
    scheduler.resume(id).await.unwrap();
    scheduler
        .update(id, Some("updated prompt".into()), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        scheduler.get(id).await.unwrap().unwrap().pending_delivery,
        delivery
    );
    scheduler.stop();
    sink.release.notify_one();
    wait_until_stopped(&scheduler).await;
    let finished = scheduler.get(id).await.unwrap().unwrap();
    assert!(finished.completed);
    assert!(!finished.paused);
    assert_eq!(finished.prompt, "updated prompt");
    assert_eq!(scheduler.history(Some(id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn slow_delivery_does_not_claim_the_rest_of_the_due_batch() {
    let sink = Arc::new(ControlledSink::default());
    let scheduler = build_in_memory(sink.clone(), Duration::from_secs(60));
    scheduler.add(due_job("first")).await.unwrap();
    scheduler.add(due_job("second")).await.unwrap();
    scheduler.start();
    sink.entered.notified().await;
    let jobs = scheduler.list().await.unwrap();
    assert_eq!(
        jobs.iter()
            .filter(|job| job.pending_delivery.is_some())
            .count(),
        1
    );
    scheduler.stop();
    sink.release.notify_one();
    wait_until_stopped(&scheduler).await;
    let jobs = scheduler.list().await.unwrap();
    assert_eq!(jobs.iter().filter(|job| job.completed).count(), 1);
    assert_eq!(
        jobs.iter()
            .filter(|job| job.pending_delivery.is_none())
            .count(),
        2
    );
}

#[tokio::test]
async fn worker_renews_a_blocked_sink_and_drops_it_after_ownership_is_lost() {
    use sea_orm::ConnectionTrait;
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    crate::schema::initialize_schema(&db).await.unwrap();
    db.execute(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "CREATE TABLE renewal_observations (at_ms INTEGER)",
    ))
    .await
    .unwrap();
    db.execute(sea_orm::Statement::from_string(sea_orm::DatabaseBackend::Sqlite,
        "CREATE TRIGGER observe_renewal AFTER UPDATE OF claimed_at_ms ON agena_scheduler_jobs WHEN NEW.delivery_key = OLD.delivery_key BEGIN INSERT INTO renewal_observations VALUES (NEW.claimed_at_ms); END",
    )).await.unwrap();
    use crate::JobStore;
    let sink = Arc::new(ControlledSink::default());
    let store = Arc::new(crate::SqliteJobStore::new(db.clone()));
    let worker = super::SchedulerWorker {
        store: store.clone(),
        sink: sink.clone(),
        tick: Duration::from_secs(60),
    };
    let job = due_job("renew until deleted");
    let id = job.id;
    store.put(job).await.unwrap();
    let claim = worker
        .claim_candidate(store.get(id).await.unwrap().unwrap(), Utc::now())
        .await
        .unwrap()
        .unwrap();
    let handle = tokio::spawn(async move {
        worker
            .deliver_with_lease(claim, Duration::from_millis(20))
            .await
    });
    sink.entered.notified().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let count: i64 = db
                .query_one(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    "SELECT COUNT(*) AS count FROM renewal_observations",
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get("", "count")
                .unwrap();
            if count >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("periodic renewal should reach the database");
    assert!(!sink.dropped.load(Ordering::SeqCst));
    store.remove(id).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(crate::SchedulerError::Conflict(_))));
    assert!(sink.dropped.load(Ordering::SeqCst));
    assert!(store.list_history(Some(id), 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn lease_renewal_does_not_stall_a_finalization_waiting_for_sqlite() {
    use crate::JobStore;
    use sea_orm::{
        ConnectOptions, ConnectionTrait, Database, DatabaseBackend, Statement, TransactionTrait,
    };
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("scheduler.db").display()
    );
    let mut options = ConnectOptions::new(url.clone());
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    crate::schema::initialize_schema(&db).await.unwrap();
    let blocker = Database::connect(url).await.unwrap();
    let store = Arc::new(crate::SqliteJobStore::new(db));
    let sink = Arc::new(ControlledSink::default());
    let worker = super::SchedulerWorker {
        store: store.clone(),
        sink: sink.clone(),
        tick: Duration::from_secs(60),
    };
    let job = due_job("finish through temporary contention");
    let id = job.id;
    store.put(job).await.unwrap();
    let claim = worker
        .claim_candidate(store.get(id).await.unwrap().unwrap(), Utc::now())
        .await
        .unwrap()
        .unwrap();
    let handle = tokio::spawn(async move {
        worker
            .deliver_with_lease(claim, Duration::from_millis(20))
            .await
    });
    sink.entered.notified().await;
    // Hold SQLite's writer lock on a separate connection. Finalization must
    // retain its transaction while waiting; renewal then needs that same
    // single-connection pool and must not prevent finalization being polled.
    let txn = blocker.begin().await.unwrap();
    txn.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "UPDATE agena_scheduler_jobs SET updated_at_ms = updated_at_ms",
    ))
    .await
    .unwrap();
    sink.release.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;
    txn.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("finalization must finish once the writer lock is released")
        .unwrap()
        .unwrap();
    assert!(store.get(id).await.unwrap().unwrap().job.completed);
    assert_eq!(store.list_history(Some(id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_prior_cron_occurrence_cannot_finalize_a_later_claim() {
    use crate::store::{InMemoryJobStore, JobStore};
    let store = Arc::new(InMemoryJobStore::new());
    let worker = super::SchedulerWorker {
        store: store.clone(),
        sink: Arc::new(NoopSink),
        tick: Duration::from_secs(60),
    };
    let mut job = ScheduledJob::new_cron("0 * * * * * *", "recurring", 1).unwrap();
    job.next_fire_at = Some(Utc::now() - chrono::Duration::seconds(2));
    let id = job.id;
    store.put(job).await.unwrap();
    let first = worker
        .claim_candidate(store.get(id).await.unwrap().unwrap(), Utc::now())
        .await
        .unwrap()
        .unwrap();
    let first_snapshot = store.get(id).await.unwrap().unwrap();
    worker
        .persist_completed_delivery(&first, JobDeliveryResult::submitted(None), Utc::now())
        .await
        .unwrap();
    let expected = store.get(id).await.unwrap().unwrap();
    let mut next = expected.job.clone();
    next.next_fire_at = Some(Utc::now() - chrono::Duration::seconds(1));
    assert!(store.replace(&expected, next).await.unwrap());
    let next = worker
        .claim_candidate(store.get(id).await.unwrap().unwrap(), Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.delivery.attempt, next.delivery.attempt);
    assert_ne!(first.delivery.delivery_key, next.delivery.delivery_key);
    assert_ne!(first.claim_key, next.claim_key);
    assert!(
        !store
            .finish(
                &first_snapshot,
                &first.claim_key,
                next.job,
                Utc::now().timestamp_millis()
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn stop_before_the_first_poll_prevents_delivery() {
    let sink = Arc::new(ControlledSink::default());
    let scheduler = build_in_memory(sink.clone(), Duration::from_secs(60));
    scheduler.add(due_job("must not fire")).await.unwrap();
    scheduler.start();
    scheduler.stop();
    wait_until_stopped(&scheduler).await;
    assert!(sink.finished_at.lock().is_none());
    assert!(
        scheduler.list().await.unwrap()[0]
            .pending_delivery
            .is_none()
    );
}

#[tokio::test]
async fn stop_during_delivery_is_retained_and_the_scheduler_can_restart() {
    let sink = Arc::new(ControlledSink::default());
    let scheduler = build_in_memory(sink.clone(), Duration::from_secs(60));
    let first = due_job("first");
    let id = first.id;
    scheduler.add(first).await.unwrap();
    scheduler.start();
    sink.entered.notified().await;
    scheduler.stop();
    sink.release.notify_one();
    wait_until_stopped(&scheduler).await;
    let completed = scheduler.get(id).await.unwrap().unwrap();
    assert!(completed.completed);
    assert!(
        completed.last_fired_at >= *sink.finished_at.lock(),
        "completion time must be sampled after the delivery finishes"
    );

    scheduler.add(due_job("after restart")).await.unwrap();
    scheduler.start();
    scheduler.start();
    tokio::time::timeout(Duration::from_secs(1), sink.entered.notified())
        .await
        .expect("start must launch a new loop after the previous one stopped");
    scheduler.stop();
    sink.release.notify_one();
    wait_until_stopped(&scheduler).await;
    assert!(
        scheduler
            .list()
            .await
            .unwrap()
            .iter()
            .all(|job| job.completed)
    );
}

#[tokio::test]
async fn dropping_the_scheduler_releases_an_in_flight_delivery() {
    let sink = Arc::new(ControlledSink::default());
    let scheduler = build_in_memory(sink.clone(), Duration::from_secs(60));
    scheduler.add(due_job("blocked delivery")).await.unwrap();
    scheduler.start();
    sink.entered.notified().await;
    let weak = Arc::downgrade(&scheduler);
    drop(scheduler);
    assert!(
        weak.upgrade().is_none(),
        "the worker must not retain the scheduler owner"
    );
    tokio::task::yield_now().await;
    assert!(
        sink.dropped.load(Ordering::SeqCst),
        "dropping the scheduler cancels its worker"
    );
}

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
