use super::*;
use agena_domain::{Model, ModelId};
use agena_runtime_provider::{
    ProviderError,
    provider::{CompletionResponse, ModelRuntime},
};

struct ResumeProvider {
    model: ModelId,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl ModelRuntime for ResumeProvider {
    fn id(&self) -> &str {
        "queue-resume-test"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn prompt_cache_shape(&self, _: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id(),
            [("destination_revision", "queue-resume-test")],
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _: agena_provider::CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(ProviderError::Config(
            "intentional queue resume test failure".into(),
        ))
    }
}

async fn fixture() -> (SessionManager, SessionId) {
    fixture_with_config(RuntimeSessionManagerConfig::default()).await
}

async fn fixture_with_config(config: RuntimeSessionManagerConfig) -> (SessionManager, SessionId) {
    fixture_with_providers(config, ProviderRegistry::default()).await
}

async fn fixture_with_providers(
    config: RuntimeSessionManagerConfig,
    providers: ProviderRegistry,
) -> (SessionManager, SessionId) {
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
        Arc::new(providers),
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
async fn resuming_during_paused_supervisor_retirement_does_not_strand_the_same_attempt() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut providers = ProviderRegistry::new();
    providers.register(ResumeProvider {
        model: ModelId::new("test"),
        calls: calls.clone(),
    });
    let (manager, session) = fixture_with_providers(Default::default(), providers).await;
    let model = ModelRef::new("queue-resume-test", "test");
    manager
        .kernel()
        .sessions
        .update_selection(
            RequestId::new(),
            session,
            SelectionChange::Model(Some(model.clone())),
        )
        .await
        .unwrap();
    manager
        .set_message_queue_paused(session.get(), true)
        .await
        .unwrap();
    let execution = send(&manager, session).await;
    let view = manager
        .kernel()
        .reader
        .execution(session, execution)
        .await
        .unwrap();
    let previous = Arc::new(RunControl {
        session,
        execution,
        generation: view.generation,
        model,
        state: manager.state.load_full(),
        cancel: CancellationToken::new(),
        phase: StdMutex::new(agena_domain::ExecutionPhase::Starting),
        done: Notify::new(),
        budget: None,
        failure: Default::default(),
        settled: std::sync::atomic::AtomicBool::new(false),
    });
    manager
        .active
        .lock()
        .unwrap()
        .insert(execution, previous.clone());
    let dispatch = manager.session_dispatch(session);
    *dispatch.tail.lock().unwrap() = Arc::downgrade(&previous);
    let driver_gate = dispatch.execution.lock().await;
    let permit = manager.reserve_execution().unwrap();
    let _other_admission = manager
        .admission
        .clone()
        .acquire_many_owned(manager.admission.available_permits() as u32)
        .await
        .unwrap();

    // The real serialized writer has rejected this supervisor's claim. Hold
    // its still-live registration while an explicit resume observes it.
    assert!(matches!(
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, execution,)
            .await,
        Err(KernelError::QueuePaused)
    ));
    manager
        .set_message_queue_paused(session.get(), false)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        manager.active.lock().unwrap().get(&execution).unwrap(),
        &previous
    ));
    manager.retire_supervised_run(&previous, permit, true).await;

    let replacement = manager
        .active
        .lock()
        .unwrap()
        .get(&execution)
        .cloned()
        .expect("resume must replace the retired supervisor without another send or recovery scan");
    assert!(!Arc::ptr_eq(&previous, &replacement));
    assert_eq!(
        replacement.generation, previous.generation,
        "resume must not retry or replace the durable attempt"
    );
    assert!(previous.settled.load(std::sync::atomic::Ordering::Acquire));
    drop(driver_gate);
    tokio::time::timeout(Duration::from_secs(5), replacement.wait_settled())
        .await
        .expect("the resumed attempt must make progress immediately");
    let current = manager
        .kernel()
        .reader
        .execution(session, execution)
        .await
        .unwrap();
    assert_eq!(current.generation, previous.generation);
    assert_eq!(current.phase, ExecutionPhase::Failed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(!manager.active.lock().unwrap().contains_key(&execution));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
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
                generation: attempt.generation(),
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
async fn prompt_projection_preserves_the_session_layer_without_a_usable_provider() {
    fn session_layer(body: &str) -> agena_prompt_layers::PromptLayer {
        let id = agena_prompt_layers::FragmentId::parse("session-note").unwrap();
        agena_prompt_layers::PromptLayer {
            mode: agena_prompt_layers::LayerMode::Append,
            order: vec![id.clone()],
            disabled: Vec::new(),
            inline: vec![agena_prompt_layers::PromptFragment::new(id, body)],
        }
    }

    let (manager, session) = fixture_with_config(RuntimeSessionManagerConfig::default()).await;
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
        for body in [None, Some("Session instructions")] {
            let layer = body.map(session_layer).unwrap_or_default();
            manager
                .kernel()
                .sessions
                .configure(
                    RequestId::new(),
                    session,
                    SessionConfigurationChange {
                        prompts: Some(layer.clone()),
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
            assert_eq!(context.prompts, layer);
            assert_eq!(
                manager
                    .execution_context_value(session)
                    .await
                    .unwrap()
                    .prompts,
                layer
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
