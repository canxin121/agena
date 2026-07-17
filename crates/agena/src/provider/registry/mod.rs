use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use tracing::Instrument;

use crate::config::{AgenaToolMode, ProviderNativeToolsConfig};
use crate::error::{AppError, ProviderErrorKind};
use crate::model::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelPricing, ModelPricingTier,
    ModelRef, ModelSpeedMode, ModelThinkingMode, ProviderId,
};
use crate::plugin::ProviderDescriptor;

use super::core::{
    ForwardingModelRuntime, impl_model_runtime_target_defaults, impl_model_runtime_target_methods,
    remap_stream_event_provider_id,
};
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionUsage, ModelRuntime,
    ProviderHttpClientConfig, StreamResumePolicy, wire_message,
};

const REQUEST_MAX_RETRIES: u32 = 5;
const RETRY_BASE_DELAY_MS: u64 = 250;
const RETRY_MAX_DELAY_MS: u64 = 2_000;
const STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT: u32 = 5;
const STREAM_REPLAY_MAX_TRACKED_EVENTS: usize = 2048;

#[derive(Debug, Clone, Copy)]
struct RequestRetryPolicy {
    max_retries: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl Default for RequestRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: REQUEST_MAX_RETRIES,
            base_delay: Duration::from_millis(RETRY_BASE_DELAY_MS),
            max_delay: Duration::from_millis(RETRY_MAX_DELAY_MS),
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
}

fn assign_catalog_model_id(model: &mut Model) {
    model.catalog_model_id = catalog_model_id_for(&model.id);
}

fn catalog_model_id_for(model_id: &ModelId) -> Option<ModelId> {
    let catalog_model_id = crate::model_catalog::canonical_model_catalog_id(model_id.as_ref());
    (!catalog_model_id.is_empty()).then(|| ModelId::new(catalog_model_id))
}

fn provider_not_found_error(provider_id: &str) -> AppError {
    AppError::Config(format!("provider not found: {provider_id}"))
}

#[derive(Debug, Clone, Copy)]
enum ModeHydration {
    FillEmpty,
    OverrideIfPresent,
}

fn hydrate_model_from_provider(
    provider: &dyn ModelRuntime,
    model: &mut Model,
    adapter_id: Option<&AdapterId>,
    mode_hydration: ModeHydration,
) {
    let adapter_id = adapter_id.or(model.adapter_id.as_ref()).cloned();
    let fallback = provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model.id);
    let current_capabilities = std::mem::take(&mut model.capabilities);
    model.capabilities = if current_capabilities.is_default_placeholder() {
        fallback
    } else {
        current_capabilities.merged_with_fallbacks_from(&fallback)
    };

    let metadata_fallback = provider.model_metadata_for_adapter(adapter_id.as_ref(), &model.id);
    model.metadata = model
        .metadata
        .clone()
        .merged_with_fallbacks_from(&metadata_fallback);

    let thinking_modes = provider.model_thinking_modes_for_adapter(adapter_id.as_ref(), &model.id);
    let speed_modes = provider.model_speed_modes_for_adapter(adapter_id.as_ref(), &model.id);
    match mode_hydration {
        ModeHydration::FillEmpty => {
            if model.thinking_modes.is_empty() {
                model.thinking_modes = thinking_modes;
            }
            if model.speed_modes.is_empty() {
                model.speed_modes = speed_modes;
            }
        }
        ModeHydration::OverrideIfPresent => {
            if !thinking_modes.is_empty() {
                model.thinking_modes = thinking_modes;
            }
            if !speed_modes.is_empty() {
                model.speed_modes = speed_modes;
            }
        }
    }
}

fn hydrated_model_from_provider(
    provider: &dyn ModelRuntime,
    mut model: Model,
    adapter_id: Option<&AdapterId>,
    mode_hydration: ModeHydration,
) -> Model {
    hydrate_model_from_provider(provider, &mut model, adapter_id, mode_hydration);
    model
}

fn prepare_listed_model(
    provider: &dyn ModelRuntime,
    provider_id: &str,
    model: &mut Model,
    assign_catalog_id: bool,
) {
    model.provider_id = ProviderId::new(provider_id.to_owned());
    if assign_catalog_id {
        assign_catalog_model_id(model);
    }
    hydrate_model_from_provider(provider, model, None, ModeHydration::FillEmpty);
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

    // CompletionUsage already stores visible output separately from reasoning.
    let visible_output_tokens = usage.output_tokens;
    let estimated = (usage.input_tokens as f64 * rates.input.unwrap_or(0.0) / 1_000_000.0)
        + (visible_output_tokens as f64 * rates.output.unwrap_or(0.0) / 1_000_000.0)
        + (usage.reasoning_tokens as f64 * rates.output.unwrap_or(0.0) / 1_000_000.0)
        + (usage.cache_read_tokens as f64 * rates.cache_read.unwrap_or(0.0) / 1_000_000.0)
        + (usage.cache_write_tokens as f64 * rates.cache_write.unwrap_or(0.0) / 1_000_000.0);
    estimated.is_finite().then_some(estimated)
}

fn hydrate_usage_cost_from_provider_metadata(
    provider: &dyn ModelRuntime,
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
            max_retries_after_output: STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT,
            max_tracked_events: STREAM_REPLAY_MAX_TRACKED_EVENTS,
        }
    }
}

impl StreamReplayPolicy {
    fn enabled(self, provider_policy: StreamResumePolicy) -> bool {
        matches!(provider_policy, StreamResumePolicy::ReplaySafePrefix)
            && self.max_retries_after_output > 0
            && self.max_tracked_events > 0
    }
}

pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn ModelRuntime>>,
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
    target: Arc<dyn ModelRuntime>,
}

impl NamedProvider {
    pub fn new(provider_id: impl Into<String>, target: Arc<dyn ModelRuntime>) -> Self {
        Self {
            provider_id: provider_id.into(),
            target,
        }
    }

    fn rewrite_response_provider_id(&self, response: &mut CompletionResponse) {
        response.provider_id = ProviderId::new(self.provider_id.clone());
    }

    fn remap_stream_provider_id(
        &self,
        stream: std::pin::Pin<
            Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>,
        >,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>> {
        let provider_id = ProviderId::new(self.provider_id.clone());
        Box::pin(stream.map(move |item| {
            let provider_id = provider_id.clone();
            item.map(|event| remap_stream_event_provider_id(&provider_id, event))
        }))
    }
}

#[async_trait]
impl ForwardingModelRuntime for NamedProvider {
    fn target(&self) -> &dyn ModelRuntime {
        self.target.as_ref()
    }

    fn rewrite_response(
        &self,
        adapter_id: Option<&AdapterId>,
        mut response: CompletionResponse,
    ) -> CompletionResponse {
        let _ = adapter_id;
        self.rewrite_response_provider_id(&mut response);
        response
    }

    fn rewrite_stream(
        &self,
        adapter_id: Option<&AdapterId>,
        stream: super::core::CompletionEventStream,
    ) -> super::core::CompletionEventStream {
        let _ = adapter_id;
        self.remap_stream_provider_id(stream)
    }
}

#[async_trait]
impl ModelRuntime for NamedProvider {
    fn id(&self) -> &str {
        self.provider_id.as_ref()
    }

    impl_model_runtime_target_defaults!();

    impl_model_runtime_target_methods! {
        fn model_capabilities / model_capabilities_for_adapter (&self, model: &ModelId) -> super::ModelCapabilities;
        fn model_metadata / model_metadata_for_adapter (&self, model: &ModelId) -> super::ModelMetadata;
        fn model_thinking_modes / model_thinking_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode>;
        fn model_speed_modes / model_speed_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode>;
        fn supports_prompt_continuation / supports_prompt_continuation_for_adapter (&self, model: &ModelId) -> bool;
        fn prompt_cache_shape / prompt_cache_shape_for_adapter (&self, model: &ModelId) -> Option<super::PromptCacheShape>;
        fn provider_native_tools_config / provider_native_tools_config_for_adapter (&self, model: &ModelId) -> ProviderNativeToolsConfig;
        fn agena_tool_mode / agena_tool_mode_for_adapter (&self, model: &ModelId) -> AgenaToolMode;
    }

    fn validate_provider_native_tools_request(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        self.target
            .validate_provider_native_tools_request(adapter_id, request)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        for model in &mut models {
            prepare_listed_model(
                self.target.as_ref(),
                self.provider_id.as_ref(),
                model,
                false,
            );
        }
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.forward_complete(None, request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        self.forward_complete(adapter_id, request).await
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.forward_compact_conversation(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.forward_compact_conversation(adapter_id, request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.forward_complete_stream(None, request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.forward_complete_stream(adapter_id, request).await
    }
}

#[async_trait]
impl ModelRuntime for PluginRegisteredProvider {
    fn id(&self) -> &str {
        self.id.as_ref()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        Ok(self
            .models
            .iter()
            .map(|model| Model {
                provider_id: ProviderId::new(self.id.as_str()),
                adapter_id: None,
                id: ModelId::new(model.as_ref()),
                catalog_model_id: None,
                display_name: Some(self.display_name.clone()),
                capabilities: ModelCapabilities::default(),
                metadata: ModelMetadata::default(),
                thinking_modes: std::collections::BTreeMap::new(),
                speed_modes: std::collections::BTreeMap::new(),
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

    pub fn build_http_client(
        config: ProviderHttpClientConfig,
    ) -> Result<reqwest::Client, AppError> {
        config.build_client()
    }

    pub fn register<P>(&mut self, provider: P)
    where
        P: ModelRuntime + 'static,
    {
        self.providers
            .insert(provider.id().to_owned(), Arc::new(provider));
    }

    pub fn register_arc(&mut self, provider: Arc<dyn ModelRuntime>) {
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

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ModelRuntime>> {
        self.providers.get(provider_id).cloned()
    }

    pub(super) fn require_provider(
        &self,
        provider_id: &str,
    ) -> Result<Arc<dyn ModelRuntime>, AppError> {
        self.get(provider_id)
            .ok_or_else(|| provider_not_found_error(provider_id))
    }

    pub(super) fn provider_for_model_ref(
        &self,
        model: &ModelRef,
    ) -> Result<Arc<dyn ModelRuntime>, AppError> {
        self.require_provider(model.provider_id.as_ref())
    }

    pub(super) fn use_model_ref_provider<T>(
        &self,
        model: &ModelRef,
        map: impl FnOnce(&dyn ModelRuntime, Option<&AdapterId>, &ModelId) -> T,
    ) -> Result<T, AppError> {
        let provider = self.provider_for_model_ref(model)?;
        Ok(map(
            provider.as_ref(),
            model.adapter_id.as_ref(),
            &model.model_id,
        ))
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
    provider: &dyn ModelRuntime,
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

    if !unsupported.is_empty() {
        let details = unsupported
            .into_iter()
            .map(|(modality, label)| format!("{modality} (`{label}`)"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::Provider(format!(
            "provider `{}` model `{}` explicitly does not support requested input modalities: {details}",
            model.provider_id, model.model_id
        )));
    }

    provider.validate_provider_native_tools_request(model.adapter_id.as_ref(), request)
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
mod tests {
    use super::estimate_total_cost_from_metadata;
    use crate::{
        model::{ModelMetadata, ModelPricing},
        provider::CompletionUsage,
    };

    #[test]
    fn cost_estimation_prices_visible_output_and_reasoning_once_each() {
        let metadata = ModelMetadata {
            pricing: Some(ModelPricing {
                output_usd_per_million_tokens: Some("1".to_owned()),
                ..ModelPricing::default()
            }),
            ..ModelMetadata::default()
        };
        let usage = CompletionUsage {
            input_tokens: 0,
            output_tokens: 50,
            reasoning_tokens: 30,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            total_cost: 0.0,
        };

        let estimated =
            estimate_total_cost_from_metadata(&metadata, &usage).expect("estimated cost");
        assert!((estimated - 0.000_080).abs() < f64::EPSILON);
    }
}
