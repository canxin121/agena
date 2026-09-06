use super::*;

async fn active_fixture() -> (Fixture, Arc<AgenaRuntime>) {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    assert!(!runtime.is_shutdown());
    (fixture, runtime)
}

pub(super) async fn build_candidate(
    runtime: &AgenaRuntime,
    original: &Arc<super::super::RuntimeSnapshot>,
) -> super::super::RuntimeSnapshot {
    super::super::RuntimeSnapshot::build_with_previous(
        original.generation() + 1,
        &runtime.inner.loader,
        &runtime.inner.load_request,
        runtime.workspace_root(),
        super::super::SnapshotDatabases {
            chat: runtime.inner.database.clone(),
            scheduler: runtime.inner.scheduler_database.clone(),
        },
        original.session_manager(),
        original.clone(),
        runtime.activities.monitor.clone(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn an_unpublished_candidate_does_not_reconfigure_the_shared_session_manager() {
    let (mut fixture, runtime) = active_fixture().await;
    let original = runtime.current_snapshot();
    let manager = original.session_manager().unwrap();
    let original_host = manager.tool_executor().plugin_manager().clone();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("zh-CN");
    let candidate = build_candidate(&runtime, &original).await;
    let manager_kept_original =
        Arc::ptr_eq(manager.tool_executor().plugin_manager(), &original_host);
    let candidate_host = Arc::downgrade(&candidate.plugin_manager());
    let shutdowns = fixture.observed.shutdowns.load(Ordering::SeqCst);
    drop(candidate);
    let cleanup = tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.observed.shutdowns.load(Ordering::SeqCst) == shutdowns {
            tokio::task::yield_now().await;
        }
    })
    .await;
    if let Some(host) = candidate_host.upgrade() {
        host.shutdown().await;
    }
    runtime.shutdown();
    assert!(Arc::ptr_eq(&original, &runtime.current_snapshot()));
    assert!(
        manager_kept_original,
        "building a candidate changed shared session execution before snapshot publication"
    );
    assert!(
        cleanup.is_ok(),
        "discarding a candidate must not retain its plugin through shared sessions"
    );
}

#[tokio::test]
async fn shutdown_before_final_publication_keeps_snapshot_sessions_and_process_slot() {
    let (mut fixture, runtime) = active_fixture().await;
    let gate = runtime.inner.control_state.reload_gate().acquire().await;
    let original = runtime.current_snapshot();
    let manager = original.session_manager().unwrap();
    let original_host = manager.tool_executor().plugin_manager().clone();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("zh-CN");
    let candidate = Arc::new(build_candidate(&runtime, &original).await);
    let shutdowns = fixture.observed.shutdowns.load(Ordering::SeqCst);
    runtime.shutdown();
    let outcome = runtime.publish_reload_candidate(
        original.clone(),
        candidate,
        crate::RuntimeReloadCause::Manual,
    );
    drop(gate);
    assert!(matches!(outcome, Err(crate::AppError::Cancelled)));
    assert!(Arc::ptr_eq(&original, &runtime.current_snapshot()));
    assert!(Arc::ptr_eq(
        manager.tool_executor().plugin_manager(),
        &original_host
    ));
    assert!(Arc::ptr_eq(
        &crate::current_plugin_host().unwrap(),
        &original_host
    ));
    tokio::time::timeout(WAIT, async {
        while fixture.observed.shutdowns.load(Ordering::SeqCst) == shutdowns {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a rejected completed candidate must release its initialized plugins");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publication_already_in_progress_finishes_before_shutdown_returns() {
    use std::sync::mpsc;
    use tracing_subscriber::prelude::*;

    // Pause the real publication at its tracing update, inside the lifecycle
    // guard and before the snapshot/session writes, without a production hook.
    struct PublicationPause {
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        seen: AtomicBool,
    }
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for PublicationPause {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event.metadata().target() == "agena_runtime::runtime::builder"
                && !self.seen.swap(true, Ordering::SeqCst)
            {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv_timeout(WAIT).unwrap();
            }
        }
    }

    let (mut fixture, runtime) = active_fixture().await;
    let gate = runtime.inner.control_state.reload_gate().acquire().await;
    let original = runtime.current_snapshot();
    let manager = original.session_manager().unwrap();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("zh-CN");
    let candidate = Arc::new(build_candidate(&runtime, &original).await);
    let candidate_host = candidate.plugin_manager();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let subscriber = tracing_subscriber::registry().with(PublicationPause {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        seen: AtomicBool::new(false),
    });
    let publisher_runtime = runtime.clone();
    let published_original = original.clone();
    let publisher = std::thread::spawn(move || {
        tracing::subscriber::with_default(subscriber, || {
            publisher_runtime.publish_reload_candidate(
                published_original,
                candidate,
                crate::RuntimeReloadCause::Manual,
            )
        })
    });
    entered_rx.recv_timeout(WAIT).unwrap();
    let still_original = Arc::ptr_eq(&original, &runtime.current_snapshot());
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
    let shutdown_runtime = runtime.clone();
    let executor = tokio::runtime::Handle::current();
    let shutdown = std::thread::spawn(move || {
        let _entered = executor.enter();
        started_tx.send(()).unwrap();
        shutdown_runtime.shutdown();
        stopped_tx
            .send(shutdown_runtime.current_snapshot().generation())
            .unwrap();
    });
    started_rx.recv_timeout(WAIT).unwrap();
    let early_shutdown = stopped_rx.recv_timeout(Duration::from_millis(100));
    let marked_shutdown_early = runtime.is_shutdown();
    release_tx.send(()).unwrap();
    let report = publisher.join().unwrap().unwrap();
    shutdown.join().unwrap();
    drop(gate);

    assert!(
        still_original,
        "the pause must precede snapshot publication"
    );
    assert!(matches!(
        early_shutdown,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(
        !marked_shutdown_early,
        "shutdown must wait for the admitted publication"
    );
    assert_eq!(stopped_rx.recv_timeout(WAIT).unwrap(), report.generation);
    assert_eq!(report.generation, original.generation() + 1);
    assert!(runtime.is_shutdown());
    assert!(Arc::ptr_eq(
        manager.tool_executor().plugin_manager(),
        &candidate_host
    ));
    assert!(Arc::ptr_eq(
        &crate::current_plugin_host().unwrap(),
        &candidate_host
    ));
}

#[tokio::test]
async fn direct_reload_after_shutdown_rejects_before_replacing_the_snapshot() {
    let (fixture, runtime) = active_fixture().await;
    let original = runtime.current_snapshot();
    runtime.shutdown();
    fixture.write_config("zh-CN");
    let result = tokio::time::timeout(WAIT, runtime.reload()).await.unwrap();
    let current = runtime.current_snapshot();
    current.plugin_manager().shutdown().await;
    assert!(
        matches!(result, Err(crate::AppError::Cancelled)),
        "a stopped runtime accepted direct reload: {result:?}"
    );
    assert!(Arc::ptr_eq(&original, &current));
    assert_eq!(fixture.observed.init_config.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn shutdown_inside_managed_reload_work_records_cancellation_without_failure() {
    let (fixture, runtime) = active_fixture().await;
    fixture.write_config("zh-CN");
    let worker_runtime = runtime.clone();
    let started = runtime
        .spawn_background_task(
            crate::RuntimeBackgroundTaskKind::RuntimeReload,
            crate::RuntimeBackgroundTaskOrigin::User,
            "managed reload interrupted during its poll",
            None,
            false,
            move |_| async move {
                // Shutdown arrives after the registry polls its cancellation
                // branch, but before the same work poll returns the reload result.
                worker_runtime.shutdown();
                worker_runtime.reload().await?;
                Ok(crate::RuntimeBackgroundTaskOutcome::succeeded("reloaded"))
            },
        )
        .unwrap();
    assert_cancelled_task(&runtime, &started.task.id).await;
}

#[tokio::test]
async fn shutdown_inside_service_reload_work_records_cancellation_without_failure() {
    let (_fixture, runtime) = active_fixture().await;
    let worker_runtime = runtime.clone();
    let started = crate::RuntimeControlService::start_background_task(
        runtime.as_ref(),
        crate::RuntimeBackgroundTaskKind::RuntimeReload,
        crate::RuntimeBackgroundTaskOrigin::User,
        "service reload interrupted during its poll".into(),
        None,
        false,
        Box::new(move |_| {
            Box::pin(async move {
                worker_runtime.shutdown();
                crate::RuntimeControlService::reload(worker_runtime.as_ref()).await?;
                Ok(crate::RuntimeBackgroundTaskOutcome::succeeded("reloaded"))
            })
        }),
    )
    .unwrap();
    assert_cancelled_task(&runtime, &started.task.id).await;
}

async fn assert_cancelled_task(runtime: &AgenaRuntime, task_id: &str) {
    let task = tokio::time::timeout(WAIT, async {
        loop {
            let task = runtime
                .background_tasks()
                .into_iter()
                .find(|task| task.id == task_id)
                .unwrap();
            if !task.is_running() {
                break task;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(runtime.is_shutdown());
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(task.status, crate::RuntimeBackgroundTaskStatus::Cancelled);
    assert!(task.failure.is_none());
}

#[tokio::test]
async fn shutdown_wakes_a_direct_reload_waiting_for_the_reload_gate() {
    let (_fixture, runtime) = active_fixture().await;
    let gate = runtime.inner.control_state.reload_gate().acquire().await;
    let (entered, polled) = tokio::sync::oneshot::channel();
    let reload_runtime = runtime.clone();
    let mut reload = tokio::spawn(async move {
        let future = reload_runtime.reload();
        tokio::pin!(future);
        assert!(futures_util::poll!(&mut future).is_pending());
        entered.send(()).unwrap();
        future.await
    });
    polled.await.unwrap();
    runtime.shutdown();
    let outcome = tokio::time::timeout(Duration::from_secs(1), &mut reload).await;
    if outcome.is_err() {
        reload.abort();
        let _ = reload.await;
    }
    drop(gate);
    runtime.current_snapshot().plugin_manager().shutdown().await;
    assert!(
        matches!(outcome, Ok(Ok(Err(crate::AppError::Cancelled)))),
        "shutdown must wake queued direct reload without acquiring the held gate: {outcome:?}"
    );
    assert_eq!(runtime.current_snapshot().generation(), 1);
}

#[tokio::test]
async fn shutdown_cancels_a_direct_candidate_build_and_keeps_it_unpublished() {
    let (mut fixture, runtime) = active_fixture().await;
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision":2});
    fixture.write_config("zh-CN");
    fixture.observed.provider_wait.store(true, Ordering::SeqCst);
    let reload_runtime = runtime.clone();
    let mut reload = tokio::spawn(async move { reload_runtime.reload().await });
    tokio::time::timeout(WAIT, fixture.observed.provider_entered.notified())
        .await
        .unwrap();
    let shutdowns = fixture.observed.shutdowns.load(Ordering::SeqCst);
    assert_eq!(runtime.current_snapshot().generation(), 1);
    runtime.shutdown();
    let outcome = tokio::time::timeout(Duration::from_secs(1), &mut reload).await;
    if outcome.is_err() {
        reload.abort();
        let _ = reload.await;
    }
    let cleanup = tokio::time::timeout(WAIT, async {
        while fixture.observed.shutdowns.load(Ordering::SeqCst) == shutdowns {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert!(
        matches!(outcome, Ok(Ok(Err(crate::AppError::Cancelled)))),
        "shutdown must cancel direct candidate composition: {outcome:?}"
    );
    assert!(
        cleanup.is_ok(),
        "cancelled candidate must release its initialized plugin"
    );
}
