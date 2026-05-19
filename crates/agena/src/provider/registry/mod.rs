use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tracing::Instrument;

use crate::error::{AppError, ProviderErrorKind};
use crate::model::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelPricing, ModelPricingTier,
    ModelRef, ModelSpeedMode, ModelThinkingMode, ProviderId,
};
use crate::plugin::ProviderDescriptor;

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionUsage, ModelProvider,
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig, StreamResumePolicy, wire_message,
};

#[derive(Debug, Clone, Copy)]
struct RequestRetryPolicy {
    max_retries: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl Default for RequestRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: ProviderRequestRetryConfig::default().max_retries,
            base_delay: ProviderRequestRetryConfig::default().base_delay,
            max_delay: ProviderRequestRetryConfig::default().max_delay,
        }
    }
}

impl RequestRetryPolicy {
    fn delay_for_retry(&self, retry_index: u32) -> Duration {
        let capped_retry_index = retry_index.min(20);
        let multiplier = 1_u128 << capped_retry_index;
        let base_ms = self.base_delay.as_millis();
        let max_ms = self.max_delay.as_millis();
        let next_ms = base_ms.saturating_mul(multiplier).min(max_ms);
        Duration::from_millis(next_ms as u64)
    }

    fn from_config(config: ProviderRequestRetryConfig) -> Self {
        Self {
            max_retries: config.max_retries,
            base_delay: config.base_delay,
            max_delay: config.max_delay.max(config.base_delay),
        }
    }
}

fn assign_catalog_model_id(model: &mut Model) {
    let catalog_model_id = crate::model_catalog::canonical_model_catalog_id(model.id.as_str());
    if catalog_model_id.is_empty() {
        model.catalog_model_id = None;
        return;
    }
    model.catalog_model_id = Some(ModelId::new(catalog_model_id));
}

#[derive(Debug, Clone, Copy, Default)]
struct PricingRates {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

impl PricingRates {
    fn from_pricing(pricing: &ModelPricing) -> Self {
        Self {
            input: parse_pricing_rate(pricing.input_usd_per_million_tokens.as_deref()),
            output: parse_pricing_rate(pricing.output_usd_per_million_tokens.as_deref()),
            cache_read: parse_pricing_rate(pricing.cache_read_usd_per_million_tokens.as_deref()),
            cache_write: parse_pricing_rate(pricing.cache_write_usd_per_million_tokens.as_deref()),
        }
    }

    fn apply_tier(&mut self, tier: &ModelPricingTier) {
        if let Some(value) = parse_pricing_rate(tier.input_usd_per_million_tokens.as_deref()) {
            self.input = Some(value);
        }
        if let Some(value) = parse_pricing_rate(tier.output_usd_per_million_tokens.as_deref()) {
            self.output = Some(value);
        }
        if let Some(value) = parse_pricing_rate(tier.cache_read_usd_per_million_tokens.as_deref()) {
            self.cache_read = Some(value);
        }
        if let Some(value) = parse_pricing_rate(tier.cache_write_usd_per_million_tokens.as_deref())
        {
            self.cache_write = Some(value);
        }
    }

    fn is_empty(self) -> bool {
        self.input.is_none()
            && self.output.is_none()
            && self.cache_read.is_none()
            && self.cache_write.is_none()
    }
}

fn parse_pricing_rate(value: Option<&str>) -> Option<f64> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn estimate_total_cost_from_metadata(
    metadata: &ModelMetadata,
    usage: &CompletionUsage,
) -> Option<f64> {
    let pricing = metadata.pricing.as_ref()?;
    let context_tokens = usage
        .input_tokens
        .saturating_add(usage.cache_read_tokens)
        .saturating_add(usage.cache_write_tokens);
    let matching_context_tier = pricing
        .tiers
        .iter()
        .filter(|tier| tier.tier_type.as_deref() == Some("context"))
        .filter_map(|tier| tier.size_tokens.map(|size| (u64::from(size), tier)))
        .filter(|(size, _)| context_tokens > *size)
        .max_by_key(|(size, _)| *size)
        .map(|(_, tier)| tier);

    let mut rates = PricingRates::from_pricing(pricing);
    if let Some(tier) = matching_context_tier {
        rates.apply_tier(tier);
    }
    if rates.is_empty() {
        return None;
    }

    let visible_output_tokens = usage.output_tokens.saturating_sub(usage.reasoning_tokens);
    let estimated = (usage.input_tokens as f64 * rates.input.unwrap_or(0.0) / 1_000_000.0)
        + (visible_output_tokens as f64 * rates.output.unwrap_or(0.0) / 1_000_000.0)
        + (usage.reasoning_tokens as f64 * rates.output.unwrap_or(0.0) / 1_000_000.0)
        + (usage.cache_read_tokens as f64 * rates.cache_read.unwrap_or(0.0) / 1_000_000.0)
        + (usage.cache_write_tokens as f64 * rates.cache_write.unwrap_or(0.0) / 1_000_000.0);
    estimated.is_finite().then_some(estimated)
}

fn hydrate_usage_cost_from_provider_metadata(
    provider: &dyn ModelProvider,
    model: &ModelRef,
    usage: &mut Option<CompletionUsage>,
) {
    let Some(usage) = usage.as_mut() else {
        return;
    };
    if usage.total_cost > 0.0 {
        return;
    }
    let metadata = provider.model_metadata_for_adapter(model.adapter_id.as_ref(), &model.model_id);
    if let Some(estimated) = estimate_total_cost_from_metadata(&metadata, usage) {
        usage.total_cost = estimated;
    }
}

#[derive(Debug, Clone, Copy)]
struct StreamReplayPolicy {
    max_retries_after_output: u32,
    max_tracked_events: usize,
}

impl Default for StreamReplayPolicy {
    fn default() -> Self {
        Self {
            max_retries_after_output: ProviderStreamReplayConfig::default()
                .max_retries_after_output,
            max_tracked_events: ProviderStreamReplayConfig::default().max_tracked_events,
        }
    }
}

impl StreamReplayPolicy {
    fn from_config(config: ProviderStreamReplayConfig) -> Self {
        Self {
            max_retries_after_output: config.max_retries_after_output,
            max_tracked_events: config.max_tracked_events,
        }
    }

    fn enabled(self, provider_policy: StreamResumePolicy) -> bool {
        matches!(provider_policy, StreamResumePolicy::ReplaySafePrefix)
            && self.max_retries_after_output > 0
            && self.max_tracked_events > 0
    }
}

pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    retry_policy: RequestRetryPolicy,
    stream_replay_policy: StreamReplayPolicy,
}

struct PluginRegisteredProvider {
    id: String,
    display_name: String,
    default_model: ModelId,
    models: Vec<ModelId>,
}

#[derive(Clone)]
pub struct NamedProvider {
    provider_id: String,
    target: Arc<dyn ModelProvider>,
}

impl NamedProvider {
    pub fn new(provider_id: impl Into<String>, target: Arc<dyn ModelProvider>) -> Self {
        Self {
            provider_id: provider_id.into(),
            target,
        }
    }
}

#[async_trait]
impl ModelProvider for NamedProvider {
    fn id(&self) -> &str {
        self.provider_id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        self.target.default_model()
    }

    fn default_adapter(&self) -> Option<&AdapterId> {
        self.target.default_adapter()
    }

    fn model_capabilities(&self, model: &ModelId) -> super::ModelCapabilities {
        self.target.model_capabilities(model)
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> super::ModelCapabilities {
        self.target
            .model_capabilities_for_adapter(adapter_id, model)
    }

    fn model_metadata(&self, model: &ModelId) -> super::ModelMetadata {
        self.target.model_metadata(model)
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> super::ModelMetadata {
        self.target.model_metadata_for_adapter(adapter_id, model)
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        self.target.model_thinking_modes(model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        self.target
            .model_thinking_modes_for_adapter(adapter_id, model)
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        self.target.model_speed_modes(model)
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.target.model_speed_modes_for_adapter(adapter_id, model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.target.supports_prompt_continuation(model)
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.target
            .supports_prompt_continuation_for_adapter(adapter_id, model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<super::PromptCacheShape> {
        self.target.prompt_cache_shape(model)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<super::PromptCacheShape> {
        self.target
            .prompt_cache_shape_for_adapter(adapter_id, model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        for model in &mut models {
            model.provider_id = ProviderId::new(self.provider_id.clone());
            let fallback = self
                .target
                .model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.id);
            let current_capabilities = std::mem::take(&mut model.capabilities);
            model.capabilities = if current_capabilities.is_default_placeholder() {
                fallback.clone()
            } else {
                current_capabilities.with_fallbacks_from(&fallback)
            };
            let metadata_fallback = self
                .target
                .model_metadata_for_adapter(model.adapter_id.as_ref(), &model.id);
            model.metadata = model
                .metadata
                .clone()
                .with_fallbacks_from(&metadata_fallback);
            if model.thinking_modes.is_empty() {
                model.thinking_modes = self
                    .target
                    .model_thinking_modes_for_adapter(model.adapter_id.as_ref(), &model.id);
            }
            if model.speed_modes.is_empty() {
                model.speed_modes = self
                    .target
                    .model_speed_modes_for_adapter(model.adapter_id.as_ref(), &model.id);
            }
        }
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let mut response = self.target.complete(request).await?;
        response.provider_id = ProviderId::new(self.provider_id.clone());
        Ok(response)
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let mut response = self
            .target
            .complete_for_adapter(adapter_id, request)
            .await?;
        response.provider_id = ProviderId::new(self.provider_id.clone());
        Ok(response)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let provider_id = self.provider_id.clone();
        let stream = self.target.complete_stream(request).await?;
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
            Box::pin(stream.map(move |item| {
                item.map(|event| match event {
                    CompletionStreamEvent::TextDelta { model, delta, .. } => {
                        CompletionStreamEvent::TextDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model,
                            delta,
                        }
                    }
                    CompletionStreamEvent::ThinkingDelta { model, delta, .. } => {
                        CompletionStreamEvent::ThinkingDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model,
                            delta,
                        }
                    }
                    CompletionStreamEvent::ToolCallDelta {
                        model,
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                        ..
                    } => CompletionStreamEvent::ToolCallDelta {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model,
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                    },
                    CompletionStreamEvent::Completed {
                        model,
                        finish_reason,
                        usage,
                        provider_metadata,
                        ..
                    } => CompletionStreamEvent::Completed {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model,
                        finish_reason,
                        usage,
                        provider_metadata,
                    },
                })
            }));
        Ok(Box::pin(stream))
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let provider_id = self.provider_id.clone();
        let stream = self
            .target
            .complete_stream_for_adapter(adapter_id, request)
            .await?;
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
            Box::pin(stream.map(move |item| {
                item.map(|event| match event {
                    CompletionStreamEvent::TextDelta { model, delta, .. } => {
                        CompletionStreamEvent::TextDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model,
                            delta,
                        }
                    }
                    CompletionStreamEvent::ThinkingDelta { model, delta, .. } => {
                        CompletionStreamEvent::ThinkingDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model,
                            delta,
                        }
                    }
                    CompletionStreamEvent::ToolCallDelta {
                        model,
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                        ..
                    } => CompletionStreamEvent::ToolCallDelta {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model,
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                    },
                    CompletionStreamEvent::Completed {
                        model,
                        finish_reason,
                        usage,
                        provider_metadata,
                        ..
                    } => CompletionStreamEvent::Completed {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model,
                        finish_reason,
                        usage,
                        provider_metadata,
                    },
                })
            }));
        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl ModelProvider for PluginRegisteredProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        Ok(self
            .models
            .iter()
            .map(|model| {
                Model::new(self.id.as_str(), model.as_str())
                    .with_display_name(self.display_name.clone())
            })
            .collect())
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Err(AppError::Provider(format!(
            "plugin-registered provider `{}` does not implement completions",
            self.id
        )))
    }
}

mod completion;
mod listing;
mod resolution;

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            retry_policy: RequestRetryPolicy::default(),
            stream_replay_policy: StreamReplayPolicy::default(),
        }
    }

    pub fn with_runtime_config(config: ProviderRuntimeConfig) -> Self {
        Self::new()
            .with_request_retry_policy(RequestRetryPolicy::from_config(config.request_retry))
            .with_stream_replay_policy(StreamReplayPolicy::from_config(config.stream_replay))
    }

    pub fn build_http_client(
        config: ProviderHttpClientConfig,
    ) -> Result<reqwest::Client, AppError> {
        config.build_client()
    }

    fn with_request_retry_policy(mut self, retry_policy: RequestRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    fn with_stream_replay_policy(mut self, stream_replay_policy: StreamReplayPolicy) -> Self {
        self.stream_replay_policy = stream_replay_policy;
        self
    }

    pub fn register<P>(&mut self, provider: P)
    where
        P: ModelProvider + 'static,
    {
        self.providers
            .insert(provider.id().to_owned(), Arc::new(provider));
    }

    pub fn register_arc(&mut self, provider: Arc<dyn ModelProvider>) {
        self.providers.insert(provider.id().to_owned(), provider);
    }

    pub fn remove(&mut self, provider_id: &str) {
        self.providers.remove(provider_id);
    }

    pub fn register_plugin_provider(
        &mut self,
        descriptor: ProviderDescriptor,
    ) -> Result<(), AppError> {
        let id = descriptor.id.trim();
        if id.is_empty() {
            return Err(AppError::Config(
                "plugin provider id cannot be empty".to_owned(),
            ));
        }
        let models = descriptor
            .models
            .iter()
            .map(|model| ModelId::try_new(model.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| AppError::Config(format!("invalid plugin provider model: {err}")))?;
        let default_model = models
            .first()
            .cloned()
            .unwrap_or_else(|| ModelId::new("default"));
        self.register(PluginRegisteredProvider {
            id: id.to_owned(),
            display_name: descriptor.display_name,
            default_model,
            models,
        });
        Ok(())
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ModelProvider>> {
        self.providers.get(provider_id).cloned()
    }

    fn should_retry_error(&self, err: &AppError, retry_index: u32) -> bool {
        err.retryable() && retry_index < self.retry_policy.max_retries
    }

    async fn call_with_retry<T, F, Fut>(
        &self,
        provider_id: &str,
        operation: &str,
        mut op: F,
    ) -> Result<T, AppError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, AppError>>,
    {
        let request_span = tracing::info_span!("provider.request", provider_id, operation,);
        let mut retry_index = 0_u32;
        loop {
            let attempt = retry_index + 1;
            let started_at = Instant::now();
            tracing::debug!(
                provider_id,
                operation,
                attempt,
                status = "attempt_started",
                "provider operation attempt started"
            );

            match op().instrument(request_span.clone()).await {
                Ok(value) => {
                    tracing::info!(
                        provider_id,
                        operation,
                        attempt,
                        retries = retry_index,
                        latency_ms = elapsed_ms(started_at),
                        status = "success",
                        "provider operation attempt succeeded"
                    );
                    return Ok(value);
                }
                Err(err) => {
                    let reason = retry_reason(&err);
                    if !self.should_retry_error(&err, retry_index) {
                        tracing::error!(
                            provider_id,
                            operation,
                            attempt,
                            retries = retry_index,
                            latency_ms = elapsed_ms(started_at),
                            status = "failed",
                            retry_reason = reason,
                            error = %err,
                            "provider operation attempt failed"
                        );
                        return Err(err);
                    }

                    let delay = self.retry_policy.delay_for_retry(retry_index);
                    tracing::warn!(
                        provider_id,
                        operation,
                        attempt,
                        retries = retry_index,
                        max_retries = self.retry_policy.max_retries,
                        latency_ms = elapsed_ms(started_at),
                        status = "retry_scheduled",
                        retry_reason = reason,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "provider operation attempt failed with retryable error; scheduling retry"
                    );
                    tokio::time::sleep(delay).await;
                    retry_index += 1;
                }
            }
        }
    }
}

fn validate_request_capabilities(
    model: &ModelRef,
    provider: &dyn ModelProvider,
    request: &CompletionRequest,
) -> Result<(), AppError> {
    let capabilities =
        provider.model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.model_id);

    let mut unsupported = Vec::new();
    for message in &request.messages {
        for part in wire_message::project(message) {
            let wire_message::WirePart::Attachment { item } = part else {
                continue;
            };

            if let Some(modality) = capabilities.unsupported_attachment_modality(&item) {
                unsupported.push((modality, item.summary_label()));
            }
        }
    }

    if unsupported.is_empty() {
        return Ok(());
    }

    let details = unsupported
        .into_iter()
        .map(|(modality, label)| format!("{} (`{label}`)", modality.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(AppError::Provider(format!(
        "provider `{}` model `{}` explicitly does not support requested input modalities: {details}",
        model.provider_id, model.model_id
    )))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis() as u64
}

fn stream_resume_policy_label(policy: StreamResumePolicy) -> &'static str {
    match policy {
        StreamResumePolicy::Disabled => "disabled",
        StreamResumePolicy::ReplaySafePrefix => "replay_safe_prefix",
    }
}

fn retry_reason(err: &AppError) -> &'static str {
    match err {
        AppError::Http(inner) if inner.is_timeout() => "http_timeout",
        AppError::Http(inner) if inner.is_connect() => "http_connect",
        AppError::HttpStatus {
            kind, retryable, ..
        }
        | AppError::ProviderClassified {
            kind, retryable, ..
        } if *retryable => provider_error_kind_label(*kind),
        AppError::Config(_) => "config_error",
        AppError::Provider(_) => "provider_error",
        AppError::Database(_) => "database_error",
        AppError::SerdeJson(_) => "serde_json_error",
        AppError::Http(_) => "http_error",
        AppError::Io(_) => "io_error",
        AppError::InvalidRole(_) => "invalid_role",
        AppError::Internal(_) => "internal_error",
        _ => "non_retryable",
    }
}

fn provider_error_kind_label(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::ApiError => "provider_api_error",
        ProviderErrorKind::ContextOverflow => "context_overflow",
    }
}

#[cfg(test)]
mod tests;
