use super::*;
use agena_domain::{StructuredObject, ToolApiCall, ToolApiFunction};
use agena_provider::CompletionInputPart;
use serde_json::Value;

struct Fixture {
    manager: SessionManager,
    session: SessionId,
    attempt: AttemptHandle,
    model: ModelRef,
}

impl Fixture {
    async fn new() -> Self {
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let workspace = kernel
            .workspaces
            .ensure_id("/tool-api-tests")
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
                workspace_root: "/tool-api-tests".into(),
                agena_version: "test".into(),
                callback_base_url: None,
                host_client: None,
                previous: None,
                previous_plugins: Default::default(),
            })
            .await
            .unwrap();
        let executor = ToolExecutor::new(
            "/tool-api-tests",
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
    }
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
    let fixture = Fixture::new().await;
    let id = fixture
        .dispatch(
            ToolApiFunction::Call,
            json!({"tool":"fs.read","input":"{}"}),
        )
        .await;
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
