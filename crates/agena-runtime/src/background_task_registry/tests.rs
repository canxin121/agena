use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

fn spec(key: Option<&str>) -> RuntimeBackgroundTaskSpec {
    RuntimeBackgroundTaskSpec::new(
        RuntimeBackgroundTaskKind::RuntimeReload,
        crate::RuntimeBackgroundTaskOrigin::User,
        "registry lifecycle test",
        key.map(str::to_owned),
        true,
    )
}

async fn terminal(
    registry: &RuntimeBackgroundTaskRegistry<String>,
    id: &str,
) -> RuntimeBackgroundTask {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let task = registry
                .list()
                .into_iter()
                .find(|task| task.id == id)
                .unwrap();
            if !task.is_running() {
                break task;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a completed/cancelled/panicked worker must reach a terminal record")
}

#[tokio::test]
async fn precancelled_worker_never_calls_the_factory_or_reports_success() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = calls.clone();
    let start = registry
        .spawn(spec(None), move |_| {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(RuntimeBackgroundTaskOutcome::succeeded("unexpected")) }
        })
        .unwrap();
    registry.cancel(&start.task.id).unwrap();
    let task = terminal(&registry, &start.task.id).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "already-cancelled work must never be invoked"
    );
    assert_eq!(task.status, RuntimeBackgroundTaskStatus::Cancelled);
}

#[tokio::test]
async fn panicking_worker_reaches_failure_and_releases_its_dedupe_key() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let start = registry
        .spawn(spec(Some("panic")), |_| async {
            tokio::task::yield_now().await;
            panic!("injected task panic: token=do-not-expose");
        })
        .unwrap();
    let task = terminal(&registry, &start.task.id).await;
    assert_eq!(task.status, RuntimeBackgroundTaskStatus::Failed);
    let encoded = serde_json::to_string(&task).unwrap();
    assert!(!encoded.contains("do-not-expose"));
    let next = registry
        .spawn(spec(Some("panic")), |_| async {
            Ok(RuntimeBackgroundTaskOutcome::succeeded("recovered"))
        })
        .unwrap();
    assert!(next.started);
    assert_eq!(
        terminal(&registry, &next.task.id).await.status,
        RuntimeBackgroundTaskStatus::Succeeded
    );
}

struct PanickingListener;
impl RuntimeBackgroundTaskListener for PanickingListener {
    fn on_started(&self, _: &RuntimeBackgroundTask) {
        panic!("injected observer panic");
    }
    fn on_finished(&self, _: &RuntimeBackgroundTask) {
        panic!("injected terminal observer panic");
    }
}

#[tokio::test]
async fn observer_panics_do_not_orphan_accepted_work() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    registry.set_listener(Arc::new(PanickingListener));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        registry.spawn(spec(None), |_| async {
            Ok(RuntimeBackgroundTaskOutcome::succeeded("done"))
        })
    }));
    assert!(outcome.is_ok(), "an observer cannot unwind task admission");
    let start = outcome.unwrap().unwrap();
    assert_eq!(
        terminal(&registry, &start.task.id).await.status,
        RuntimeBackgroundTaskStatus::Succeeded
    );
}

struct LockCheckingListener {
    inner: std::sync::Weak<Mutex<RuntimeBackgroundTaskState>>,
    lock_available: Arc<AtomicBool>,
    finished: Arc<tokio::sync::Notify>,
}
impl RuntimeBackgroundTaskListener for LockCheckingListener {
    fn on_finished(&self, _: &RuntimeBackgroundTask) {
        let inner = self.inner.upgrade().unwrap();
        self.lock_available
            .store(inner.try_lock().is_some(), Ordering::SeqCst);
        self.finished.notify_one();
    }
}

#[tokio::test]
async fn terminal_observer_can_reenter_the_registry() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let lock_available = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    registry.set_listener(Arc::new(LockCheckingListener {
        inner: Arc::downgrade(&registry.inner),
        lock_available: lock_available.clone(),
        finished: finished.clone(),
    }));
    registry
        .spawn(spec(None), |_| async {
            Ok(RuntimeBackgroundTaskOutcome::succeeded("done"))
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), finished.notified())
        .await
        .unwrap();
    assert!(
        lock_available.load(Ordering::SeqCst),
        "terminal observers must run outside the registry lock"
    );
}

#[tokio::test]
async fn background_maintenance_cannot_start_after_shutdown() {
    let control = crate::TaskControl::default();
    control.shutdown();
    let ran = Arc::new(AtomicBool::new(false));
    let worker_ran = ran.clone();
    control.spawn(async move {
        worker_ran.store(true, Ordering::SeqCst);
    });
    tokio::task::yield_now().await;
    assert!(
        !ran.load(Ordering::SeqCst),
        "stopped maintenance control must reject new work"
    );
}

#[test]
fn unavailable_executor_rejects_before_creating_a_task_record() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let result = registry.spawn(spec(None), |_| async { unreachable!() });
    assert!(matches!(
        result,
        Err(RuntimeBackgroundTaskControlError::ExecutorUnavailable)
    ));
    assert!(registry.list().is_empty());
}

#[tokio::test]
async fn capacity_preserves_deduplication_and_releases_slots_on_completion() {
    let registry = RuntimeBackgroundTaskRegistry::<String> {
        active_limit: 2,
        history_limit: 2,
        ..Default::default()
    };
    let first = registry
        .spawn(spec(Some("first")), |_| std::future::pending())
        .unwrap();
    let second = registry
        .spawn(spec(Some("second")), |_| std::future::pending())
        .unwrap();
    let rejected = registry.spawn(spec(None), |_| async {
        panic!("overload factory must not execute")
    });
    assert!(matches!(
        rejected,
        Err(RuntimeBackgroundTaskControlError::Capacity { limit: 2 })
    ));
    let duplicate = registry
        .spawn(spec(Some("first")), |_| async {
            panic!("duplicate factory must not execute")
        })
        .unwrap();
    assert!(!duplicate.started);
    assert_eq!(duplicate.task.id, first.task.id);
    assert_eq!(registry.list().len(), 2);
    registry.cancel(&first.task.id).unwrap();
    assert_eq!(
        terminal(&registry, &first.task.id).await.status,
        RuntimeBackgroundTaskStatus::Cancelled
    );
    for _ in 0..6 {
        let next = registry
            .spawn(spec(Some("next")), |_| async {
                Ok(RuntimeBackgroundTaskOutcome::succeeded("done"))
            })
            .unwrap();
        assert!(next.started);
        assert_eq!(
            terminal(&registry, &next.task.id).await.status,
            RuntimeBackgroundTaskStatus::Succeeded
        );
        assert!(
            registry.list().len() <= 2,
            "history must not grow with completed work"
        );
        assert!(
            registry
                .list()
                .iter()
                .any(|task| task.id == second.task.id && task.is_running())
        );
    }
    registry.cancel(&second.task.id).unwrap();
    terminal(&registry, &second.task.id).await;
    let state = registry.inner.lock();
    assert!(
        state.controls.is_empty()
            && state.workers.is_empty()
            && state.dedupe_keys.is_empty()
            && state.active_by_key.is_empty()
    );
}

struct ShutdownOnStart(std::sync::Weak<crate::RuntimeControlState<(), String>>);
impl RuntimeBackgroundTaskListener for ShutdownOnStart {
    fn on_started(&self, _: &RuntimeBackgroundTask) {
        self.0.upgrade().unwrap().shutdown();
    }
}

#[tokio::test]
async fn shutdown_during_start_notification_cancels_before_factory_execution() {
    let control = Arc::new(crate::RuntimeControlState::<(), String>::new(
        Arc::new(()),
        None,
    ));
    let registry = control.background_tasks();
    registry.set_listener(Arc::new(ShutdownOnStart(Arc::downgrade(&control))));
    let called = Arc::new(AtomicBool::new(false));
    let factory_called = called.clone();
    let accepted = registry
        .spawn(spec(None), move |_| {
            factory_called.store(true, Ordering::SeqCst);
            async { Ok(RuntimeBackgroundTaskOutcome::succeeded("unexpected")) }
        })
        .unwrap();
    assert_eq!(
        terminal(registry, &accepted.task.id).await.status,
        RuntimeBackgroundTaskStatus::Cancelled
    );
    assert!(!called.load(Ordering::SeqCst));
    assert!(matches!(
        registry.spawn(spec(None), |_| async { unreachable!() }),
        Err(RuntimeBackgroundTaskControlError::Shutdown)
    ));
}

#[tokio::test]
async fn factory_panic_is_recorded_as_failure() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let start = registry
        .spawn(
            spec(None),
            |_| -> std::future::Ready<Result<RuntimeBackgroundTaskOutcome, String>> {
                panic!("injected synchronous factory panic");
            },
        )
        .unwrap();
    assert_eq!(
        terminal(&registry, &start.task.id).await.status,
        RuntimeBackgroundTaskStatus::Failed
    );
    assert!(registry.inner.lock().controls.is_empty());
}

#[tokio::test]
async fn aborted_workers_release_records_and_keys_before_and_after_first_poll() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    for wait_until_started in [false, true] {
        let (started, receiver) = tokio::sync::oneshot::channel();
        let start = registry
            .spawn(spec(Some("abort")), |_| async move {
                let _ = started.send(());
                std::future::pending().await
            })
            .unwrap();
        assert!(start.started);
        if wait_until_started {
            receiver.await.unwrap();
        }
        let abort = registry
            .inner
            .lock()
            .workers
            .get(&start.task.id)
            .unwrap()
            .clone();
        abort.abort();
        assert_eq!(
            terminal(&registry, &start.task.id).await.status,
            RuntimeBackgroundTaskStatus::Cancelled
        );
        let state = registry.inner.lock();
        assert!(
            state.controls.is_empty() && state.workers.is_empty() && state.active_by_key.is_empty()
        );
    }
}

#[test]
fn executor_shutdown_terminalizes_unpolled_workers_in_a_retained_registry() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let start = {
        let _entered = executor.enter();
        registry
            .spawn(spec(None), |_| async {
                panic!("unpolled task should be cancelled")
            })
            .unwrap()
    };
    drop(executor);
    let task = registry
        .list()
        .into_iter()
        .find(|task| task.id == start.task.id)
        .unwrap();
    assert_eq!(task.status, RuntimeBackgroundTaskStatus::Cancelled);
    assert!(registry.inner.lock().controls.is_empty());
}

#[tokio::test]
async fn concurrent_admission_and_shutdown_leave_no_running_or_uncontrolled_tasks() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    let barrier = Arc::new(std::sync::Barrier::new(17));
    let executor = tokio::runtime::Handle::current();
    let effects = Arc::new(AtomicUsize::new(0));
    let threads = (0..16)
        .map(|_| {
            let registry = registry.clone();
            let barrier = barrier.clone();
            let executor = executor.clone();
            let effects = effects.clone();
            std::thread::spawn(move || {
                let _entered = executor.enter();
                barrier.wait();
                registry.spawn(spec(None), move |_| {
                    effects.fetch_add(1, Ordering::SeqCst);
                    async { Ok(RuntimeBackgroundTaskOutcome::succeeded("unexpected")) }
                })
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    registry.shutdown();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    for result in results {
        match result {
            Ok(start) => assert_eq!(
                terminal(&registry, &start.task.id).await.status,
                RuntimeBackgroundTaskStatus::Cancelled
            ),
            Err(error) => assert!(matches!(error, RuntimeBackgroundTaskControlError::Shutdown)),
        }
    }
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let state = registry.inner.lock();
    assert!(state.controls.is_empty() && state.workers.is_empty());
}

#[tokio::test]
async fn missing_terminal_and_noncancellable_tasks_have_distinct_control_errors() {
    let registry = RuntimeBackgroundTaskRegistry::<String>::default();
    assert!(matches!(
        registry.cancel("missing"),
        Err(RuntimeBackgroundTaskControlError::NotFound(_))
    ));
    let uncancellable = RuntimeBackgroundTaskSpec::new(
        RuntimeBackgroundTaskKind::RuntimeReload,
        crate::RuntimeBackgroundTaskOrigin::User,
        "cannot cancel",
        None,
        false,
    );
    let start = registry
        .spawn(uncancellable, |_| async {
            Ok(RuntimeBackgroundTaskOutcome::succeeded("done"))
        })
        .unwrap();
    assert!(matches!(
        registry.cancel(&start.task.id),
        Err(RuntimeBackgroundTaskControlError::NotCancellable(_))
    ));
    terminal(&registry, &start.task.id).await;
    assert!(matches!(
        registry.cancel(&start.task.id),
        Err(RuntimeBackgroundTaskControlError::NotRunning(_))
    ));
}
