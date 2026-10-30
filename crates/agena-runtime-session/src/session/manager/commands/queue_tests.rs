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
async fn generated_images_use_the_owning_workspace_not_server_default_or_effective_cwd() {
    let (manager, _) = fixture().await;
    let base = std::env::temp_dir().join(format!(
        "agena-owning-image-workspace-{}",
        uuid::Uuid::new_v4(),
    ));
    let owner = base.join("owning");
    let effective = base.join("child-cwd");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::create_dir_all(&effective).unwrap();
    let owner = std::fs::canonicalize(owner).unwrap();
    let effective = std::fs::canonicalize(effective).unwrap();
    let workspace = manager
        .kernel_runtime
        .workspaces
        .ensure_id(&owner.to_string_lossy())
        .await
        .unwrap();
    let session = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Different owning workspace".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let server_root = manager.tool_executor().workspace_root().to_path_buf();
    assert_ne!(server_root, owner);
    assert_ne!(effective, owner);

    for (index, cwd) in [None, Some(effective.clone())].into_iter().enumerate() {
        manager
            .kernel()
            .sessions
            .configure(
                RequestId::new(),
                session,
                SessionConfigurationChange {
                    effective_workspace_root: Some(cwd.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let resolved = manager.session_workspace_root(session.get()).await.unwrap();
        assert_eq!(resolved, owner);
        if let Some(cwd) = cwd {
            let context = manager.execution_context_value(session).await.unwrap();
            let scoped = manager
                .tool_executor()
                .for_session_context_async(&context)
                .await;
            assert_eq!(scoped.workspace_root(), cwd.as_path());
        }
        // Exercise the actual artifact writer with the owning-root resolver
        // used by both hosted provider rounds and the direct image host API.
        let call_id = format!("provider-hosted-image-workspace-{index}");
        let saved = agena_runtime_tools::persist_generated_image_artifact(
            &resolved,
            session.get(),
            &call_id,
            0,
            "image/png",
            None,
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==",
        )
        .await
        .unwrap()
        .unwrap();
        let expected = agena_runtime_tools::project_state_dir(&owner)
            .join("generated_images")
            .join(session.get().to_string());
        assert!(std::path::Path::new(&saved.path).starts_with(expected));
        assert_eq!(
            std::fs::metadata(&saved.path).unwrap().len(),
            saved.size_bytes
        );
        assert!(
            !std::path::Path::new(&saved.path)
                .starts_with(agena_runtime_tools::project_state_dir(&server_root))
        );
        assert!(
            !std::path::Path::new(&saved.path)
                .starts_with(agena_runtime_tools::project_state_dir(&effective))
        );
        assert_eq!(
            manager.tool_executor().workspace_root(),
            server_root.as_path()
        );
    }
    assert!(manager.session_workspace_root(i64::MAX).await.is_err());
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
    std::fs::remove_dir_all(agena_runtime_tools::project_state_dir(&owner)).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}

#[tokio::test]
async fn tool_authorization_uses_session_workspace_rules_without_default_fallback() {
    use agena_domain::{PermissionAction, PermissionDecision, PermissionMode, PermissionScope};
    use agena_storage::PersistedPermissionRule;

    let (manager, session_a) = fixture().await;
    let workspace_a = manager
        .kernel()
        .reader
        .metadata(session_a)
        .await
        .unwrap()
        .workspace_id;
    // This is the server's already-resolved default workspace. It deliberately
    // differs from B, whose persisted rule must govern B's tools.
    manager.workspace.set(workspace_a).unwrap();
    let workspace_b = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/queue-tests/other-workspace")
        .await
        .unwrap();
    let session_b = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace_b,
            "Workspace permission isolation".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let action = PermissionAction::Tool {
        tool_name: "workspace-scope-probe".into(),
        qualifier: None,
    };
    let action_key = super::super::execution::permission_key(&action).unwrap();
    let mut deny_rule_id = None;
    for (workspace, mode) in [
        (workspace_a, PermissionMode::Allow),
        (workspace_b, PermissionMode::Deny),
    ] {
        let (record, _) = manager
            .kernel_runtime
            .permissions
            .upsert(&PersistedPermissionRule {
                id: None,
                created_at_ms: None,
                updated_at_ms: None,
                action_key: action_key.clone(),
                mode,
                scope: PermissionScope::Workspace,
                session_id: None,
                workspace_id: Some(workspace),
                source: "workspace-isolation-test".into(),
                reason: Some("Workspace B denies this operation".into()),
                operator: None,
                revoked_at_ms: None,
                revoked_reason: None,
                revoked_by: None,
            })
            .await
            .unwrap();
        if workspace == workspace_b {
            deny_rule_id = Some(record.id);
        }
    }

    for (session, should_deny) in [(session_a, false), (session_b, true)] {
        let execution = send(&manager, session).await;
        let attempt = manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "workspace-permission-probe".into(),
                    name: "workspace-scope-probe".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let state = manager.state.load_full();
        let register = |session| {
            let control = Arc::new(RunControl {
                session,
                execution,
                model: ModelRef::new("test", "test"),
                state: state.clone(),
                cancel: CancellationToken::new(),
                phase: StdMutex::new(agena_domain::ExecutionPhase::ExecutingTools),
                done: Notify::new(),
                budget: None,
                failure: Default::default(),
                settled: std::sync::atomic::AtomicBool::new(false),
            });
            manager.tools.lock().unwrap().insert(
                (session.get(), tool.id().get()),
                LiveTool {
                    tool: tool.clone(),
                    control: control.clone(),
                    permit: Arc::new(tokio::sync::Mutex::new(None)),
                    permissions: Default::default(),
                },
            );
            control
        };
        let control = register(session);
        let checks = || {
            vec![agena_tool::ToolPermissionCheck {
                action: action.clone(),
                decision: PermissionDecision::Allow,
            }]
        };
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            manager.authorize_checks(&tool, checks(), &state.executor, &control, &state),
        )
        .await
        .expect("persisted allow/deny rules must not request interactive approval");
        if should_deny {
            let Err(ToolError::PolicyDenied(denied)) = result else {
                panic!("workspace B must use its own deny, not default A's allow");
            };
            assert_eq!(denied.scope, Some(PermissionScope::Workspace));
            assert_eq!(denied.rule_id, deny_rule_id);
            assert_eq!(denied.reason, "Workspace B denies this operation");
        } else {
            result.unwrap();
        }

        // Even with a live-call slot and an allowed default workspace, an
        // unknown owning session must fail instead of selecting that default.
        let missing: SessionId = i64::MAX.try_into().unwrap();
        let missing_control = register(missing);
        assert!(
            manager
                .authorize_checks(&tool, checks(), &state.executor, &missing_control, &state)
                .await
                .is_err()
        );
        manager
            .tools
            .lock()
            .unwrap()
            .remove(&(missing.get(), tool.id().get()));
        manager
            .tools
            .lock()
            .unwrap()
            .remove(&(session.get(), tool.id().get()));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: if should_deny {
                    ToolPhase::Failed
                } else {
                    ToolPhase::Succeeded
                },
                value: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
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
