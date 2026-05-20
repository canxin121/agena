use super::*;
use crate::message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent};
use crate::model::{ModelId, ModelMetadata, ModelPricing, ModelPricingTier, ModelRef, ProviderId};
use crate::provider::{
    CapabilityFamily, CapabilitySupport, CompletionFinishReason, CompletionRequest,
    CompletionResponse, CompletionStreamEvent, CompletionUsage, ModelCapabilities, ProviderModel,
};
use futures_util::{StreamExt, stream};
use std::sync::{
    LazyLock,
    atomic::{AtomicUsize, Ordering},
};

struct FlakyProvider {
    provider_id: &'static str,
    attempts: Arc<AtomicUsize>,
    fail_attempts: usize,
    retryable: bool,
}

#[derive(Debug, Clone, Copy)]
enum StreamFailureMode {
    StartupRetryableOnceThenSuccess,
    FirstItemRetryableOnceThenSuccess,
    MidStreamRetryableNoRestart,
    MidStreamReplaySafeResumeThenSuccess,
    MidStreamReplaySafeDiverges,
}

struct FlakyStreamProvider {
    provider_id: &'static str,
    stream_starts: Arc<AtomicUsize>,
    mode: StreamFailureMode,
    resume_policy: StreamResumePolicy,
}

struct MultiStartupFailureProvider {
    provider_id: &'static str,
    stream_starts: Arc<AtomicUsize>,
    fail_attempts: usize,
}

struct ModeSynthProvider {
    provider_id: &'static str,
    family: CapabilityFamily,
    models: Vec<Model>,
}

struct PricedUsageProvider {
    provider_id: &'static str,
    metadata: ModelMetadata,
    usage: CompletionUsage,
}

fn retryable_api_error(provider_id: &str, message: &str) -> AppError {
    AppError::ProviderClassified {
        provider: provider_id.to_owned(),
        message: message.to_owned(),
        kind: crate::error::ProviderErrorKind::ApiError,
        retryable: true,
    }
}

fn pid(value: &str) -> ProviderId {
    ProviderId::new(value)
}

fn mid(value: &str) -> ModelId {
    ModelId::new(value)
}

fn model_ref(provider: &str, model: &str) -> ModelRef {
    ModelRef::new(provider, model)
}

fn completion_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: mid(model),
        system: None,
        messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
        tools: Vec::new(),
        temperature: None,
        max_output_tokens: Some(16),
        prompt_cache_key: None,
        previous_response_id: None,
        prompt_window_generation: None,

        stop_sequences: Vec::new(),

        top_p: None,

        top_k: None,

        seed: None,

        thinking: None,

        verbosity: None,

        request_override: Default::default(),

        response_format: None,
    }
}

fn sample_pricing() -> ModelPricing {
    ModelPricing {
        input_usd_per_million_tokens: Some("1.0".to_owned()),
        output_usd_per_million_tokens: Some("2.0".to_owned()),
        cache_read_usd_per_million_tokens: Some("0.1".to_owned()),
        cache_write_usd_per_million_tokens: Some("0.2".to_owned()),
        tiers: vec![ModelPricingTier {
            tier_type: Some("context".to_owned()),
            size_tokens: Some(200_000),
            input_usd_per_million_tokens: Some("3.0".to_owned()),
            output_usd_per_million_tokens: Some("6.0".to_owned()),
            cache_read_usd_per_million_tokens: Some("0.3".to_owned()),
            cache_write_usd_per_million_tokens: Some("0.6".to_owned()),
        }],
    }
}

#[async_trait]
impl ModelRuntime for ModeSynthProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        &self.models[0].id
    }

    fn capability_family(&self) -> Option<CapabilityFamily> {
        Some(self.family)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        Ok(self.models.clone())
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Err(AppError::Internal(
            "unused in mode synth provider".to_owned(),
        ))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for PricedUsageProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: LazyLock<ModelId> = LazyLock::new(|| ModelId::new("priced-model"));
        &DEFAULT_MODEL
    }

    fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
        ModelCapabilities::default().with_streaming(CapabilitySupport::Supported)
    }

    fn model_metadata(&self, _model: &ModelId) -> ModelMetadata {
        self.metadata.clone()
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(self.provider_id, self.default_model().as_str())
                .with_metadata(self.metadata.clone()),
        ])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Ok(CompletionResponse {
            provider_id: pid(self.provider_id),
            model: request.model,
            text: "ok".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(self.usage.clone()),
            provider_metadata: None,
        })
    }
}

#[async_trait::async_trait]
impl ModelRuntime for FlakyProvider {
    fn id(&self) -> &str {
        self.provider_id
    }
    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: LazyLock<ModelId> = LazyLock::new(|| ModelId::new("flaky-model"));
        &DEFAULT_MODEL
    }
    fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
        ModelCapabilities::default()
            .with_tool_calling(CapabilitySupport::Supported)
            .with_streaming(CapabilitySupport::Supported)
    }
    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(self.provider_id, self.default_model().as_str())
                .with_display_name("Flaky Model"),
        ])
    }
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_attempts {
            if self.retryable {
                return Err(AppError::ProviderClassified {
                    provider: self.provider_id.to_owned(),
                    message: "transient failure".to_owned(),
                    kind: crate::error::ProviderErrorKind::ApiError,
                    retryable: true,
                });
            }
            return Err(AppError::Provider("permanent failure".to_owned()));
        }
        Ok(CompletionResponse {
            provider_id: pid(self.provider_id),
            model: request.model,
            text: "ok".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }
}

#[async_trait::async_trait]
impl ModelRuntime for FlakyStreamProvider {
    fn id(&self) -> &str {
        self.provider_id
    }
    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: LazyLock<ModelId> =
            LazyLock::new(|| ModelId::new("flaky-stream-model"));
        &DEFAULT_MODEL
    }
    fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
        ModelCapabilities::default().with_streaming(CapabilitySupport::Supported)
    }
    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.resume_policy
    }
    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(self.provider_id, self.default_model().as_str())
                .with_display_name("Flaky Stream Model"),
        ])
    }
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Ok(CompletionResponse {
            provider_id: pid(self.provider_id),
            model: self.default_model().clone(),
            text: "ok".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }
    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let start = self.stream_starts.fetch_add(1, Ordering::SeqCst);
        let provider_id = pid(self.provider_id);
        let model = self.default_model().clone();
        let success_events = || {
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "ok".to_owned(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        };
        match self.mode {
            StreamFailureMode::StartupRetryableOnceThenSuccess => {
                if start == 0 {
                    return Err(retryable_api_error(
                        self.provider_id,
                        "startup stream failure",
                    ));
                }
                Ok(Box::pin(stream::iter(success_events())))
            }
            StreamFailureMode::FirstItemRetryableOnceThenSuccess => {
                if start == 0 {
                    return Ok(Box::pin(stream::iter(vec![Err(retryable_api_error(
                        self.provider_id,
                        "first item stream failure",
                    ))])));
                }
                Ok(Box::pin(stream::iter(success_events())))
            }
            StreamFailureMode::MidStreamRetryableNoRestart => Ok(Box::pin(stream::iter(vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "partial".to_owned(),
                }),
                Err(retryable_api_error(self.provider_id, "mid-stream failure")),
            ]))),
            StreamFailureMode::MidStreamReplaySafeResumeThenSuccess => {
                if start == 0 {
                    return Ok(Box::pin(stream::iter(vec![
                        Ok(CompletionStreamEvent::TextDelta {
                            provider_id: provider_id.clone(),
                            model: model.clone(),
                            delta: "partial".to_owned(),
                        }),
                        Err(retryable_api_error(self.provider_id, "mid-stream failure")),
                    ])));
                }
                Ok(Box::pin(stream::iter(vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        delta: "partial".to_owned(),
                    }),
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        delta: "final".to_owned(),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ])))
            }
            StreamFailureMode::MidStreamReplaySafeDiverges => {
                if start == 0 {
                    return Ok(Box::pin(stream::iter(vec![
                        Ok(CompletionStreamEvent::TextDelta {
                            provider_id: provider_id.clone(),
                            model: model.clone(),
                            delta: "partial".to_owned(),
                        }),
                        Err(retryable_api_error(self.provider_id, "mid-stream failure")),
                    ])));
                }
                Ok(Box::pin(stream::iter(vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        delta: "DIFF".to_owned(),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ])))
            }
        }
    }
}

#[async_trait::async_trait]
impl ModelRuntime for MultiStartupFailureProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: LazyLock<ModelId> =
            LazyLock::new(|| ModelId::new("flaky-stream-model"));
        &DEFAULT_MODEL
    }

    fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
        ModelCapabilities::default().with_streaming(CapabilitySupport::Supported)
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(self.provider_id, self.default_model().as_str())
                .with_display_name("Flaky Stream Model"),
        ])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Ok(CompletionResponse {
            provider_id: pid(self.provider_id),
            model: self.default_model().clone(),
            text: "ok".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let start = self.stream_starts.fetch_add(1, Ordering::SeqCst);
        if start < self.fail_attempts {
            return Err(retryable_api_error(
                self.provider_id,
                "startup stream failure",
            ));
        }

        Ok(Box::pin(stream::iter(vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: pid(self.provider_id),
                model: self.default_model().clone(),
                delta: "ok".to_owned(),
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: pid(self.provider_id),
                model: self.default_model().clone(),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: None,
                provider_metadata: None,
            }),
        ])))
    }
}

struct UnsupportedImageProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ModelRuntime for UnsupportedImageProvider {
    fn id(&self) -> &str {
        "unsupported-image"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: LazyLock<ModelId> =
            LazyLock::new(|| ModelId::new("unsupported-image-model"));
        &DEFAULT_MODEL
    }

    fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
        ModelCapabilities::default()
            .with_image_input(CapabilitySupport::Unsupported)
            .with_streaming(CapabilitySupport::Supported)
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new("unsupported-image", self.default_model().as_str())
                .with_capabilities(self.model_capabilities(self.default_model())),
        ])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse {
            provider_id: pid("unsupported-image"),
            model: self.default_model().clone(),
            text: "should not run".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }
}

#[test]
fn provider_http_client_config_rejects_zero_timeout() {
    let err = ProviderHttpClientConfig {
        timeout: Duration::ZERO,
        connect_timeout: Duration::from_secs(1),
    }
    .build_client()
    .expect_err("zero timeout should be rejected");
    assert!(matches!(err, AppError::Config(message) if message.contains("must be greater than 0")));
}

#[test]
fn provider_http_client_config_rejects_zero_connect_timeout() {
    let err = ProviderHttpClientConfig {
        timeout: Duration::from_secs(1),
        connect_timeout: Duration::ZERO,
    }
    .build_client()
    .expect_err("zero connect timeout should be rejected");
    assert!(matches!(err, AppError::Config(message) if message.contains("must be greater than 0")));
}

#[tokio::test]
async fn registry_retries_retryable_complete_errors() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyProvider {
        provider_id: "flaky-retryable",
        attempts: Arc::clone(&attempts),
        fail_attempts: 1,
        retryable: true,
    };
    let mut registry = ProviderRegistry::new().with_request_retry_policy(RequestRetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(0),
        max_delay: Duration::from_millis(0),
    });
    registry.register(provider);
    let response = registry
        .complete(
            &model_ref("flaky-retryable", "flaky-model"),
            completion_request("flaky-model"),
        )
        .await
        .expect("completion should succeed after retry");
    assert_eq!(response.text, "ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_does_not_retry_non_retryable_complete_errors() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyProvider {
        provider_id: "flaky-non-retryable",
        attempts: Arc::clone(&attempts),
        fail_attempts: 2,
        retryable: false,
    };
    let mut registry = ProviderRegistry::new().with_request_retry_policy(RequestRetryPolicy {
        max_retries: 3,
        base_delay: Duration::from_millis(0),
        max_delay: Duration::from_millis(0),
    });
    registry.register(provider);
    let err = registry
        .complete(
            &model_ref("flaky-non-retryable", "flaky-model"),
            completion_request("flaky-model"),
        )
        .await
        .expect_err("non-retryable error should bubble up immediately");
    assert!(matches!(err, AppError::Provider(message) if message == "permanent failure"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn register_plugin_provider_exposes_models() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_plugin_provider(crate::plugin::ProviderDescriptor {
            id: "plugin-mock".to_owned(),
            display_name: "Plugin Mock".to_owned(),
            models: vec!["mock-model".to_owned()],
            endpoint: None,
            kind: crate::plugin::ProviderKind::Custom,
        })
        .expect("plugin provider should register");

    let models = registry
        .list_models("plugin-mock")
        .await
        .expect("plugin models should list");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.as_str(), "mock-model");
    assert_eq!(models[0].display_name.as_deref(), Some("Plugin Mock"));
}

#[tokio::test]
async fn plugin_registered_provider_completion_returns_clear_error() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_plugin_provider(crate::plugin::ProviderDescriptor {
            id: "plugin-mock".to_owned(),
            display_name: "Plugin Mock".to_owned(),
            models: vec!["mock-model".to_owned()],
            endpoint: None,
            kind: crate::plugin::ProviderKind::Custom,
        })
        .expect("plugin provider should register");

    let err = registry
        .complete(
            &model_ref("plugin-mock", "mock-model"),
            completion_request("mock-model"),
        )
        .await
        .expect_err("plugin provider completion should not be executable yet");

    assert!(
        matches!(err, AppError::Provider(message) if message.contains("does not implement completions"))
    );
}

#[test]
fn resolve_model_target_parses_explicit_model_reference() {
    let registry = ProviderRegistry::new();
    let resolved = registry
        .resolve_model_target("openai/openai/gpt-5", None)
        .expect("model reference should parse");
    assert_eq!(resolved, model_ref("openai", "openai/gpt-5"));
}

#[tokio::test]
async fn provider_model_returns_capabilities_for_listed_models() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyProvider {
        provider_id: "model-info",
        attempts,
        fail_attempts: 0,
        retryable: false,
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let model = registry
        .resolve_model(&model_ref("model-info", "flaky-model"))
        .await
        .expect("provider model should resolve");

    assert_eq!(model.provider_id, pid("model-info"));
    assert_eq!(model.id, mid("flaky-model"));
    assert_eq!(model.display_name.as_deref(), Some("Flaky Model"));
    assert_eq!(
        model.capabilities.tool_calling,
        CapabilitySupport::Supported
    );
    assert_eq!(model.capabilities.streaming, CapabilitySupport::Supported);
}

#[tokio::test]
async fn provider_model_synthesizes_capabilities_for_unlisted_model() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyProvider {
        provider_id: "model-fallback",
        attempts,
        fail_attempts: 0,
        retryable: false,
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let model = registry
        .resolve_model(&model_ref("model-fallback", "custom-unlisted-model"))
        .await
        .expect("provider model should synthesize missing entry");

    assert_eq!(model.provider_id, pid("model-fallback"));
    assert_eq!(model.id, mid("custom-unlisted-model"));
    assert_eq!(model.display_name, None);
    assert_eq!(
        model.capabilities.tool_calling,
        CapabilitySupport::Supported
    );
    assert_eq!(model.capabilities.streaming, CapabilitySupport::Supported);
}

#[test]
fn estimated_total_cost_prefers_matching_context_tier_and_charges_reasoning_as_output() {
    let metadata = ModelMetadata::default().with_pricing(sample_pricing());
    let usage = CompletionUsage {
        input_tokens: 250_000,
        output_tokens: 700,
        reasoning_tokens: 200,
        cache_write_tokens: 10_000,
        cache_read_tokens: 5_000,
        total_cost: 0.0,
    };

    let estimated =
        estimate_total_cost_from_metadata(&metadata, &usage).expect("pricing should estimate");

    assert!((estimated - 0.7617).abs() < 1e-9);
}

#[tokio::test]
async fn registry_complete_estimates_usage_cost_from_model_metadata() {
    let provider = PricedUsageProvider {
        provider_id: "priced-complete",
        metadata: ModelMetadata::default().with_pricing(sample_pricing()),
        usage: CompletionUsage {
            input_tokens: 1_000,
            output_tokens: 300,
            reasoning_tokens: 100,
            cache_write_tokens: 50,
            cache_read_tokens: 100,
            total_cost: 0.0,
        },
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let response = registry
        .complete(
            &model_ref("priced-complete", "priced-model"),
            completion_request("priced-model"),
        )
        .await
        .expect("completion should succeed");

    let usage = response.usage.expect("usage should be present");
    assert!((usage.total_cost - 0.00162).abs() < 1e-9);
}

#[tokio::test]
async fn registry_complete_stream_estimates_usage_cost_from_model_metadata() {
    let provider = PricedUsageProvider {
        provider_id: "priced-stream",
        metadata: ModelMetadata::default().with_pricing(sample_pricing()),
        usage: CompletionUsage {
            input_tokens: 1_000,
            output_tokens: 300,
            reasoning_tokens: 100,
            cache_write_tokens: 50,
            cache_read_tokens: 100,
            total_cost: 0.0,
        },
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let mut stream = registry
        .complete_stream(
            &model_ref("priced-stream", "priced-model"),
            completion_request("priced-model"),
        )
        .await
        .expect("stream should succeed");

    let mut completed_usage = None;
    while let Some(item) = stream.next().await {
        if let CompletionStreamEvent::Completed { usage, .. } =
            item.expect("stream item should succeed")
        {
            completed_usage = usage;
        }
    }

    let usage = completed_usage.expect("completed usage should be present");
    assert!((usage.total_cost - 0.00162).abs() < 1e-9);
}

#[tokio::test]
async fn provider_registry_list_models_applies_default_thinking_mode_synthesis() {
    let provider = ModeSynthProvider {
        provider_id: "mode-synth",
        family: CapabilityFamily::OpenAi,
        models: vec![Model::new("mode-synth", "gpt-5.2")],
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let listed = registry
        .list_models("mode-synth")
        .await
        .expect("provider models should list");
    let model = listed
        .into_iter()
        .find(|model| model.id.as_str() == "gpt-5.2")
        .expect("gpt-5.2 should be listed");

    assert!(model.thinking_modes.contains_key("no-thinking"));
    assert!(model.thinking_modes.contains_key("thinking-low"));
    assert!(model.thinking_modes.contains_key("thinking-medium"));
    assert!(model.thinking_modes.contains_key("thinking-high"));
    assert!(model.thinking_modes.contains_key("thinking-xhigh"));
}

#[tokio::test]
async fn provider_registry_list_models_applies_bedrock_opus_47_reasoning_and_capability_rules() {
    let provider = ModeSynthProvider {
        provider_id: "bedrock-synth",
        family: CapabilityFamily::Bedrock,
        models: vec![Model::new("bedrock-synth", "anthropic.claude-opus-4-7")],
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let listed = registry
        .list_models("bedrock-synth")
        .await
        .expect("provider models should list");
    let model = listed
        .into_iter()
        .find(|model| model.id.as_str() == "anthropic.claude-opus-4-7")
        .expect("anthropic.claude-opus-4-7 should be listed");

    assert!(model.thinking_modes.contains_key("thinking-low"));
    assert!(model.thinking_modes.contains_key("thinking-medium"));
    assert!(model.thinking_modes.contains_key("thinking-high"));
    assert!(model.thinking_modes.contains_key("thinking-xhigh"));
    assert!(model.thinking_modes.contains_key("thinking-max"));
    assert_eq!(
        model.capabilities.temperature_supported,
        CapabilitySupport::Unsupported
    );
}

#[tokio::test]
async fn registry_rejects_explicitly_unsupported_image_inputs_before_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = UnsupportedImageProvider {
        calls: Arc::clone(&calls),
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let err = registry
        .complete(
            &model_ref("unsupported-image", "unsupported-image-model"),
            CompletionRequest {
                model: mid("unsupported-image-model"),
                system: None,
                messages: vec![Message::prompt_parts(
                    crate::role::Role::User,
                    vec![PartContent::attachments(vec![AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_owned(),
                        source: AttachmentSource::DataUrl {
                            url: "data:image/png;base64,AAA".to_owned(),
                        },
                        filename: Some("pixel.png".to_owned()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: Some(1),
                        height: Some(1),
                        duration_ms: None,
                        page_count: None,
                    }])],
                )],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(16),
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                verbosity: None,
                request_override: Default::default(),
                response_format: None,
            },
        )
        .await
        .expect_err("unsupported image request should be rejected");

    assert!(
        matches!(err, AppError::Provider(message) if message.contains("does not support requested input modalities") && message.contains("image (`pixel.png`)"))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn registry_retries_stream_when_startup_fails_before_events() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyStreamProvider {
        provider_id: "flaky-stream-startup",
        stream_starts: Arc::clone(&stream_starts),
        mode: StreamFailureMode::StartupRetryableOnceThenSuccess,
        resume_policy: StreamResumePolicy::Disabled,
    };
    let mut registry = ProviderRegistry::new().with_request_retry_policy(RequestRetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(0),
        max_delay: Duration::from_millis(0),
    });
    registry.register(provider);
    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-startup", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("stream should recover after startup retry");
    let mut text = String::new();
    let mut done = false;
    while let Some(item) = stream.next().await {
        match item.expect("stream item should succeed") {
            CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
            CompletionStreamEvent::Completed { .. } => done = true,
            CompletionStreamEvent::ToolCallDelta { .. } => {}
            CompletionStreamEvent::ThinkingDelta { .. } => {}
        }
    }
    assert_eq!(text, "ok");
    assert!(done);
    assert_eq!(stream_starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_default_retry_policy_recovers_after_multiple_startup_failures() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = MultiStartupFailureProvider {
        provider_id: "flaky-stream-default-retries",
        stream_starts: Arc::clone(&stream_starts),
        fail_attempts: 2,
    };
    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-default-retries", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("default retry policy should absorb multiple startup failures");
    let mut text = String::new();
    let mut done = false;
    while let Some(item) = stream.next().await {
        match item.expect("stream item should succeed") {
            CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
            CompletionStreamEvent::Completed { .. } => done = true,
            CompletionStreamEvent::ToolCallDelta { .. } => {}
            CompletionStreamEvent::ThinkingDelta { .. } => {}
        }
    }

    assert_eq!(text, "ok");
    assert!(done);
    assert_eq!(stream_starts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn registry_retries_stream_when_first_item_is_retryable_error() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyStreamProvider {
        provider_id: "flaky-stream-first-item",
        stream_starts: Arc::clone(&stream_starts),
        mode: StreamFailureMode::FirstItemRetryableOnceThenSuccess,
        resume_policy: StreamResumePolicy::Disabled,
    };
    let mut registry = ProviderRegistry::new().with_request_retry_policy(RequestRetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(0),
        max_delay: Duration::from_millis(0),
    });
    registry.register(provider);
    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-first-item", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("stream should recover when first item fails");
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        match item.expect("stream item should succeed") {
            CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
            CompletionStreamEvent::Completed { .. }
            | CompletionStreamEvent::ToolCallDelta { .. } => {}
            CompletionStreamEvent::ThinkingDelta { .. } => {}
        }
    }
    assert_eq!(text, "ok");
    assert_eq!(stream_starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_does_not_restart_stream_after_first_event_is_emitted() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyStreamProvider {
        provider_id: "flaky-stream-mid",
        stream_starts: Arc::clone(&stream_starts),
        mode: StreamFailureMode::MidStreamRetryableNoRestart,
        resume_policy: StreamResumePolicy::Disabled,
    };
    let mut registry = ProviderRegistry::new().with_request_retry_policy(RequestRetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(0),
        max_delay: Duration::from_millis(0),
    });
    registry.register(provider);
    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-mid", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("stream should start");
    let first = stream
        .next()
        .await
        .expect("first item should exist")
        .expect("first item should be success");
    assert!(
        matches!(first, CompletionStreamEvent::TextDelta { ref delta, .. } if delta == "partial")
    );
    let second = stream
        .next()
        .await
        .expect("second item should exist")
        .expect_err("second item should be error");
    assert!(second.retryable());
    assert_eq!(stream_starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn registry_restarts_stream_after_output_with_replay_safe_prefix() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyStreamProvider {
        provider_id: "flaky-stream-replay",
        stream_starts: Arc::clone(&stream_starts),
        mode: StreamFailureMode::MidStreamReplaySafeResumeThenSuccess,
        resume_policy: StreamResumePolicy::ReplaySafePrefix,
    };
    let mut registry = ProviderRegistry::new()
        .with_request_retry_policy(RequestRetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
        })
        .with_stream_replay_policy(StreamReplayPolicy {
            max_retries_after_output: 2,
            max_tracked_events: 32,
        });
    registry.register(provider);
    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-replay", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("stream should resume after mid-stream retryable failure");
    let mut text = String::new();
    let mut done = false;
    while let Some(item) = stream.next().await {
        match item.expect("stream item should succeed") {
            CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
            CompletionStreamEvent::Completed { .. } => done = true,
            CompletionStreamEvent::ToolCallDelta { .. } => {}
            CompletionStreamEvent::ThinkingDelta { .. } => {}
        }
    }
    assert_eq!(text, "partialfinal");
    assert!(done);
    assert_eq!(stream_starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_aborts_on_replay_prefix_divergence() {
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let provider = FlakyStreamProvider {
        provider_id: "flaky-stream-diverge",
        stream_starts: Arc::clone(&stream_starts),
        mode: StreamFailureMode::MidStreamReplaySafeDiverges,
        resume_policy: StreamResumePolicy::ReplaySafePrefix,
    };
    let mut registry = ProviderRegistry::new()
        .with_request_retry_policy(RequestRetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
        })
        .with_stream_replay_policy(StreamReplayPolicy {
            max_retries_after_output: 2,
            max_tracked_events: 32,
        });
    registry.register(provider);
    let mut stream = registry
        .complete_stream(
            &model_ref("flaky-stream-diverge", "flaky-stream-model"),
            CompletionRequest {
                model: mid("flaky-stream-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-stream-model")
            },
        )
        .await
        .expect("stream should start");
    let first = stream
        .next()
        .await
        .expect("first item should exist")
        .expect("first item should be success");
    assert!(
        matches!(first, CompletionStreamEvent::TextDelta { ref delta, .. } if delta == "partial")
    );
    let second = stream
        .next()
        .await
        .expect("second item should exist")
        .expect_err("second item should fail due to replay divergence");
    assert!(
        matches!(second, AppError::Provider(message) if message.contains("replay prefix diverged"))
    );
    assert_eq!(stream_starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn named_provider_rewrites_stream_event_provider_ids_to_registered_alias() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let native = FlakyProvider {
        provider_id: "native-openai",
        attempts,
        fail_attempts: 0,
        retryable: false,
    };
    let alias = NamedProvider::new("configured-codex", Arc::new(native));
    let mut registry = ProviderRegistry::new();
    registry.register(alias);

    let mut stream = registry
        .complete_stream(
            &model_ref("configured-codex", "flaky-model"),
            CompletionRequest {
                model: mid("flaky-model"),
                max_output_tokens: Some(32),
                ..completion_request("flaky-model")
            },
        )
        .await
        .expect("stream should start");

    let first = stream
        .next()
        .await
        .expect("first item should exist")
        .expect("first item should succeed");
    match first {
        CompletionStreamEvent::TextDelta { provider_id, .. } => {
            assert_eq!(provider_id.as_str(), "configured-codex");
        }
        other => panic!("unexpected first event: {other:?}"),
    }

    let second = stream
        .next()
        .await
        .expect("completed item should exist")
        .expect("completed item should succeed");
    match second {
        CompletionStreamEvent::Completed { provider_id, .. } => {
            assert_eq!(provider_id.as_str(), "configured-codex");
        }
        other => panic!("unexpected second event: {other:?}"),
    }
}
