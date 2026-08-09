pub(super) use agena_domain::ModelMetadata;
use agena_domain::*;
use agena_provider::{CompletionUsage, StreamResumePolicy};

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_core::Stream;
use tracing::Instrument;

use crate::ProviderError;
use agena_domain::{
    AdapterId, Model, ModelId, ModelRef, ModelSpeedMode, ModelThinkingMode, ProviderId,
};
use agena_plugin_host::ProviderDescriptor;
use agena_provider::ProviderErrorKind;
use agena_runtime_contracts::part::{AttachmentItem, AttachmentKind};

use super::{CompletionResponse, ModelRuntime, wire_message};
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;

/// How many times a retryable provider request is retried before the request
/// fails. 10 mirrors codex's `stream_max_retries` default and is deliberately
/// generous: transient network failures (timeout, connect, 429/5xx) are the
/// most common reason a reply would otherwise be interrupted, and the circuit
/// breaker below keeps us from hammering a provider that is actually down.
const REQUEST_MAX_RETRIES: u32 = 10;
const RETRY_BASE_DELAY_MS: u64 = 250;
const RETRY_MAX_DELAY_MS: u64 = 2_000;
const STREAM_REPLAY_MAX_RETRIES_AFTER_OUTPUT: u32 = 5;
const STREAM_REPLAY_MAX_TRACKED_EVENTS: usize = 2048;

/// Circuit-breaker trip threshold: after this many consecutive request-level
/// failures for one provider, the breaker opens and new requests fail fast
/// instead of burning the full retry budget against a provider that is down.
const BREAKER_TRIP_THRESHOLD: u32 = 3;
/// How long the circuit stays open before the next request is allowed to try
/// again (half-open probe).
const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);

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
    let catalog_model_id = agena_provider::normalized_catalog_model_id(model_id.as_ref());
    (!catalog_model_id.is_empty()).then(|| ModelId::new(catalog_model_id))
}

fn provider_not_found_error(provider_id: &str) -> ProviderError {
    ProviderError::Config(format!("provider not found: {provider_id}"))
}

fn hydrate_model_from_provider(
    provider: &dyn ModelRuntime,
    model: &mut Model,
    adapter_id: Option<&AdapterId>,
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
    model.native_compaction =
        provider.native_compaction_enabled_for_adapter(adapter_id.as_ref(), &model.id);

    let thinking_modes = provider.model_thinking_modes_for_adapter(adapter_id.as_ref(), &model.id);
    let speed_modes = provider.model_speed_modes_for_adapter(adapter_id.as_ref(), &model.id);
    if model.thinking_modes.is_empty() {
        model.thinking_modes = thinking_modes;
    }
    if model.speed_modes.is_empty() {
        model.speed_modes = speed_modes;
    }
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
    hydrate_model_from_provider(provider, model, None);
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
    usage.requests = usage.requests.max(1);
    if usage.recorded_cost_available || usage.total_cost > 0.0 || usage.estimated_cost > 0.0 {
        return;
    }
    let metadata = provider.model_metadata_for_adapter(model.adapter_id.as_ref(), &model.model_id);
    let built_in = agena_provider::estimate_completion_usage_cost_usd(
        provider.id(),
        model.model_id.as_ref(),
        usage,
    );
    // Model metadata remains authoritative for ordinary cache pricing, but its
    // single cache-write rate cannot represent Anthropic's distinct 5m/1h
    // write multipliers. Prefer the official built-in snapshot when 1h writes
    // are present, then fall back to configured metadata.
    let metadata_estimate = estimate_total_cost_from_metadata(&metadata, usage);
    let estimated = if usage.cache_write_1h_tokens > 0 {
        built_in.or(metadata_estimate)
    } else {
        metadata_estimate.or(built_in)
    };
    if let Some(estimated) = estimated {
        usage.estimated_cost = estimated;
        if usage.cache_write_1h_tokens > 0 && built_in.is_none() {
            usage.cost_estimate_incomplete = true;
        }
    } else if usage.has_own_usage() {
        usage.cost_estimate_incomplete = true;
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

/// Registry of model providers.
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn ModelRuntime>>,
    retry_policy: RequestRetryPolicy,
    stream_replay_policy: StreamReplayPolicy,
    breaker: Arc<Mutex<HashMap<String, BreakerState>>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BreakerState {
    consecutive_failures: u32,
    /// When the circuit is open (Some), requests fail fast until this instant.
    open_until: Option<Instant>,
}

/// Whether the circuit for this provider is currently open. Lock failure is
/// treated as closed (fail-open is safer than suppressing traffic on a lock
/// hiccup).
fn breaker_open(breaker: &Arc<Mutex<HashMap<String, BreakerState>>>, provider_id: &str) -> bool {
    let Ok(breaker) = breaker.lock() else {
        return false;
    };
    let Some(state) = breaker.get(provider_id) else {
        return false;
    };
    state
        .open_until
        .is_some_and(|open_until| Instant::now() < open_until)
}

/// Record a request-level failure for a provider. Trips the circuit once the
/// consecutive-failure threshold is reached; a later success resets.
fn breaker_record_failure(breaker: &Arc<Mutex<HashMap<String, BreakerState>>>, provider_id: &str) {
    let Ok(mut breaker) = breaker.lock() else {
        return;
    };
    let state = breaker.entry(provider_id.to_owned()).or_default();
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    if state.consecutive_failures >= BREAKER_TRIP_THRESHOLD {
        state.open_until = Some(Instant::now() + BREAKER_COOLDOWN);
        tracing::warn!(
            provider_id,
            consecutive_failures = state.consecutive_failures,
            cooldown_secs = BREAKER_COOLDOWN.as_secs(),
            "provider circuit breaker opened after consecutive request failures"
        );
    }
}

/// Record a successful request for a provider; closes an open circuit and
/// resets the consecutive-failure counter.
fn breaker_record_success(breaker: &Arc<Mutex<HashMap<String, BreakerState>>>, provider_id: &str) {
    let Ok(mut breaker) = breaker.lock() else {
        return;
    };
    let state = breaker.entry(provider_id.to_owned()).or_default();
    if state.open_until.is_some() {
        tracing::info!(
            provider_id,
            "provider circuit breaker closed after a successful request"
        );
    }
    *state = BreakerState::default();
}

struct PluginRegisteredProvider {
    id: String,
    display_name: String,
    default_model: ModelId,
    models: Vec<ModelId>,
}

#[async_trait]
impl ModelRuntime for PluginRegisteredProvider {
    fn id(&self) -> &str {
        self.id.as_ref()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(self
            .models
            .iter()
            .map(|model| Model {
                provider_id: ProviderId::new(self.id.as_str()),
                adapter_id: None,
                id: ModelId::new(model.as_ref()),
                catalog_model_id: None,
                display_name: Some(self.display_name.clone()),
                native_compaction: true,
                capabilities: ModelCapabilities::default(),
                metadata: ModelMetadata::default(),
                thinking_modes: Vec::new(),
                speed_modes: std::collections::BTreeMap::new(),
            })
            .collect())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Err(ProviderError::Provider(format!(
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
            breaker: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Whether the circuit for this provider is currently open: recent
    /// consecutive request-level failures tripped it, so new requests should
    /// fail fast instead of burning the full retry budget.
    fn breaker_open(&self, provider_id: &str) -> bool {
        breaker_open(&self.breaker, provider_id)
    }

    /// Record a request-level failure for a provider. Trips the circuit once
    /// the consecutive-failure threshold is reached; a later success resets.
    fn breaker_record_failure(&self, provider_id: &str) {
        breaker_record_failure(&self.breaker, provider_id)
    }

    /// Record a successful request for a provider; closes an open circuit and
    /// resets the consecutive-failure counter.
    fn breaker_record_success(&self, provider_id: &str) {
        breaker_record_success(&self.breaker, provider_id)
    }

    /// Circuit-aware retry decision: an open circuit suppresses retries so the
    /// caller surfaces the underlying error promptly (and can fall back to an
    /// alternate model) instead of hammering a provider that is down.
    fn should_retry_error(&self, provider_id: &str, err: &ProviderError, retry_index: u32) -> bool {
        if self.breaker_open(provider_id) {
            return false;
        }
        err.retryable() && retry_index < self.retry_policy.max_retries
    }

    pub fn build_http_client(
        config: agena_provider::ProviderHttpClientConfig,
    ) -> Result<reqwest::Client, ProviderError> {
        if config.timeout.is_zero() {
            return Err(ProviderError::Config(
                "provider http timeout must be greater than 0".to_owned(),
            ));
        }
        if config.connect_timeout.is_zero() {
            return Err(ProviderError::Config(
                "provider connect timeout must be greater than 0".to_owned(),
            ));
        }
        reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(ProviderError::from)
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
    ) -> Result<(), ProviderError> {
        let id = descriptor.id.trim();
        if id.is_empty() {
            return Err(ProviderError::Config(
                "plugin provider id cannot be empty".to_owned(),
            ));
        }
        let models = descriptor
            .models
            .iter()
            .map(|model| ModelId::try_new(model.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                ProviderError::Config(format!("invalid plugin provider model: {err}"))
            })?;
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
    ) -> Result<Arc<dyn ModelRuntime>, ProviderError> {
        self.get(provider_id)
            .ok_or_else(|| provider_not_found_error(provider_id))
    }

    pub(super) fn provider_for_model_ref(
        &self,
        model: &ModelRef,
    ) -> Result<Arc<dyn ModelRuntime>, ProviderError> {
        self.require_provider(model.provider_id.as_ref())
    }

    pub(super) fn use_model_ref_provider<T>(
        &self,
        model: &ModelRef,
        map: impl FnOnce(&dyn ModelRuntime, Option<&AdapterId>, &ModelId) -> T,
    ) -> Result<T, ProviderError> {
        let provider = self.provider_for_model_ref(model)?;
        Ok(map(
            provider.as_ref(),
            model.adapter_id.as_ref(),
            &model.model_id,
        ))
    }

    async fn call_with_retry<T, F, Fut>(
        &self,
        provider_id: &str,
        operation: &str,
        mut op: F,
    ) -> Result<T, ProviderError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, ProviderError>>,
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
                    self.breaker_record_success(provider_id);
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
                    if !self.should_retry_error(provider_id, &err, retry_index) {
                        self.breaker_record_failure(provider_id);
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
) -> Result<(), ProviderError> {
    let capabilities =
        provider.model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.model_id);

    let mut unsupported = Vec::new();
    for run in &request.turns {
        for part in wire_message::project(run) {
            let wire_message::WirePart::Attachment { item } = part else {
                continue;
            };

            if let Some(modality) = unsupported_attachment_modality(&capabilities, &item) {
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
        return Err(ProviderError::Provider(format!(
            "provider `{}` model `{}` explicitly does not support requested input modalities: {details}",
            model.provider_id, model.model_id
        )));
    }

    provider.validate_provider_native_tools_request(model.adapter_id.as_ref(), request)
}

fn unsupported_attachment_modality(
    capabilities: &ModelCapabilities,
    attachment: &AttachmentItem,
) -> Option<ModelInputModality> {
    let required = match attachment.kind {
        AttachmentKind::Image => ModelInputModality::Image,
        AttachmentKind::Pdf => ModelInputModality::Document,
        AttachmentKind::Audio => ModelInputModality::Audio,
        AttachmentKind::Video => ModelInputModality::Video,
        AttachmentKind::File => {
            let mime = attachment.mime.trim().to_ascii_lowercase();
            let text_like = mime.starts_with("text/")
                || matches!(
                    mime.as_str(),
                    "application/json"
                        | "application/xml"
                        | "application/yaml"
                        | "application/x-yaml"
                        | "application/javascript"
                );
            (!text_like).then_some(ModelInputModality::File)?
        }
    };
    capabilities
        .support_for_input_modality(required)
        .is_unsupported()
        .then_some(required)
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

fn retry_reason(err: &ProviderError) -> &'static str {
    match err {
        ProviderError::Http(inner) if inner.is_timeout() => "http_timeout",
        ProviderError::Http(inner) if inner.is_connect() => "http_connect",
        ProviderError::Http(inner) if inner.is_body() => "http_body",
        ProviderError::Http(inner) if inner.is_decode() => "http_decode",
        ProviderError::HttpStatus {
            kind, retryable, ..
        }
        | ProviderError::ProviderClassified {
            kind, retryable, ..
        } if *retryable => provider_error_kind_label(*kind),
        ProviderError::Config(_) => "config_error",
        ProviderError::Provider(_) => "provider_error",
        ProviderError::Database(_) => "database_error",
        ProviderError::SerdeJson(_) => "serde_json_error",
        ProviderError::Http(_) => "http_error",
        ProviderError::Io(_) => "io_error",
        ProviderError::Internal(_) => "internal_error",
        _ => "non_retryable",
    }
}

fn provider_error_kind_label(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Authentication => "authentication",
        ProviderErrorKind::RateLimited => "rate_limited",
        ProviderErrorKind::QuotaExceeded => "quota_exceeded",
        ProviderErrorKind::ContextOverflow => "context_overflow",
        ProviderErrorKind::InvalidRequest => "invalid_request",
        ProviderErrorKind::Unavailable => "unavailable",
        ProviderErrorKind::Timeout => "timeout",
        ProviderErrorKind::Connection => "connection",
        ProviderErrorKind::MalformedResponse => "malformed_response",
        ProviderErrorKind::ToolProtocolViolation => "tool_protocol_violation",
        ProviderErrorKind::Misconfiguration => "misconfiguration",
        ProviderErrorKind::Internal => "internal",
    }
}

#[cfg(test)]
mod tests {
    use super::estimate_total_cost_from_metadata;
    use agena_domain::{ModelMetadata, ModelPricing};
    use agena_provider::CompletionUsage;

    #[test]
    fn breaker_trips_after_consecutive_failures_and_closes_on_success() {
        let registry = super::ProviderRegistry::new();
        assert!(!registry.breaker_open("p1"));

        registry.breaker_record_failure("p1");
        assert!(!registry.breaker_open("p1"));
        registry.breaker_record_failure("p1");
        assert!(!registry.breaker_open("p1"));
        registry.breaker_record_failure("p1");
        assert!(registry.breaker_open("p1"));

        // Failures are tracked per provider: another provider does not trip
        // this circuit and does not reset the open one.
        registry.breaker_record_failure("p2");
        registry.breaker_record_failure("p2");
        assert!(!registry.breaker_open("p2"));
        assert!(registry.breaker_open("p1"));
        registry.breaker_record_failure("p2");
        assert!(registry.breaker_open("p2"));
        assert!(registry.breaker_open("p1"));

        // A success closes the circuit and resets the counter.
        registry.breaker_record_success("p1");
        assert!(!registry.breaker_open("p1"));
        assert!(registry.breaker_open("p2"));
        registry.breaker_record_success("p2");
        assert!(!registry.breaker_open("p2"));
    }

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
            requests: 1,
            input_tokens: 0,
            output_tokens: 50,
            reasoning_tokens: 30,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            total_cost: 0.0,
            ..CompletionUsage::default()
        };

        let estimated =
            estimate_total_cost_from_metadata(&metadata, &usage).expect("estimated cost");
        assert!((estimated - 0.000_080).abs() < f64::EPSILON);
    }

    #[test]
    fn retry_reason_labels_malformed_response_as_retryable() {
        let err = crate::ProviderError::ProviderClassified {
            provider: "p1".to_owned(),
            message: "invalid SSE payload".to_owned(),
            kind: agena_provider::ProviderErrorKind::MalformedResponse,
            retryable: true,
        };
        assert_eq!(super::retry_reason(&err), "malformed_response");
        assert!(err.retryable());
    }
}
