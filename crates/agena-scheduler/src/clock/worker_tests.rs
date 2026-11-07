//! Exercise the real scheduler at saturation in an isolated test subprocess.
//! The clock opt-in is process-wide, so never mutate the parent test runner's
//! environment or interfere with its concurrently running contract tests.

use crate::{JobDeliveryAttempt, JobDeliveryResult, JobSink, ScheduledJob};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

const CHILD: &str = "AGENA_CLOCK_SATURATION_TEST_CHILD";
const TEST_NAME: &str =
    "clock::worker_tests::admission_controls_acknowledge_while_all_slots_are_busy";

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct BlockedSink {
    entered: mpsc::UnboundedSender<uuid::Uuid>,
    release: Semaphore,
}

#[async_trait::async_trait]
impl JobSink for BlockedSink {
    async fn deliver(
        &self,
        job: &ScheduledJob,
        _attempt: &JobDeliveryAttempt,
    ) -> JobDeliveryResult {
        self.entered.send(job.id).unwrap();
        self.release.acquire().await.unwrap().forget();
        JobDeliveryResult::submitted(None)
    }
}

fn publish(path: &Path, revision: u64, paused: bool) {
    publish_at(path, 1_700_000_000_000_i64, revision, paused);
}

fn publish_at(path: &Path, now_ms: i64, revision: u64, paused: bool) {
    let state = serde_json::json!({
        "version": 1, "now_ms": now_ms,
        "elapsed_ms": 0, "revision": revision, "scheduler_paused": paused,
    });
    let tmp = path.with_extension("control.swap");
    std::fs::write(&tmp, serde_json::to_vec(&state).unwrap()).unwrap();
    std::fs::rename(tmp, path).unwrap();
}

async fn observe(path: &Path, revision: u64, paused: bool, active: usize) {
    loop {
        if let Ok(raw) = std::fs::read(path.with_extension("observed.json")) {
            let observation: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            if observation["revision"] == revision
                && observation["scheduler_paused"] == paused
                && observation["active_deliveries"] == active
            {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn saturated_worker() {
    let path = PathBuf::from(std::env::var_os(super::TEST_CLOCK_ENV).unwrap());
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let sink = Arc::new(BlockedSink {
        entered,
        release: Semaphore::new(0),
    });
    let scheduler = crate::scheduler::build_in_memory(sink.clone(), Duration::from_secs(60));
    for index in 0..5 {
        scheduler
            .add(ScheduledJob::new_once(
                super::now(),
                format!("blocked-{index}"),
            ))
            .await
            .unwrap();
    }
    scheduler.start();
    for _ in 0..4 {
        arrivals.recv().await.unwrap();
    }
    assert!(
        arrivals.try_recv().is_err(),
        "worker exceeded its admission limit"
    );

    // A full worker disables its ordinary deadline branch, but must still
    // observe and acknowledge controls instead of hanging the test driver.
    publish(&path, 1, true);
    observe(&path, 1, true, 4).await;
    sink.release.add_permits(1);
    observe(&path, 1, true, 3).await;
    assert!(
        arrivals.try_recv().is_err(),
        "paused worker admitted queued work"
    );

    publish(&path, 2, false);
    arrivals.recv().await.unwrap();
    observe(&path, 2, false, 4).await;
    assert!(arrivals.try_recv().is_err());
    scheduler.stop();
    sink.release.add_permits(4);
}

#[test]
fn admission_controls_acknowledge_while_all_slots_are_busy() {
    if std::env::var_os(CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(8), saturated_worker())
                .await
                .expect("saturated scheduler failed to acknowledge a clock admission boundary");
        });
        return;
    }
    let path = std::env::temp_dir().join(format!("agena-clock-worker-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    let _cleanup = Cleanup(path.clone());
    let clock = path.join("clock.txt");
    publish(&clock, 0, false);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD, "1")
        .env(super::TEST_CLOCK_ENV, &clock)
        .output()
        .expect("launch isolated scheduler clock test");
    assert!(
        output.status.success(),
        "isolated scheduler failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A six-hour virtual calendar jump must not expire a healthy delivery's
/// real lease while its sink is already running. The lease clock and the
/// scheduler's business clock are intentionally separate domains.
async fn active_lease_survives_virtual_calendar_jump() {
    let path = PathBuf::from(std::env::var_os(super::TEST_CLOCK_ENV).unwrap());
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let sink = Arc::new(BlockedSink {
        entered,
        release: Semaphore::new(0),
    });
    let scheduler = crate::scheduler::build_in_memory(sink.clone(), Duration::from_secs(60));
    let job = ScheduledJob::new_once(super::now(), "leased-through-six-hour-jump");
    let id = job.id;
    scheduler.add(job).await.unwrap();
    scheduler.start();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), arrivals.recv())
            .await
            .unwrap(),
        Some(id)
    );
    publish_at(&path, 1_700_000_000_000 + 6 * 3_600_000, 1, false);
    sink.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let job = scheduler.get(id).await.unwrap().unwrap();
            if job.completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the healthy first claim must commit after the virtual jump");
    let history = scheduler.history(Some(id), 10).await.unwrap();
    assert_eq!(
        history.len(),
        1,
        "one durable delivery, not a recovered retry"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), arrivals.recv())
            .await
            .is_err(),
        "scheduler dispatched a second sink invocation after a successful first delivery"
    );
    scheduler.stop();
}

#[test]
fn virtual_calendar_jump_preserves_active_lease() {
    const LEASE_CHILD: &str = "AGENA_CLOCK_ACTIVE_LEASE_TEST_CHILD";
    const LEASE_NAME: &str = "clock::worker_tests::virtual_calendar_jump_preserves_active_lease";
    if std::env::var_os(LEASE_CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(8),
                active_lease_survives_virtual_calendar_jump(),
            )
            .await
            .expect("active lease should not expire during a six-hour business-time jump");
        });
        return;
    }
    let dir = std::env::temp_dir().join(format!("agena-clock-lease-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let clock = dir.join("clock.txt");
    publish(&clock, 0, false);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", LEASE_NAME, "--nocapture"])
        .env(LEASE_CHILD, "1")
        .env(super::TEST_CLOCK_ENV, &clock)
        .output()
        .expect("run isolated lease clock case");
    assert!(
        output.status.success(),
        "virtual clock lease regression failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
