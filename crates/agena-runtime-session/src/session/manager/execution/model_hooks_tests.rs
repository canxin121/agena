//! Model provider integration: real SessionManager → kernel round → hook host
//! → plugin callback, including lazy provider-stream polling and request scope.

use super::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agena_domain::{ComposerDocument, ComposerNode, Model, ModelId, ModelRef, ProviderId};
use agena_plugin_host::sdk::{
    ChatDirection, ChatHeadersInput, ChatHeadersPatch, ChatMessage, ChatMessageInput,
    ChatMessagePatch, ChatMessagesTransformInput, ChatMessagesTransformPatch,
    ChatSystemTransformInput, ChatSystemTransformPatch, ModelRequestAfterInput,
    ModelRequestBeforeInput, ModelRequestBeforePatch, ModelRequestErrorInput,
    ModelRequestErrorPatch, PostRunInput, PreRunInput,
};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionUsage,
};
use agena_runtime_provider::{
    ProviderError,
    provider::{CompletionResponse as RuntimeCompletionResponse, ModelRuntime},
};

// Provider round leases are process-budgeted. Each fixture constructs its own
// in-memory kernel, whose autoincrement session IDs can overlap with peers.
// Serialize these integrations so unrelated test kernels cannot compete for
// the same global session allocation.
static MODEL_FIXTURE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone)]
struct ModelHookProbe {
    fired: Arc<Mutex<Vec<&'static str>>>,
}

impl ModelHookProbe {
    fn record(&self, name: &'static str) {
        self.fired.lock().unwrap().push(name);
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "test",
    name = "model-hooks",
    version = "test",
    summary = "Verify live model projection and provider lifecycle hooks."
)]
impl ModelHookProbe {
    #[hook(chat.system, summary = "Inject test system instructions", help = "Appends a test marker to the provider-bound system prompt.")]
    async fn system(
        &self,
        input: ChatSystemTransformInput,
    ) -> agena_plugin_host::sdk::Result<Option<ChatSystemTransformPatch>> {
        assert!(input.session_id > 0);
        self.record("chat.system");
        Ok(Some(ChatSystemTransformPatch {
            append: Some("\nSYSTEM HOOK MARKER".into()),
            ..Default::default()
        }))
    }

    #[hook(chat.message, summary = "Observe projected chat messages", help = "Inspects individual provider-bound messages without changing history.")]
    async fn message(
        &self,
        input: ChatMessageInput,
    ) -> agena_plugin_host::sdk::Result<Option<ChatMessagePatch>> {
        assert_eq!(input.direction, ChatDirection::ToProvider);
        self.record("chat.message");
        Ok(None)
    }

    #[hook(chat.messages, summary = "Inject synthetic model context", help = "Inserts a test context message in the provider request projection.")]
    async fn messages(
        &self,
        input: ChatMessagesTransformInput,
    ) -> agena_plugin_host::sdk::Result<Option<ChatMessagesTransformPatch>> {
        self.record("chat.messages");
        let mut messages = vec![ChatMessage::system("MEMORY HOOK MARKER")];
        messages.extend(input.messages);
        Ok(Some(ChatMessagesTransformPatch {
            messages: Some(messages),
        }))
    }

    #[hook(chat.headers, summary = "Verify scoped request headers", help = "Adds a test header scoped to the current provider request.")]
    async fn headers(
        &self,
        input: ChatHeadersInput,
    ) -> agena_plugin_host::sdk::Result<Option<ChatHeadersPatch>> {
        assert_eq!(input.provider, "hook-model-test");
        assert!(input.session_id > 0);
        self.record("chat.headers");
        Ok(Some(ChatHeadersPatch {
            set: BTreeMap::from([("x-hook-probe".into(), "present".into())]),
            remove: Vec::new(),
        }))
    }

    #[hook(run.pre, summary = "Observe execution start", help = "Records a model test execution before its first effect.")]
    async fn before_run(&self, input: PreRunInput) -> agena_plugin_host::sdk::Result<()> {
        assert_eq!(input.model, "model");
        assert_eq!(input.kind, agena_plugin_host::RunKind::Conversation);
        assert!(!input.execution_id.is_empty());
        self.record("run.pre");
        Ok(())
    }

    #[hook(run.post, summary = "Observe execution completion", help = "Records the final outcome of a model test execution.")]
    async fn after_run(&self, input: PostRunInput) -> agena_plugin_host::sdk::Result<()> {
        assert_eq!(input.status, agena_plugin_host::RunStatus::Succeeded);
        assert_eq!(input.kind, agena_plugin_host::RunKind::Conversation);
        self.record("run.post");
        Ok(())
    }

    #[hook(model.request.before, summary = "Check provider request boundary", help = "Records each admitted provider request before network I/O.")]
    async fn model_before(
        &self,
        input: ModelRequestBeforeInput,
    ) -> agena_plugin_host::sdk::Result<Option<ModelRequestBeforePatch>> {
        assert_eq!(input.model, "model");
        self.record("model.request.before");
        Ok(None)
    }

    #[hook(model.request.after, summary = "Observe provider request outcome", help = "Records the outcome of each completed or failed provider request.")]
    async fn model_after(
        &self,
        input: ModelRequestAfterInput,
    ) -> agena_plugin_host::sdk::Result<()> {
        assert!(matches!(input.status.as_str(), "completed" | "error"));
        assert_eq!(input.model, "model");
        self.record("model.request.after");
        Ok(())
    }

    #[hook(model.request.error, summary = "Test model failure retries", help = "Requests retry only for the injected retryable test failure.")]
    async fn model_error(
        &self,
        input: ModelRequestErrorInput,
    ) -> agena_plugin_host::sdk::Result<Option<ModelRequestErrorPatch>> {
        self.record("model.request.error");
        Ok(Some(ModelRequestErrorPatch {
            retry: input.message.contains("retryable-probe-failure"),
        }))
    }
}

#[derive(Clone, Copy)]
enum ProviderSequence {
    Stop,
    LengthThenStop,
    ErrorThenStop,
}

struct HookModelProvider {
    model: ModelId,
    received: Arc<Mutex<Vec<(CompletionRequest, BTreeMap<String, String>)>>>,
    sequence: ProviderSequence,
}

#[async_trait::async_trait]
impl ModelRuntime for HookModelProvider {
    fn id(&self) -> &str {
        "hook-model-test"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn prompt_cache_shape(&self, _: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id(),
            [("destination_revision", "hook-model-test")],
        ))
    }
    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<RuntimeCompletionResponse, ProviderError> {
        // This function is reached only after the lazy stream starts polling.
        // It must still see the owner-scoped Header Hook.
        let headers = crate::provider::provider::utils::resolved_request_headers_async(
            self.id(),
            BTreeMap::new(),
        )
        .await?;
        let mut received = self.received.lock().unwrap();
        let first_round = received.is_empty();
        received.push((request, headers));
        drop(received);
        if first_round && matches!(self.sequence, ProviderSequence::ErrorThenStop) {
            return Err(ProviderError::Config("retryable-probe-failure".into()));
        }
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: "model response".into(),
            reasoning_text: None,
            finish_reason: Some(
                if first_round && matches!(self.sequence, ProviderSequence::LengthThenStop) {
                    CompletionFinishReason::Length
                } else {
                    CompletionFinishReason::Stop
                },
            ),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }
}

async fn fixture() -> (
    SessionManager,
    SessionId,
    Arc<Mutex<Vec<&'static str>>>,
    Arc<Mutex<Vec<(CompletionRequest, BTreeMap<String, String>)>>>,
) {
    fixture_with_sequence(ProviderSequence::Stop).await
}

async fn fixture_with_sequence(
    sequence: ProviderSequence,
) -> (
    SessionManager,
    SessionId,
    Arc<Mutex<Vec<&'static str>>>,
    Arc<Mutex<Vec<(CompletionRequest, BTreeMap<String, String>)>>>,
) {
    let fired = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut config = PluginsConfig::default();
    config.list.insert(
        "test.model-hooks".into(),
        ConfiguredPlugin::static_default(),
    );
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "test.model-hooks".parse().unwrap(),
            ModelHookProbe {
                fired: fired.clone(),
            },
        )],
        config,
        workspace_root: "/model-hooks-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        "/model-hooks-tests",
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(HookModelProvider {
        model: ModelId::new("model"),
        received: received.clone(),
        sequence,
    });
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let workspace = kernel
        .workspaces
        .ensure_id("/model-hooks-tests")
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Model Hooks".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let manager = SessionManager::new(
        kernel,
        Arc::new(providers),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        Default::default(),
    );
    (manager, session, fired, received)
}

async fn execute_session(manager: &SessionManager, session: SessionId) {
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "check model hooks".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let options = SessionRunOptions {
        model: ModelRef::new("hook-model-test", "model"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        model_supports_parallel_tool_calls: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    };
    let run = manager.supervise(
        execution.clone(),
        options,
        false,
        manager.reserve_execution().unwrap(),
    );
    tokio::time::timeout(std::time::Duration::from_secs(15), run.wait_settled())
        .await
        .expect("hook integration run must finish");
    let result = manager
        .kernel()
        .reader
        .execution(session, execution.id)
        .await
        .unwrap();
    assert_eq!(
        result.phase,
        agena_domain::session_kernel::ExecutionPhase::Succeeded
    );
}

#[tokio::test]
async fn model_hooks_fire_at_the_actual_provider_boundary() {
    let _fixture = MODEL_FIXTURE_GATE.lock().await;
    let (manager, session, fired, received) = fixture().await;
    execute_session(&manager, session).await;

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1);
    let (request, headers) = &received[0];
    assert!(
        request
            .system
            .as_deref()
            .unwrap_or_default()
            .contains("SYSTEM HOOK MARKER")
    );
    assert_eq!(
        headers.get("x-hook-probe").map(String::as_str),
        Some("present")
    );
    assert!(request.turns.iter().any(|run| {
        run.role == agena_domain::Role::System && run.as_text_lossy().contains("MEMORY HOOK MARKER")
    }));

    let fired = fired.lock().unwrap();
    for event in [
        "chat.system",
        "chat.message",
        "chat.messages",
        "chat.headers",
        "run.pre",
        "run.post",
        "model.request.before",
        "model.request.after",
    ] {
        assert!(
            fired.contains(&event),
            "missing actual trigger {event}: {fired:?}"
        );
    }
    assert!(
        fired
            .iter()
            .position(|x| *x == "model.request.before")
            .unwrap()
            < fired.iter().position(|x| *x == "chat.headers").unwrap()
    );
}

#[tokio::test]
async fn run_hooks_fire_once_even_when_an_execution_requires_two_model_rounds() {
    let _fixture = MODEL_FIXTURE_GATE.lock().await;
    let (manager, session, fired, received) =
        fixture_with_sequence(ProviderSequence::LengthThenStop).await;
    execute_session(&manager, session).await;
    assert_eq!(
        received.lock().unwrap().len(),
        2,
        "length finish must cause a second round"
    );
    let events = fired.lock().unwrap();
    let count = |name: &str| events.iter().filter(|event| **event == name).count();
    assert_eq!(count("run.pre"), 1);
    assert_eq!(count("run.post"), 1);
    assert_eq!(count("model.request.before"), 2);
    assert_eq!(count("model.request.after"), 2);
    assert_eq!(count("chat.headers"), 2);
}

#[tokio::test]
async fn failed_model_request_is_retried_only_through_the_explicit_error_hook() {
    let _fixture = MODEL_FIXTURE_GATE.lock().await;
    let (manager, session, fired, received) =
        fixture_with_sequence(ProviderSequence::ErrorThenStop).await;
    execute_session(&manager, session).await;
    assert_eq!(received.lock().unwrap().len(), 2);
    let events = fired.lock().unwrap();
    let count = |name: &str| events.iter().filter(|event| **event == name).count();
    assert_eq!(count("run.pre"), 1);
    assert_eq!(count("run.post"), 1);
    assert_eq!(count("model.request.before"), 2);
    assert_eq!(count("model.request.after"), 2);
    assert_eq!(count("model.request.error"), 1);
}
