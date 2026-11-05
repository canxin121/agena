use super::*;
use agena_domain::{PermissionDecision, StructuredObject, ToolApiCall, ToolApiFunction};
use agena_provider::CompletionInputPart;
use serde_json::Value;

#[derive(Clone)]
struct PermissionHookProbe {
    deny: Arc<std::sync::atomic::AtomicBool>,
    observed: Arc<std::sync::Mutex<Vec<String>>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "test",
    name = "permission-probe",
    version = "test",
    summary = "Verify that policy hooks run before user interaction."
)]
impl PermissionHookProbe {
    #[hook(permission.request, summary = "Deny unauthorized approvals", help = "Verifies that a plugin can deny tool approval before a user interaction is created.")]
    async fn permission_request(
        &self,
        input: agena_plugin_host::PermissionRequestInput,
    ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::PermissionRequestPatch>> {
        assert!(input.session_id > 0);
        self.observed.lock().unwrap().push(input.action);
        Ok(Some(agena_plugin_host::PermissionRequestPatch {
            deny_reason: self
                .deny
                .load(std::sync::atomic::Ordering::Acquire)
                .then(|| "plugin permission policy denied".into()),
        }))
    }
}

struct Fixture {
    permission_hook: PermissionHookProbe,
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
            generation: self.attempt.generation(),
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
        Self::with_permission_plugin(false).await
    }

    async fn with_permission_plugin(enabled: bool) -> Self {
        let permission_hook = PermissionHookProbe {
            deny: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            observed: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
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
        let mut plugin_config = agena_plugin_host::PluginsConfig::default();
        let static_plugins = if enabled {
            plugin_config.list.insert(
                "test.permission-probe".into(),
                agena_plugin_host::ConfiguredPlugin::static_default(),
            );
            vec![agena_plugin_host::StaticPluginRegistration::new(
                "test.permission-probe".parse().unwrap(),
                permission_hook.clone(),
            )]
        } else {
            Vec::new()
        };
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                static_plugins,
                config: plugin_config,
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
            permission_hook,
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
            generation: self.attempt.generation(),
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
async fn permission_hook_denies_before_any_interactive_approval_is_committed() {
    let fixture = Fixture::with_permission_plugin(true).await;
    fixture
        .permission_hook
        .deny
        .store(true, std::sync::atomic::Ordering::Release);
    let tool = fixture.begin_call("fixture.run", json!({})).await;
    let control = fixture.register_live_call(&tool);
    let executor = fixture.scoped_executor().await;
    let result = fixture
        .manager
        .authorize_checks(
            &tool,
            vec![permission_check(
                "fixture.requires_approval",
                PermissionDecision::Ask {
                    reason: "human approval".into(),
                },
            )],
            &executor,
            &control,
            &control.state,
        )
        .await;
    match result {
        Err(ToolError::PolicyDenied(denied)) => {
            assert!(denied.reason.contains("plugin permission policy denied"));
        }
        other => panic!("expected plugin policy denial, got {other:?}"),
    }
    assert_eq!(fixture.permission_hook.observed.lock().unwrap().len(), 1);
    fixture.assert_no_pending_interaction().await;
    fixture.shutdown().await;
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

#[tokio::test]
async fn failed_inline_ui_feedback_survives_current_history_hydration() {
    for (name, report, expected) in [
        (
            "agena.inline_ui.prepare",
            json!({
                "code":"invalid_ui", "applied":false, "truncated":false,
                "issues":[{"path":"document.blocks[0].program.nodes[2].props.style",
                    "reason":"unsupported property", "fix":"Remove the unsupported property."}]
            }),
            vec![
                "document.blocks[0].program.nodes[2].props.style",
                "Remove the unsupported property.",
                "invalid_ui",
            ],
        ),
        (
            "agena.inline_ui.update",
            json!({
                "code":"revision_conflict", "applied":false, "truncated":false,
                "expected_revision":1, "current_revision":2,
                "issues":[{"path":"update.expected_revision", "reason":"expected 1, current 2",
                    "fix":"Read current UI and reconcile before retrying."}]
            }),
            vec![
                "revision_conflict",
                "update.expected_revision",
                "expected 1",
                "current 2",
                "reconcile",
            ],
        ),
    ] {
        let mut sdk = agena_plugin_host::sdk::PluginError::invalid_params_with_data(
            "private-diagnostic-sentinel",
            json!({"agena_inline_ui_error":report, "private_detail":"private-payload-sentinel"}),
        );
        sdk.failure.user = agena_failure::UserPresentation::validated(
            "inline_ui.error",
            "The plugin input is invalid.",
        );
        let failure =
            super::super::tool_error(crate::tool::ToolError::from_plugin_error(sdk)).failure();
        let fixture = Fixture::new().await;
        let tool = fixture
            .begin_call(
                ToolApiFunction::Call.function_name(),
                json!({"tool":name,"input":{}}),
            )
            .await;
        tool.complete(RequestId::new(), ToolResult {
            phase: ToolPhase::Failed,
            value: json!({
                "error":agena_failure::UserProblem::from(&failure), "failure":failure,
                "invocation":agena_domain::ToolInvocation::new(name, Default::default()), "metadata":{},
            }),
        }).await.unwrap();
        let prompt = fixture.prompt().await;
        let output_json = prompt
            .turns()
            .iter()
            .flat_map(agena_runtime_provider::provider::wire_message::project)
            .find_map(|part| match part {
                agena_runtime_provider::provider::wire_message::WirePart::ToolResult {
                    status,
                    output_json,
                    ..
                } => {
                    assert_eq!(
                        status,
                        agena_provider::CompletionInputToolResultStatus::Failed
                    );
                    Some(output_json)
                }
                _ => None,
            })
            .expect("the current kernel must retain the failed protocol result");
        let output: Value = serde_json::from_str(&output_json).unwrap();
        let restored: agena_failure::Failure =
            serde_json::from_value(output["result"]["failure"].clone()).unwrap();
        let model_text = restored
            .model
            .expect("reviewed corrective feedback survives replay")
            .message();
        for expected in expected {
            assert!(model_text.contains(expected), "{model_text}");
            assert!(output_json.contains(expected), "{output_json}");
        }
        assert!(model_text.len() < 450);
        assert!(!output_json.contains("agena_inline_ui_error"));
        assert!(!output_json.contains("private-diagnostic-sentinel"));
        assert!(!output_json.contains("private-payload-sentinel"));
        assert!(output["result"]["error"].get("model").is_none());
        drop(prompt);
        fixture.shutdown().await;
    }
}
