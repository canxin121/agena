use super::*;
use agena_domain::{PermissionDecision, StructuredObject, ToolApiCall, ToolApiFunction};
use agena_provider::CompletionInputPart;
use serde_json::Value;

struct Fixture {
    manager: SessionManager,
    session: SessionId,
    attempt: AttemptHandle,
    model: ModelRef,
    workspace_root: std::path::PathBuf,
}

impl Fixture {
    async fn new_subagent() -> Self {
        let mut fixture = Self::new().await;
        let workspace = fixture.manager.current_workspace_id().await.unwrap();
        let session = fixture
            .manager
            .kernel()
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Subagent interaction tests".into(),
                SessionConfiguration {
                    parent_session_id: Some(fixture.session),
                    relation_kind: agena_domain::SessionRelationKind::Subagent,
                    agent: Some(SessionAgentConfiguration {
                        name: "child".into(),
                        status: agena_domain::AgentStatus::Created,
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
            .id;
        let execution = fixture
            .manager
            .kernel()
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Complete the delegated task.".into(),
                }]),
            )
            .await
            .unwrap()
            .value
            .id;
        fixture.attempt = fixture
            .manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        fixture.session = session;
        fixture
    }

    fn register_live_call(&self, tool: &ToolHandle) -> Arc<RunControl> {
        let state = self.manager.state.load_full();
        let control = Arc::new(RunControl {
            session: self.session,
            execution: self.attempt.execution_id(),
            model: self.model.clone(),
            state,
            cancel: CancellationToken::new(),
            phase: StdMutex::new(agena_domain::ExecutionPhase::ExecutingTools),
            done: Notify::new(),
            budget: None,
            failure: Default::default(),
            settled: Default::default(),
        });
        self.manager
            .tools
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                (self.session.get(), tool.id().get()),
                LiveTool {
                    tool: tool.clone(),
                    control: control.clone(),
                    permit: Default::default(),
                    permissions: Default::default(),
                },
            );
        control
    }

    async fn scoped_executor(&self) -> ToolExecutor {
        let context = self
            .manager
            .execution_context_value(self.session)
            .await
            .unwrap();
        self.manager
            .state
            .load_full()
            .executor
            .for_session_context_async(&context)
            .await
    }

    async fn assert_no_pending_interaction(&self) {
        assert!(
            self.manager
                .kernel()
                .reader
                .pending_interactions(self.session, None, 10)
                .await
                .unwrap()
                .interactions
                .is_empty()
        );
    }

    async fn new() -> Self {
        let workspace_root =
            std::env::temp_dir().join(format!("agena-tool-api-tests-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&workspace_root).await.unwrap();
        let workspace_root = tokio::fs::canonicalize(workspace_root).await.unwrap();
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let workspace = kernel
            .workspaces
            .ensure_id(&workspace_root.to_string_lossy())
            .await
            .unwrap();
        let session = kernel
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Tool API tests".into(),
                Default::default(),
            )
            .await
            .unwrap()
            .value
            .id;
        let execution = kernel
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text { text: "hi".into() }]),
            )
            .await
            .unwrap()
            .value
            .id;
        let attempt = kernel
            .ports
            .executor
            .claim_next(RequestId::new(), session, execution)
            .await
            .unwrap()
            .value;
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                static_plugins: vec![],
                config: Default::default(),
                workspace_root: workspace_root.clone(),
                agena_version: "test".into(),
                callback_base_url: None,
                host_client: None,
                previous: None,
                previous_plugins: Default::default(),
            })
            .await
            .unwrap();
        let executor = ToolExecutor::new(
            workspace_root.clone(),
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
            RuntimeSessionManagerConfig::default(),
        );
        Self {
            manager,
            session,
            attempt,
            model: ModelRef::new("fixture", "model"),
            workspace_root,
        }
    }

    async fn begin_call(&self, name: &str, input: Value) -> ToolHandle {
        let round = self
            .attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: format!("call_{}", round.id().get()),
                    name: name.into(),
                    input,
                },
            )
            .await
            .unwrap()
            .value;
        round.finish(RequestId::new(), None, true).await.unwrap();
        tool
    }

    async fn dispatch(&self, function: ToolApiFunction, arguments: Value) -> ToolId {
        let tool = self
            .begin_call(function.function_name(), arguments.clone())
            .await;
        let id = tool.id();
        let state = self.manager.state.load_full();
        let control = Arc::new(RunControl {
            session: self.session,
            execution: self.attempt.execution_id(),
            model: self.model.clone(),
            state: state.clone(),
            cancel: CancellationToken::new(),
            phase: StdMutex::new(agena_domain::ExecutionPhase::ExecutingTools),
            done: Notify::new(),
            budget: None,
            failure: Default::default(),
            settled: Default::default(),
        });
        self.manager
            .dispatch_tool_api(
                KernelToolApiCall {
                    tool,
                    call: ToolApiCall {
                        function,
                        arguments: StructuredObject::try_from(arguments).unwrap(),
                    },
                    error: None,
                    signature: String::new(),
                },
                state.executor.clone(),
                control,
                state,
                self.model.clone(),
            )
            .await
            .unwrap();
        id
    }

    async fn prompt(&self) -> crate::session::kernel::KernelPrompt {
        let round = self
            .attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        self.manager
            .prompt_hydrator
            .hydrate_with_executor(
                &self.manager.kernel().reader,
                round.prompt().await.unwrap(),
                &self.model,
                false,
                &self.manager.state.load_full().executor,
                None,
            )
            .await
            .unwrap()
    }

    async fn shutdown(self) {
        self.manager
            .kernel_runtime
            .ports
            .clone()
            .shutdown()
            .await
            .unwrap();
        tokio::fs::remove_dir_all(&self.workspace_root)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn subagent_host_question_does_not_create_an_interaction() {
    let fixture = Fixture::new_subagent().await;
    let tool = fixture.begin_call("fixture.ask", json!({})).await;
    let control = fixture.register_live_call(&tool);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.manager.request_host_user_input(
            fixture.session.get(),
            tool.id().get(),
            crate::part::AskUserToolInput {
                title: "User decision".into(),
                kind: "question".into(),
                body_markdown: String::new(),
                auto_resolution_ms: None,
                questions: Vec::new(),
            },
        ),
    )
    .await
    .expect("subagent input must fail without waiting for the user");
    match result {
        Err(AppError::CapabilityUnavailable(error)) => {
            assert_eq!(error.capability, "user_interaction");
            assert!(error.reason.contains("parent agent"));
        }
        other => panic!("expected a capability error, got {other:?}"),
    }
    fixture.assert_no_pending_interaction().await;
    assert_eq!(
        *control.phase.lock().unwrap(),
        agena_domain::ExecutionPhase::ExecutingTools
    );
    fixture.shutdown().await;
}

fn permission_check(name: &str, decision: PermissionDecision) -> agena_tool::ToolPermissionCheck {
    agena_tool::ToolPermissionCheck {
        action: agena_domain::PermissionAction::Tool {
            tool_name: name.into(),
            qualifier: None,
        },
        decision,
    }
}

#[tokio::test]
async fn subagent_permission_question_does_not_create_an_interaction() {
    let fixture = Fixture::new_subagent().await;
    let tool = fixture.begin_call("fixture.run", json!({})).await;
    let control = fixture.register_live_call(&tool);
    let executor = fixture.scoped_executor().await;
    assert!(executor.session_relation_kind().is_subagent());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.manager.authorize_checks(
            &tool,
            vec![permission_check(
                "fixture.requires_approval",
                PermissionDecision::Ask {
                    reason: "This action requires user approval.".into(),
                },
            )],
            &executor,
            &control,
            &control.state,
        ),
    )
    .await
    .expect("subagent approval must fail without waiting for the user");
    match result {
        Err(ToolError::CapabilityUnavailable(error)) => {
            assert_eq!(error.capability, "user_interaction");
            assert!(error.reason.contains("requires user approval"));
            assert!(error.reason.contains("parent agent"));
        }
        other => panic!("expected a capability error, got {other:?}"),
    }
    fixture.assert_no_pending_interaction().await;
    assert_eq!(
        *control.phase.lock().unwrap(),
        agena_domain::ExecutionPhase::ExecutingTools
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn subagent_permission_grants_and_denials_are_preserved() {
    let fixture = Fixture::new_subagent().await;
    let tool = fixture.begin_call("fixture.run", json!({})).await;
    let control = fixture.register_live_call(&tool);
    let executor = fixture.scoped_executor().await;
    for decision in [
        PermissionDecision::Allow,
        PermissionDecision::Ask {
            reason: "Reuse the existing grant for this action.".into(),
        },
    ] {
        fixture
            .manager
            .authorize_checks(
                &tool,
                vec![permission_check("fixture.granted", decision)],
                &executor,
                &control,
                &control.state,
            )
            .await
            .unwrap();
    }

    let denied = fixture
        .manager
        .authorize_checks(
            &tool,
            vec![permission_check(
                "fixture.granted",
                PermissionDecision::Deny {
                    reason: "A new deny must override the earlier grant.".into(),
                },
            )],
            &executor,
            &control,
            &control.state,
        )
        .await;
    assert!(matches!(denied, Err(ToolError::PolicyDenied(_))));

    let other_action = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.manager.authorize_checks(
            &tool,
            vec![permission_check(
                "fixture.not_granted",
                PermissionDecision::Ask {
                    reason: "This action has not been authorized.".into(),
                },
            )],
            &executor,
            &control,
            &control.state,
        ),
    )
    .await
    .expect("a new action must fail without waiting for the user");
    assert!(matches!(
        other_action,
        Err(ToolError::CapabilityUnavailable(_))
    ));
    fixture.assert_no_pending_interaction().await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn discovery_persists_and_replays_without_any_plugin_or_execution_tool() {
    let fixture = Fixture::new().await;
    assert!(
        catalog(&fixture.manager.state.load_full().executor)
            .await
            .tools
            .is_empty()
    );
    for function in [ToolApiFunction::List, ToolApiFunction::PluginsList] {
        let id = fixture.dispatch(function, json!({})).await;
        let tool = fixture
            .manager
            .kernel()
            .reader
            .tool(fixture.session, id)
            .await
            .unwrap();
        assert_eq!(tool.phase, ToolPhase::Succeeded);
        let result = tool.result.as_ref().unwrap();
        assert!(result.get("invocation").is_none());
        assert!(result.get("raw_output").is_none());
        assert!(
            result["tool_api_response"]["output_text"]
                .as_str()
                .unwrap()
                .contains("returned 0 of 0")
        );
    }
    let prompt = fixture.prompt().await;
    let responses = prompt
        .turns()
        .iter()
        .flat_map(|run| &run.parts)
        .filter_map(|part| match part {
            CompletionInputPart::ToolResult {
                function,
                output_json,
                ..
            } => Some((function, output_json)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert_eq!(
        responses[0].0.gateway_function(),
        Some(ToolApiFunction::List)
    );
    assert_eq!(
        responses[1].0.gateway_function(),
        Some(ToolApiFunction::PluginsList)
    );
    for (_, output_json) in responses {
        let text: String = serde_json::from_str(output_json).unwrap();
        assert!(text.contains("returned 0 of 0"));
    }
    drop(prompt);
    fixture.shutdown().await;
}

#[tokio::test]
async fn invalid_protocol_envelopes_never_become_execution_results() {
    for arguments in [
        json!({"tool":"fs.read","input":"{}","extra":true}),
        json!({"tool":"fs.read","input":"{"}),
        json!({"tool":"fs.read","input":"[]"}),
        json!({"tool":"fs.read","input":[]}),
    ] {
        let fixture = Fixture::new().await;
        let id = fixture.dispatch(ToolApiFunction::Call, arguments).await;
        let tool = fixture
            .manager
            .kernel()
            .reader
            .tool(fixture.session, id)
            .await
            .unwrap();
        assert_eq!(tool.phase, ToolPhase::Failed);
        let result = tool.result.as_ref().unwrap();
        assert!(result.get("failure").is_some());
        assert!(result.get("invocation").is_none());
        assert!(result.get("raw_output").is_none());
        drop(tool);
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn large_protocol_text_survives_content_storage_and_model_history() {
    let fixture = Fixture::new().await;
    let tool = fixture
        .begin_call("tools_help", json!({"tool":"fixture.large"}))
        .await;
    let id = tool.id();
    let text = format!("开始\n{}\n完整结尾", "字".repeat(25_000));
    complete_protocol_response(
        tool,
        Ok(ToolApiResponse {
            title: "Large help".into(),
            summary: "Full contract".into(),
            output_text: text.clone(),
        }),
    )
    .await
    .unwrap();
    let tool = fixture
        .manager
        .kernel()
        .reader
        .tool(fixture.session, id)
        .await
        .unwrap();
    assert_eq!(tool.phase, ToolPhase::Succeeded);
    assert_eq!(tool.outputs.len(), 1);
    assert_eq!(
        tool.result.as_ref().unwrap()["tool_api_response"]["output_text"],
        ""
    );
    drop(tool);
    let prompt = fixture.prompt().await;
    let output_json = prompt
        .turns()
        .iter()
        .flat_map(|run| &run.parts)
        .find_map(|part| match part {
            CompletionInputPart::ToolResult { output_json, .. } => Some(output_json),
            _ => None,
        })
        .unwrap();
    let output: Value = serde_json::from_str(output_json).unwrap();
    let retained = output["sources"][0]["body"]["text"]
        .as_str()
        .expect("protocol text must be present in the frozen content source");
    assert!(retained == text);
    drop(prompt);
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_execution_history_uses_an_explicit_protocol_envelope() {
    let fixture = Fixture::new().await;
    let input = json!({"file_path":"a.txt"});
    let tool = fixture.begin_call("agena.fs.read", input.clone()).await;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({}),
        },
    )
    .await
    .unwrap();
    let prompt = fixture.prompt().await;
    let calls = prompt
        .turns()
        .iter()
        .flat_map(|run| &run.parts)
        .filter_map(|part| match part {
            CompletionInputPart::ToolCall {
                function,
                arguments_json,
                ..
            } => Some((function, arguments_json)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.gateway_function(), Some(ToolApiFunction::Call));
    assert_eq!(
        serde_json::from_str::<Value>(calls[0].1).unwrap(),
        json!({"tool":"agena.fs.read","input":input})
    );
    drop(prompt);
    fixture.shutdown().await;
}
