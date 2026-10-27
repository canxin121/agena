use super::*;
use crate::SessionExecutionControl;

async fn fixture() -> (SessionManager, SessionId) {
    fixture_with_config(RuntimeSessionManagerConfig::default()).await
}

async fn fixture_with_config(config: RuntimeSessionManagerConfig) -> (SessionManager, SessionId) {
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let workspace = kernel.workspaces.ensure_id("/queue-tests").await.unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Queue tests".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let plugins = agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins: vec![],
        config: Default::default(),
        workspace_root: "/queue-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        "/queue-tests",
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let manager = SessionManager::new(
        kernel,
        Arc::new(ProviderRegistry::default()),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        config,
    );
    (manager, session)
}

async fn send(manager: &SessionManager, session: SessionId) -> ExecutionId {
    manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text { text: "hi".into() }]),
        )
        .await
        .unwrap()
        .value
        .id
}

#[tokio::test]
async fn prompt_projection_preserves_overrides_without_a_usable_provider() {
    let (manager, session) = fixture_with_config(RuntimeSessionManagerConfig {
        additional_system_prompt: Some("Inherited instructions".into()),
        ..Default::default()
    })
    .await;
    for model in [
        None,
        Some(agena_domain::ModelRef::new(
            "missing-provider",
            "missing-model",
        )),
    ] {
        manager
            .kernel()
            .sessions
            .update_selection(RequestId::new(), session, SelectionChange::Model(model))
            .await
            .unwrap();
        for raw in [
            None,
            Some(String::new()),
            Some("Session instructions".to_owned()),
        ] {
            manager
                .kernel()
                .sessions
                .configure(
                    RequestId::new(),
                    session,
                    SessionConfigurationChange {
                        additional_system_prompt: Some(raw.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let snapshot = manager
                .kernel()
                .reader
                .control_snapshot(session, 1)
                .await
                .unwrap();
            let context =
                crate::SessionQueryService::execution_context(&manager, &snapshot).unwrap();
            assert_eq!(context.additional_system_prompt, raw);
            assert_eq!(
                context.effective_additional_system_prompt,
                raw.clone()
                    .or_else(|| Some("Inherited instructions".into()))
            );
            assert_eq!(
                manager
                    .execution_context_value(session)
                    .await
                    .unwrap()
                    .additional_system_prompt,
                raw
            );
        }
    }
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn stopping_without_queued_input_allows_the_next_send_to_start() {
    // Exercise both public cancellation routes against the real writer fence.
    for exact in [false, true] {
        let (manager, session) = fixture().await;
        let execution = send(&manager, session).await;
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap();
        if exact {
            SessionExecutionControl::cancel_execution_with_outcome(
                &manager,
                session.get(),
                execution,
            )
            .await
            .unwrap();
        } else {
            manager
                .cancel_session_with_outcome(session.get())
                .await
                .unwrap();
        }
        assert_eq!(
            manager
                .kernel()
                .reader
                .execution(session, execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );
        let next = send(&manager, session).await;
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, next)
            .await
            .unwrap();
        manager
            .kernel_runtime
            .ports
            .clone()
            .shutdown()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn stopping_with_queued_input_keeps_it_paused_and_intact() {
    let (manager, session) = fixture().await;
    let execution = send(&manager, session).await;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, execution)
        .await
        .unwrap();
    let queued = send(&manager, session).await;
    manager
        .cancel_session_with_outcome(session.get())
        .await
        .unwrap();
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(session, queued)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Queued
    );
    assert!(matches!(
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, queued)
            .await,
        Err(KernelError::QueuePaused)
    ));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn stopping_an_empty_session_preserves_a_manual_pause() {
    let (manager, session) = fixture().await;
    manager
        .set_message_queue_paused(session.get(), true)
        .await
        .unwrap();
    manager
        .cancel_session_with_outcome(session.get())
        .await
        .unwrap();
    let next = send(&manager, session).await;
    assert!(matches!(
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, next)
            .await,
        Err(KernelError::QueuePaused)
    ));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}
