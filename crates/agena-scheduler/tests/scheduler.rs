use std::sync::Arc;
use std::time::Duration;

use agena_scheduler::{
    InMemoryJobStore, JobDeliveryResult, JobSink, ScheduledJob, Scheduler,
    scheduler::build_in_memory,
};
use parking_lot::Mutex;

#[derive(Default)]
struct CountingSink {
    fires: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl JobSink for CountingSink {
    async fn deliver(&self, job: &ScheduledJob) -> JobDeliveryResult {
        self.fires.lock().push(job.prompt.clone());
        JobDeliveryResult::submitted(None)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_shot_fires_once_then_disappears() {
    let sink = Arc::new(CountingSink::default());
    let store = Arc::new(InMemoryJobStore::new());
    let scheduler = Scheduler::new(store.clone(), sink.clone(), Duration::from_millis(20));
    scheduler.start();

    // Schedule for ~50ms in the future.
    let when = chrono::Utc::now() + chrono::Duration::milliseconds(50);
    scheduler.add(ScheduledJob::new_once(when, "ping")).await;

    tokio::time::sleep(Duration::from_millis(200)).await;
    let fires = sink.fires.lock().clone();
    assert_eq!(fires, vec!["ping".to_string()]);
    assert!(
        scheduler.list().await.is_empty(),
        "one-shot must be removed"
    );
    scheduler.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn add_remove_round_trip() {
    let sink = Arc::new(CountingSink::default());
    let scheduler = build_in_memory(sink, Duration::from_millis(20));

    // Cron firing every minute — won't fire during this test.
    let job = ScheduledJob::new_cron("0 * * * * *", "hi", 7).unwrap();
    let id = job.id;
    scheduler.add(job).await;
    assert_eq!(scheduler.list().await.len(), 1);
    assert!(scheduler.remove(id).await);
    assert!(scheduler.list().await.is_empty());
}

#[test]
fn cron_invalid_expression_is_rejected() {
    let res = ScheduledJob::new_cron("not-a-cron", "hi", 7);
    assert!(res.is_err());
}
