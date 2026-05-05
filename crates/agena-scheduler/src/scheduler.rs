use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::SchedulerResult;
use crate::job::{JobOutcome, JobSink, ScheduledJob};
use crate::store::JobStore;

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
        let me = self.clone();
        *g = Some(tokio::spawn(async move { me.run_loop().await }));
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

    async fn run_loop(self: Arc<Self>) {
        loop {
            let due = self.collect_due().await;
            for mut job in due {
                let now = Utc::now();
                let result = self.sink.deliver(&job).await;
                job.record_delivery(now, result);
                match job.advance(now) {
                    Ok(JobOutcome::Continued) => {
                        self.store.replace(job.id, job).await;
                    }
                    Ok(JobOutcome::Expired) => {
                        self.store.remove(job.id).await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "agena_scheduler",
                            "advance() failed for job {}: {e}",
                            job.id
                        );
                        self.store.remove(job.id).await;
                    }
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(self.tick) => {}
                _ = self.stop.notified() => break,
            }
        }
    }

    async fn collect_due(&self) -> Vec<ScheduledJob> {
        let now = Utc::now();
        self.store
            .list()
            .await
            .into_iter()
            .filter(|j| j.due(now))
            .collect()
    }
}

/// Convenience constructor: scheduler + in-memory store + custom sink.
pub fn build_in_memory(sink: Arc<dyn JobSink>, tick: Duration) -> Arc<Scheduler> {
    let store = Arc::new(crate::store::InMemoryJobStore::new()) as Arc<dyn JobStore>;
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
