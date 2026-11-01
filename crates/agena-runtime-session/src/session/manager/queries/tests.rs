use super::*;
use agena_domain::{AgentStatus, PermissionReplyKind, SessionRelationKind};

async fn fixture() -> (SessionManager, SessionId) {
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let workspace = kernel.workspaces.ensure_id("/query-tests").await.unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Query tests".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let plugins = agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins: vec![],
        config: Default::default(),
        workspace_root: "/query-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        "/query-tests",
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    (
        SessionManager::new(
            kernel,
            Arc::new(ProviderRegistry::default()),
            ContextGovernor::new(Default::default()),
            SessionProcessor::new(plugins),
            executor,
            Default::default(),
        ),
        session,
    )
}

async fn send(manager: &SessionManager, session: SessionId) -> ExecutionView {
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
}

async fn assert_workflow(manager: &SessionManager, session: SessionId, expected: WorkflowState) {
    assert_eq!(
        manager.get_session(session.get()).await.unwrap().workflow,
        expected,
        "runtime session {}",
        session.get(),
    );
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(session, 128)
        .await
        .unwrap();
    assert_eq!(
        SessionQueryService::execution_context(manager, &snapshot)
            .unwrap()
            .workflow_state,
        expected,
        "execution projection {}",
        session.get(),
    );
}

#[tokio::test]
async fn related_sessions_project_their_own_tool_and_interaction_workflow() {
    let (manager, root) = fixture().await;
    let seed = send(&manager, root).await;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), root, seed.id)
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(root)
        .await
        .unwrap()
        .workspace_id;
    let mut sessions = vec![root];
    let mut parent = root;
    for _ in 0..2 {
        parent = manager
            .kernel()
            .sessions
            .fork(RequestId::new(), parent, seed.message_id)
            .await
            .unwrap()
            .value
            .id;
        sessions.push(parent);
    }
    for relation_kind in [SessionRelationKind::Child, SessionRelationKind::Subagent] {
        sessions.push(
            manager
                .kernel()
                .sessions
                .create_configured(
                    RequestId::new(),
                    workspace,
                    relation_kind.as_str().into(),
                    SessionConfiguration {
                        parent_session_id: Some(parent),
                        relation_kind,
                        agent: relation_kind
                            .is_subagent()
                            .then(|| SessionAgentConfiguration {
                                name: "workflow_probe".into(),
                                status: AgentStatus::Created,
                                started_at_ms: None,
                                finished_at_ms: None,
                                failure: None,
                                limits: Default::default(),
                            }),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value
                .id,
        );
    }
    let mut active = Vec::new();
    for &session in &sessions {
        assert_workflow(&manager, session, WorkflowState::Quiescent).await;
        let execution = send(&manager, session).await;
        let attempt = manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, execution.id)
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
                    call_id: "probe".into(),
                    name: "workflow-probe".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        assert_workflow(&manager, session, WorkflowState::ToolPending).await;
        active.push((session, attempt, round, tool));
    }

    for (index, (session, attempt, round, tool)) in active.iter().enumerate() {
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: serde_json::to_string(&PermissionAction::Tool {
                        tool_name: "workflow-probe".into(),
                        qualifier: None,
                    })
                    .unwrap(),
                    description: "Confirm the owning session".into(),
                },
            )
            .await
            .unwrap()
            .value;
        for (other_index, &other) in sessions.iter().enumerate() {
            let expected = if other == *session {
                WorkflowState::AwaitingInteraction
            } else if other_index < index {
                WorkflowState::Quiescent
            } else {
                WorkflowState::ToolPending
            };
            assert_workflow(&manager, other, expected).await;
        }
        manager
            .kernel()
            .sessions
            .reply(
                RequestId::new(),
                *session,
                interaction,
                InteractionDecision::Permission {
                    reply: agena_domain::PermissionReply {
                        request_id: interaction.get().to_string(),
                        kind: PermissionReplyKind::AllowOnce,
                        reason: None,
                        scope: None,
                    },
                    allowed: true,
                },
                None,
            )
            .await
            .unwrap();
        assert_workflow(&manager, *session, WorkflowState::ToolPending).await;
        let snapshot = manager
            .kernel()
            .reader
            .control_snapshot(*session, 128)
            .await
            .unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"ok": true}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert_workflow(&manager, *session, WorkflowState::Quiescent).await;
        assert_eq!(
            SessionQueryService::execution_context(&manager, &snapshot)
                .unwrap()
                .workflow_state,
            WorkflowState::ToolPending,
            "a captured control snapshot must not reread later state",
        );
    }
}

#[tokio::test]
async fn session_tool_context_defaults_to_its_owning_workspace() {
    let (manager, _) = fixture().await;
    let workspace_path = std::path::PathBuf::from("/query-tests-secondary");
    let workspace = manager
        .kernel_runtime
        .workspaces
        .ensure_id(workspace_path.to_str().unwrap())
        .await
        .unwrap();
    let root = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Other workspace".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let seed = send(&manager, root).await;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), root, seed.id)
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let mut current = root;
    for _ in 0..3 {
        let session = manager.get_session(current.get()).await.unwrap();
        let context = manager.execution_context_value(current).await.unwrap();
        for execution in [&session.execution, &context] {
            let scoped = manager
                .tool_executor()
                .for_session_context_async(execution)
                .await;
            assert_eq!(scoped.workspace_root(), workspace_path);
            assert_eq!(execution.scope_session_id, Some(current.get()));
        }
        current = manager
            .kernel()
            .sessions
            .fork(RequestId::new(), current, seed.message_id)
            .await
            .unwrap()
            .value
            .id;
    }
    let configured = workspace_path.join("task-cwd");
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            current,
            SessionConfigurationChange {
                effective_workspace_root: Some(Some(configured.clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let context = manager.execution_context_value(current).await.unwrap();
    let scoped = manager
        .tool_executor()
        .for_session_context_async(&context)
        .await;
    assert_eq!(scoped.workspace_root(), configured);
    assert_eq!(
        manager.tool_executor().workspace_root(),
        std::path::Path::new("/query-tests")
    );
}
