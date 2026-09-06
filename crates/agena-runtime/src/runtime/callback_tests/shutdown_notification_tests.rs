use super::*;

async fn active_session(fixture: &Fixture, runtime: &AgenaRuntime) -> i64 {
    let manager = runtime.session_manager().unwrap();
    let session = manager
        .create_session(crate::SessionCreateRequest {
            title: "shutdown notification fixture".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    crate::SessionExecutionCommandService::submit_user_run(
        manager.as_ref(),
        crate::SessionUserRunRequest::new(
            session.id,
            crate::SessionRunOptions {
                model: agena_domain::ModelRef::new("fixture", "held-before-provider"),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
            agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "hold this execution in the prompt hook".into(),
            }]),
        ),
    )
    .await
    .unwrap();
    tokio::time::timeout(WAIT, fixture.observed.prompt_entered.notified())
        .await
        .unwrap();
    assert!(manager.is_run_active(session.id).await);
    session.id
}

async fn finish_sessions(fixture: &Fixture, runtime: &AgenaRuntime, sessions: &[i64]) {
    fixture.observed.session_end_release.cancel();
    let manager = runtime.session_manager().unwrap();
    for &session_id in sessions {
        manager
            .cancel_active_execution_with_outcome(session_id)
            .await
            .unwrap();
    }
    fixture.observed.prompt_release.cancel();
}

async fn duplicate_session_end(fixture: &Fixture) -> bool {
    tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if fixture.observed.session_ends.lock().unwrap().len() > 1 {
                break;
            }
            fixture.observed.session_end_entered.notified().await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn repeated_shutdown_schedules_one_active_session_end() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let session_id = active_session(&fixture, &runtime).await;
    runtime.shutdown();
    tokio::time::timeout(WAIT, fixture.observed.session_end_entered.notified())
        .await
        .unwrap();
    assert!(runtime.is_shutdown());
    // The first HTTP notification is still awaiting its response, so the
    // active session cannot disappear before repeated calls enumerate it.
    for _ in 0..8 {
        runtime.shutdown();
    }
    let duplicate = duplicate_session_end(&fixture).await;
    finish_sessions(&fixture, &runtime, &[session_id]).await;
    assert!(
        !duplicate,
        "repeated shutdown sent duplicate session.end hooks"
    );
}

#[tokio::test]
async fn concurrent_first_shutdown_has_one_notification_owner() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let session_id = active_session(&fixture, &runtime).await;
    let barrier = Arc::new(std::sync::Barrier::new(16));
    let handle = tokio::runtime::Handle::current();
    std::thread::scope(|scope| {
        for _ in 0..16 {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            let handle = handle.clone();
            scope.spawn(move || {
                let _entered = handle.enter();
                barrier.wait();
                runtime.shutdown();
            });
        }
    });
    tokio::time::timeout(WAIT, fixture.observed.session_end_entered.notified())
        .await
        .unwrap();
    let duplicate = duplicate_session_end(&fixture).await;
    finish_sessions(&fixture, &runtime, &[session_id]).await;
    assert!(
        !duplicate,
        "concurrent first shutdowns each scheduled a broadcast"
    );
}

#[tokio::test]
async fn shutdown_without_executor_closes_control_before_one_diagnostic() {
    use tracing_subscriber::prelude::*;

    struct WarningObserver {
        runtime: Arc<AgenaRuntime>,
        stopped: Arc<Mutex<Vec<bool>>>,
    }
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarningObserver {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event.metadata().target() == "agena_plugin_host::session_end" {
                self.stopped
                    .lock()
                    .unwrap()
                    .push(self.runtime.is_shutdown());
            }
        }
    }

    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let observer = WarningObserver {
        runtime: runtime.clone(),
        stopped: stopped.clone(),
    };
    std::thread::spawn(move || {
        tracing::subscriber::with_default(tracing_subscriber::registry().with(observer), || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            runtime.shutdown();
            runtime.shutdown();
        });
    })
    .join()
    .unwrap();
    assert_eq!(*stopped.lock().unwrap(), vec![true]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_notification_waits_for_lifecycle_publication_and_closed_control() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let session_id = active_session(&fixture, &runtime).await;
    let (held_tx, held_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let holder_runtime = runtime.clone();
    let holder = std::thread::spawn(move || {
        let _publication = holder_runtime
            .inner
            .control_state
            .task_control()
            .running_guard()
            .unwrap();
        held_tx.send(()).unwrap();
        release_rx.recv_timeout(WAIT).unwrap();
    });
    held_rx.recv_timeout(WAIT).unwrap();
    let shutdown_runtime = runtime.clone();
    let handle = tokio::runtime::Handle::current();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let shutdown = std::thread::spawn(move || {
        let _entered = handle.enter();
        started_tx.send(()).unwrap();
        shutdown_runtime.shutdown();
    });
    started_rx.recv_timeout(WAIT).unwrap();
    let early_notification = tokio::time::timeout(
        Duration::from_millis(300),
        fixture.observed.session_end_entered.notified(),
    )
    .await
    .is_ok();
    let stopped_while_publication_held = runtime.is_shutdown();
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    shutdown.join().unwrap();
    if !early_notification {
        tokio::time::timeout(WAIT, fixture.observed.session_end_entered.notified())
            .await
            .unwrap();
    }
    finish_sessions(&fixture, &runtime, &[session_id]).await;
    assert!(!stopped_while_publication_held);
    assert!(runtime.is_shutdown());
    assert!(
        !early_notification,
        "session.end ran before lifecycle publication finished and control closed"
    );
}

async fn empty_successor_host(runtime: &AgenaRuntime) -> Arc<agena_plugin_host::PluginHost> {
    agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins: Vec::new(),
        config: Default::default(),
        workspace_root: runtime.workspace_root().to_owned(),
        agena_version: "notification-successor".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap()
}

fn replace_manager_host(runtime: &AgenaRuntime, next: Arc<agena_plugin_host::PluginHost>) {
    let executor = crate::tool::ToolExecutor::new(
        runtime.workspace_root().to_owned(),
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        next.clone(),
        None,
        None,
        None,
    );
    runtime.session_manager().unwrap().reconfigure(
        runtime.current_snapshot().provider_registry(),
        crate::ContextGovernor::new(agena_domain::ContextPolicy::default()),
        crate::session::SessionProcessor::new(next),
        executor,
        crate::RuntimeSessionManagerConfig::default(),
    );
}

fn observed_session_ids(fixture: &Fixture) -> Vec<i64> {
    let mut ids = fixture
        .observed
        .session_ends
        .lock()
        .unwrap()
        .iter()
        .map(|input| input.session_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn active_session_end_batch_retains_its_host_across_reconfiguration() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let mut sessions = vec![
        active_session(&fixture, &runtime).await,
        active_session(&fixture, &runtime).await,
    ];
    sessions.sort_unstable();
    let successor = empty_successor_host(&runtime).await;
    let manager = runtime.session_manager().unwrap();
    let broadcast = tokio::spawn(async move {
        manager
            .broadcast_active_session_end(agena_plugin_host::SessionEndReason::Other)
            .await;
    });
    tokio::time::timeout(WAIT, fixture.observed.session_end_entered.notified())
        .await
        .unwrap();
    replace_manager_host(&runtime, successor.clone());
    fixture.observed.session_end_release.cancel();
    tokio::time::timeout(WAIT, broadcast)
        .await
        .unwrap()
        .unwrap();
    let observed = observed_session_ids(&fixture);
    finish_sessions(&fixture, &runtime, &sessions).await;
    successor.shutdown().await;
    assert_eq!(
        observed, sessions,
        "one broadcast mixed predecessor and successor hosts"
    );
}

#[tokio::test]
async fn queued_session_end_captures_its_host_before_the_first_poll() {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let session_id = active_session(&fixture, &runtime).await;
    let successor = empty_successor_host(&runtime).await;
    let manager = runtime.session_manager().unwrap();
    let broadcast =
        manager.broadcast_active_session_end(agena_plugin_host::SessionEndReason::Other);
    replace_manager_host(&runtime, successor.clone());
    fixture.observed.session_end_release.cancel();
    tokio::time::timeout(WAIT, broadcast).await.unwrap();
    let observed = observed_session_ids(&fixture);
    finish_sessions(&fixture, &runtime, &[session_id]).await;
    successor.shutdown().await;
    assert_eq!(
        observed,
        vec![session_id],
        "queued broadcast selected a later execution configuration"
    );
}
