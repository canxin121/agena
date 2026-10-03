//! Regression: one model reply may hand the batch several ordinary execution
//! tools at once, and the batch executes all of them concurrently.
//!
//! The tools below rendezvous on a shared barrier sized to the whole batch, so
//! the test proves real overlap instead of asserting on wall-clock ordering. A
//! batch that serialized its members — the behaviour the removed
//! `concurrency_safe` classification used to force for every tool that had not
//! declared itself safe — deadlocks on the first member and fails.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const BATCH_LABELS: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

struct ConcurrentBatchState {
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    barrier: tokio::sync::Barrier,
}

impl ConcurrentBatchState {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            barrier: tokio::sync::Barrier::new(BATCH_LABELS.len()),
        }
    }
}

struct ConcurrentBatchFixture {
    state: Arc<ConcurrentBatchState>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "conc",
    version = "test",
    summary = "Concurrent batch regression fixture"
)]
impl ConcurrentBatchFixture {
    #[tool(name = "alpha", summary = "First batch member.", tags(mutate))]
    async fn alpha(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.rendezvous("alpha").await
    }

    #[tool(name = "beta", summary = "Second batch member.", tags(mutate))]
    async fn beta(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.rendezvous("beta").await
    }

    #[tool(name = "gamma", summary = "Third batch member.", tags(mutate))]
    async fn gamma(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.rendezvous("gamma").await
    }

    #[tool(name = "delta", summary = "Fourth batch member.", tags(mutate))]
    async fn delta(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.rendezvous("delta").await
    }
}

impl ConcurrentBatchFixture {
    /// Block until every batch member is executing, then report success. A
    /// member that never sees its peers fails instead of hanging the suite.
    async fn rendezvous(
        &self,
        label: &str,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        let in_flight = self.state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.state
            .max_in_flight
            .fetch_max(in_flight, Ordering::SeqCst);
        let waited =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.state.barrier.wait())
                .await;
        self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
        match waited {
            Ok(_) => Ok(agena_plugin_host::sdk::ToolInvokeOutput::text(format!(
                "{label} rendezvoused"
            ))),
            Err(_) => Err(agena_plugin_host::sdk::PluginError::internal(format!(
                "{label} never overlapped another member of its batch"
            ))),
        }
    }
}

/// Streams one `tools_call` per fixture label in the first request, then the
/// final answer once every result has been returned to the model.
struct ConcurrentBatchProvider {
    model: ModelId,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}

impl ConcurrentBatchProvider {
    fn new() -> Self {
        Self {
            model: ModelId::new("conc-batch-model"),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request lock").len()
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl ModelRuntime for ConcurrentBatchProvider {
    fn id(&self) -> &str {
        "conc-batch"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        panic!("the concurrent batch fixture drives streaming turns only")
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let index = {
            let mut requests = self.requests.lock().expect("request lock");
            let index = requests.len();
            requests.push(request);
            index
        };
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let mut events = Vec::new();
        match index {
            0 => {
                for (position, label) in BATCH_LABELS.iter().enumerate() {
                    events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        stream_key: format!("call:{position}"),
                        id: Some(format!("conc_batch_{label}")),
                        name: Some("tools_call".to_owned()),
                        arguments_json: serde_json::json!({
                            "tool": format!("conc.{label}"),
                            "input": {},
                        })
                        .to_string(),
                    }));
                }
            }
            1 => {
                events.push(Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "CONCURRENT_BATCH_OK".to_owned(),
                }));
            }
            other => panic!("the stable run made an unexpected provider request #{other}"),
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: Some(if index == 0 {
                CompletionFinishReason::ToolCalls
            } else {
                CompletionFinishReason::Stop
            }),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
            end_turn: None,
        }));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

async fn manager_with_concurrent_batch_fixture(
    provider: Arc<dyn ModelRuntime>,
    state: Arc<ConcurrentBatchState>,
) -> SessionManager {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    for plugin_id in ["agena.tools", "agena.conc"] {
        plugins_config
            .list
            .insert(plugin_id.to_owned(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid Tool API plugin key"),
                ToolSearchFixture,
            ),
            StaticPluginRegistration::new(
                "agena.conc"
                    .parse()
                    .expect("valid batch fixture plugin key"),
                ConcurrentBatchFixture { state },
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build concurrent batch plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
    );
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider);
    let provider_registry = Arc::new(registry);
    let context_governor = ContextGovernor::new(agena_domain::ContextPolicy::default());
    let processor = SessionProcessor::new(plugins);
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open test database");
    initialize(&database).await;
    SessionManager::new(
        database,
        provider_registry,
        context_governor,
        processor,
        executor,
        RuntimeSessionManagerConfig::default(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_reply_hands_the_whole_batch_to_concurrent_execution() {
    let state = Arc::new(ConcurrentBatchState::new());
    let provider = Arc::new(ConcurrentBatchProvider::new());
    let manager = manager_with_concurrent_batch_fixture(provider.clone(), Arc::clone(&state)).await;
    let session = create(&manager, "concurrent tool batch").await;
    let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
    request_override.set_parallel_tool_calls(Some(true));
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("conc-batch", "conc-batch-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: Some("Run every batch member, then finish.".to_owned()),
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Run all four batch members.",
        ))],
    );

    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        manager.submit_subtask_user_message(request, None),
    )
    .await
    .expect("a four-tool batch must not deadlock on its own barrier")
    .expect("the stable run continues after the batch results return to the model");

    assert_eq!(
        state.max_in_flight.load(Ordering::SeqCst),
        BATCH_LABELS.len(),
        "every batch member must be executing at the same time"
    );

    let tool_states = completed
        .parts()
        .iter()
        .filter(|part| part.kind == "tool_call")
        .map(|part| part.state)
        .collect::<Vec<_>>();
    assert_eq!(tool_states.len(), BATCH_LABELS.len());
    assert!(
        tool_states
            .iter()
            .all(|state| *state == PartState::Completed),
        "each concurrent member must land as its own completed activity: {tool_states:?}"
    );
    assert!(
        completed.pending_tools().is_empty(),
        "every member of the batch is terminal"
    );
    assert_eq!(
        provider.request_count(),
        2,
        "the whole batch must return to the model as exactly one follow-up turn"
    );

    // Each member's own result is durable on its own part, not merged into a
    // sibling's activity.
    let persisted = manager
        .session_store()
        .load(session.id)
        .await
        .expect("reload the batch session");
    for label in BATCH_LABELS {
        let rendered = format!("{label} rendezvoused");
        let part = persisted
            .parts
            .iter()
            .find(|part| {
                part.kind == "tool_call"
                    && serde_json::to_string(&part.content)
                        .expect("serialize tool part")
                        .contains(&rendered)
            })
            .unwrap_or_else(|| panic!("no persisted tool part carries `{rendered}`"));
        assert_eq!(
            part.state,
            PartState::Completed,
            "`{rendered}` must be durably completed"
        );
    }

    let requests = provider.requests();
    let replayed_tool_turn = requests[1]
        .turns
        .iter()
        .find(|run| {
            run.role == Role::Assistant
                && run.parts.iter().any(|part| {
                    matches!(part, CompletionInputPart::ToolCall { function, .. }
                        if function.function_name() == "tools_call")
                })
        })
        .expect("the follow-up request must replay the tool-calling turn");
    for label in BATCH_LABELS {
        assert!(
            replayed_tool_turn.parts.iter().any(|part| {
                matches!(part, CompletionInputPart::ToolResult { output_json, .. }
                    if output_json.contains(&format!("{label} rendezvoused")))
            }),
            "the replayed transcript must carry the `{label}` result: {:#?}",
            replayed_tool_turn.parts
        );
    }
    assert!(completed.parts().iter().any(|part| {
        part.kind == "text"
            && part.content.get("text").and_then(serde_json::Value::as_str)
                == Some("CONCURRENT_BATCH_OK")
    }));
}
