//! Exercise ownership recovery after a real process exits without destructors.

use std::sync::Arc;
use std::time::Duration;

use agena_scheduler::{
    ClaimDueDelivery, JobDeliveryAttempt, JobDeliveryResult, JobSink, JobStore, ScheduledJob,
    Scheduler, SqliteJobStore,
};
use chrono::Utc;
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

const FIXTURE_DB: &str = "AGENA_SCHEDULER_RECOVERY_FIXTURE_DB";

#[tokio::test]
async fn crash_after_durable_claim() {
    let Some(path) = std::env::var_os(FIXTURE_DB) else {
        return;
    };
    let db = Database::connect(format!(
        "sqlite://{}?mode=rwc",
        std::path::Path::new(&path).display()
    ))
    .await
    .unwrap();
    agena_scheduler::schema::initialize_schema(&db)
        .await
        .unwrap();
    let store = SqliteJobStore::new(db.clone());
    // Persist a claim whose lease will already have expired when the parent
    // restarts. This avoids waiting 90 seconds while using the real clock.
    let claimed_at = Utc::now()
        - chrono::Duration::milliseconds(agena_scheduler::store::CLAIM_LEASE_MILLIS + 1_000);
    let job = ScheduledJob::new_once(claimed_at, "recover the interrupted occurrence");
    let id = job.id;
    store.put(job).await.unwrap();
    let expected = store.get(id).await.unwrap().unwrap();
    let mut pending = expected.job.clone();
    assert!(matches!(
        pending.claim_due_delivery(claimed_at).unwrap(),
        ClaimDueDelivery::Deliver(_)
    ));
    assert!(
        store
            .claim(
                &expected,
                pending,
                "abandoned-process".into(),
                claimed_at.timestamp_millis()
            )
            .await
            .unwrap()
    );
    // Schema-v1 claims written by the previous implementation cleared this
    // derived column. The pending JSON is still sufficient to recover them.
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "UPDATE agena_scheduler_jobs SET next_fire_at_ms = NULL",
    ))
    .await
    .unwrap();
    std::process::exit(0);
}

struct RecordingSink(tokio::sync::mpsc::UnboundedSender<JobDeliveryAttempt>);

#[async_trait::async_trait]
impl JobSink for RecordingSink {
    async fn deliver(&self, _: &ScheduledJob, delivery: &JobDeliveryAttempt) -> JobDeliveryResult {
        self.0.send(delivery.clone()).unwrap();
        JobDeliveryResult::submitted(Some(42))
    }
}

#[tokio::test]
async fn a_fresh_process_recovers_the_claim_with_the_same_idempotency_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scheduler.db");
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "crash_after_durable_claim", "--nocapture"])
        .env(FIXTURE_DB, &path)
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::connect(format!("sqlite://{}?mode=rw", path.display()))
        .await
        .unwrap();
    agena_scheduler::schema::initialize_schema(&db)
        .await
        .unwrap();
    let store = Arc::new(SqliteJobStore::new(db.clone()));
    let abandoned = store.list().await.unwrap().pop().unwrap();
    let id = abandoned.job.id;
    let first = abandoned.job.pending_delivery.as_ref().unwrap().clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        store.clone(),
        Arc::new(RecordingSink(tx)),
        Duration::from_millis(10),
    );
    scheduler.start();
    let recovered = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.delivery_key, first.delivery_key);
    assert_eq!(recovered.attempt, first.attempt + 1);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if scheduler.get(id).await.unwrap().unwrap().completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    scheduler.stop();
    assert!(
        !store
            .finish(
                &abandoned,
                "abandoned-process",
                abandoned.job.clone(),
                Utc::now().timestamp_millis()
            )
            .await
            .unwrap()
    );
    let history = scheduler.history(Some(id), 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].record.session_id, Some(42));
    assert!(rx.try_recv().is_err());
    drop(scheduler);
    drop(store);
    db.close().await.unwrap();
}
