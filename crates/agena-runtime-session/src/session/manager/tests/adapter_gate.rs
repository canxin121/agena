//! Real model -> tools_call -> executor regression, with local synthetic plugins.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CloudFixture {
    calls: Arc<AtomicUsize>,
}
#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "claude",
    version = "test",
    summary = "Adapter gate fixture"
)]
impl CloudFixture {
    #[tool(
        name = "cloud_web_search",
        summary = "Synthetic cloud search",
        read_only,
        concurrency_safe
    )]
    async fn search(&self) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        "cloud fixture reached".into()
    }
    #[tool(
        name = "cloud_code_execution",
        summary = "Synthetic cloud compute",
        read_only,
        concurrency_safe
    )]
    async fn compute(&self) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        "cloud fixture reached".into()
    }
}
struct OpenAiCloudFixture {
    calls: Arc<AtomicUsize>,
}
#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "chatgpt",
    version = "test",
    summary = "OpenAI adapter family fixture"
)]
impl OpenAiCloudFixture {
    #[tool(
        name = "cloud_web_search",
        summary = "Synthetic OpenAI search",
        read_only,
        concurrency_safe
    )]
    async fn search(&self) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        "cloud fixture reached".into()
    }
    #[tool(
        name = "cloud_shell",
        summary = "Synthetic OpenAI hosted shell",
        read_only,
        concurrency_safe
    )]
    async fn shell(&self) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        "cloud fixture reached".into()
    }
}

struct Caller {
    model: ModelId,
    adapter: agena_domain::AdapterId,
    targets: [&'static str; 2],
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}
#[async_trait::async_trait]
impl ModelRuntime for Caller {
    fn id(&self) -> &str {
        "caller-label-is-not-a-provider-brand"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn default_adapter(&self) -> Option<&agena_domain::AdapterId> {
        Some(&self.adapter)
    }
    fn agena_tool_mode(&self, _: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }
    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        panic!("execution check must not query model discovery")
    }
    async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        panic!("fixture uses streaming model turns")
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
            let mut requests = self.requests.lock().unwrap();
            let n = requests.len();
            requests.push(request);
            n
        };
        let mut events = Vec::new();
        if index == 0 {
            for (index, target) in self.targets.into_iter().enumerate() {
                events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                    provider_id: ProviderId::new(self.id()),
                    model: self.model.clone(),
                    stream_key: format!("call:{index}"),
                    id: Some(format!("cloud_gate_{index}")),
                    name: Some("tools_call".into()),
                    arguments_json: serde_json::json!({"tool":target,"input":{}}).to_string(),
                }));
            }
        } else {
            assert_eq!(
                index, 1,
                "capability mismatch should return to the model once, not retry provider tools automatically"
            );
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new(self.id()),
                model: self.model.clone(),
                delta: "adapter gate fixture finished".into(),
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
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
async fn run(adapter: &str, parallel: bool, expected_calls: usize) {
    run_family(adapter, parallel, expected_calls, "claude").await;
}
async fn run_family(adapter: &str, parallel: bool, expected_calls: usize, family: &str) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(Caller {
        model: ModelId::new("model-label-is-not-a-brand"),
        adapter: agena_domain::AdapterId::new(adapter),
        targets: match family {
            "chatgpt" => ["chatgpt.cloud_web_search", "chatgpt.cloud_shell"],
            "claude" => ["claude.cloud_web_search", "claude.cloud_code_execution"],
            _ => panic!("unknown fixture family"),
        },
        requests: std::sync::Mutex::new(Vec::new()),
    });
    let mut config = PluginsConfig::default();
    let plugin_id = format!("agena.{family}");
    for name in ["agena.tools", plugin_id.as_str()] {
        config
            .list
            .insert(name.into(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new("agena.tools".parse().unwrap(), ToolSearchFixture),
            match family {
                "chatgpt" => StaticPluginRegistration::new(
                    plugin_id.parse().unwrap(),
                    OpenAiCloudFixture {
                        calls: calls.clone(),
                    },
                ),
                "claude" => StaticPluginRegistration::new(
                    plugin_id.parse().unwrap(),
                    CloudFixture {
                        calls: calls.clone(),
                    },
                ),
                _ => unreachable!(),
            },
        ],
        config,
        workspace_root: root.clone(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
        None,
    );
    let database = Database::connect("sqlite::memory:").await.unwrap();
    initialize(&database).await;
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider.clone());
    let manager = SessionManager::new(
        database,
        Arc::new(registry),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        RuntimeSessionManagerConfig::default(),
    );
    let session = create(&manager, "adapter gate actual model loop").await;
    let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
    request_override.set_parallel_tool_calls(Some(parallel));
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new(provider.id(), provider.model.as_ref()),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: None,
            temperature: None,
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Run both synthetic cloud tools.",
        ))],
    );
    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        manager.submit_subtask_user_message(request, None),
    )
    .await
    .expect("runtime adapter gate must not wait for interactive approval")
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    // The model must receive an actionable tool result, not a hidden failure.
    let replay = serde_json::to_string(&requests[1]).unwrap();
    if expected_calls == 0 {
        assert!(replay.contains("provider_tool_adapter"));
        assert!(replay.contains(adapter));
        assert!(replay.contains(if family == "chatgpt" {
            "openai_responses"
        } else {
            "anthropic"
        }));
    } else {
        assert!(replay.contains("cloud fixture reached"));
    }
    assert_eq!(
        completed
            .parts()
            .iter()
            .filter(|part| part.kind == "tool_call")
            .count(),
        2
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ai_adapter_gate_is_enforced_on_sequential_model_calls() {
    run("openai_responses", false, 0).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ai_adapter_gate_is_enforced_on_parallel_model_calls() {
    run("gemini", true, 0).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ai_adapter_gate_matching_default_adapter_allows_parallel_execution() {
    run("anthropic", true, 2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ai_adapter_gate_openai_chat_completions_allows_openai_but_not_claude() {
    run_family("openai_chat_completions", false, 2, "chatgpt").await;
    run("openai_chat_completions", true, 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ai_adapter_gate_openai_responses_still_allows_openai_execution() {
    run_family("openai_responses", true, 2, "chatgpt").await;
}
