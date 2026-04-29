use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;

use crate::error::{AppError, ProviderErrorKind};
use crate::model::{Model, ModelCapabilities, ModelId, ModelMetadata, ModelRef, ProviderId};

use super::{
    CapabilityOverrideProvider, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    ModelProvider, ProviderCapabilityOverrideRule, ProviderHttpClientConfig,
    ProviderRequestRetryConfig, ProviderRuntimeConfig, ProviderStreamReplayConfig,
    StreamResumePolicy, utils,
};

#[derive(Debug, Clone)]
pub struct ProviderAliasRegistration {
    pub alias_id: String,
    pub target_provider_id: String,
    pub default_model: Option<ModelId>,
    pub capability_overrides: Vec<ProviderCapabilityOverrideRule>,
}

impl ProviderAliasRegistration {
    pub fn new(alias_id: impl Into<String>, target_provider_id: impl Into<String>) -> Self {
        Self {
            alias_id: alias_id.into(),
            target_provider_id: target_provider_id.into(),
            default_model: None,
            capability_overrides: Vec::new(),
        }
    }

    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(ModelId::new(model));
        self
    }

    pub fn with_capability_overrides(
        mut self,
        capability_overrides: Vec<ProviderCapabilityOverrideRule>,
    ) -> Self {
        self.capability_overrides = capability_overrides;
        self
    }
}

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

#[derive(Clone)]
pub struct NamedProvider {
    provider_id: String,
    target: Arc<dyn ModelProvider>,
    default_model: Option<ModelId>,
}

impl NamedProvider {
    pub fn new(provider_id: impl Into<String>, target: Arc<dyn ModelProvider>) -> Self {
        Self {
            provider_id: provider_id.into(),
            target,
            default_model: None,
        }
    }

    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(ModelId::new(model));
        self
    }
}

#[async_trait]
impl ModelProvider for NamedProvider {
    fn id(&self) -> &str {
        self.provider_id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        self.default_model
            .as_ref()
            .unwrap_or_else(|| self.target.default_model())
    }

    fn model_capabilities(&self, model: &ModelId) -> super::ModelCapabilities {
        self.target.model_capabilities(model)
    }

    fn model_metadata(&self, model: &ModelId) -> super::ModelMetadata {
        self.target.model_metadata(model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.target.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<super::PromptCacheShape> {
        self.target.prompt_cache_shape(model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        for model in &mut models {
            model.provider_id = ProviderId::new(self.provider_id.clone());
            let fallback = self.target.model_capabilities(&model.id);
            model.capabilities = model.capabilities.clone().with_fallbacks_from(&fallback);
            let metadata_fallback = self.target.model_metadata(&model.id);
            model.metadata = model
                .metadata
                .clone()
                .with_fallbacks_from(&metadata_fallback);
        }
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let mut response = self.target.complete(request).await?;
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
        let stream = self.target.complete_stream(request).await?;
        let alias_id = self.provider_id.clone();
        let mapped = stream
            .map(move |item| item.map(|event| remap_event_provider_id(event, alias_id.as_str())));
        Ok(Box::pin(mapped))
    }
}

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

    pub fn register_alias(&mut self, alias: ProviderAliasRegistration) -> Result<(), AppError> {
        let alias_id = alias.alias_id.trim();
        if alias_id.is_empty() {
            return Err(AppError::Config(
                "provider alias id cannot be empty".to_owned(),
            ));
        }

        let target_provider_id = alias.target_provider_id.trim();
        let target = self.get(target_provider_id).ok_or_else(|| {
            AppError::Config(format!(
                "provider alias target not found: {target_provider_id}"
            ))
        })?;

        let default_model = alias.default_model;

        let aliased = match default_model {
            Some(model) => {
                NamedProvider::new(alias_id.to_owned(), target).with_default_model(model)
            }
            None => NamedProvider::new(alias_id.to_owned(), target),
        };

        let provider =
            CapabilityOverrideProvider::new(Arc::new(aliased), alias.capability_overrides);

        self.providers.insert(alias_id.to_owned(), provider);
        Ok(())
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ModelProvider>> {
        self.providers.get(provider_id).cloned()
    }

    pub fn supports_prompt_continuation(&self, model: &ModelRef) -> Result<bool, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.supports_prompt_continuation(&model.model_id))
    }

    pub fn prompt_cache_shape_fingerprint(
        &self,
        model: &ModelRef,
    ) -> Result<Option<String>, AppError> {
        Ok(self
            .prompt_cache_shape(model)?
            .map(|shape| shape.fingerprint()))
    }

    pub fn prompt_cache_shape(
        &self,
        model: &ModelRef,
    ) -> Result<Option<super::PromptCacheShape>, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.prompt_cache_shape(&model.model_id))
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(AppError::Config(
                "provider or model reference cannot be empty".to_owned(),
            ));
        }

        let requested_model = model.map(str::trim).filter(|value| !value.is_empty());
        if target.contains('/') {
            if requested_model.is_some() {
                return Err(AppError::Config(format!(
                    "model reference `{target}` already includes a model; omit `--model`"
                )));
            }
            return target.parse::<ModelRef>().map_err(|err| {
                AppError::Config(format!("invalid model reference `{target}`: {err}"))
            });
        }

        let provider = self
            .get(target)
            .ok_or_else(|| AppError::Config(format!("provider not found: {target}")))?;
        let provider_id = ProviderId::try_new(target)
            .map_err(|err| AppError::Config(format!("invalid provider id `{target}`: {err}")))?;
        let model_id = match requested_model {
            Some(requested_model) => ModelId::try_new(requested_model).map_err(|err| {
                AppError::Config(format!("invalid model id `{requested_model}`: {err}"))
            })?,
            None => provider.default_model().clone(),
        };

        Ok(ModelRef {
            provider_id,
            model_id,
        })
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

            match op().await {
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

    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<Model>, AppError> {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        let provider_id = provider_id.to_owned();
        self.call_with_retry(provider_id.as_str(), "list_models", {
            let provider = provider.clone();
            let provider_id = provider_id.clone();
            move || {
                let provider = provider.clone();
                let provider_id = provider_id.clone();
                async move {
                    let mut models = provider.list_models().await?;
                    for model in &mut models {
                        model.provider_id = ProviderId::new(provider_id.clone());
                        let fallback = provider.model_capabilities(&model.id);
                        model.capabilities =
                            model.capabilities.clone().with_fallbacks_from(&fallback);
                        let metadata_fallback = provider.model_metadata(&model.id);
                        model.metadata = model
                            .metadata
                            .clone()
                            .with_fallbacks_from(&metadata_fallback);
                    }
                    Ok(models)
                }
            }
        })
        .await
    }

    pub fn model_capabilities(&self, model: &ModelRef) -> Result<ModelCapabilities, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_capabilities(&model.model_id))
    }

    pub fn model_metadata(&self, model: &ModelRef) -> Result<ModelMetadata, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_metadata(&model.model_id))
    }

    pub async fn resolve_model(&self, model: &ModelRef) -> Result<Model, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;

        let listed = self.list_models(model.provider_id.as_str()).await?;
        if let Some(entry) = listed.into_iter().find(|entry| entry.id == model.model_id) {
            let fallback = provider.model_capabilities(&entry.id);
            let metadata_fallback = provider.model_metadata(&entry.id);
            return Ok(entry
                .with_capability_fallbacks(&fallback)
                .with_metadata_fallbacks(&metadata_fallback));
        }

        Ok(
            Model::new(model.provider_id.as_str(), model.model_id.as_str())
                .with_capabilities(provider.model_capabilities(&model.model_id))
                .with_metadata(provider.model_metadata(&model.model_id)),
        )
    }

    pub async fn complete(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
        self.call_with_retry(model.provider_id.as_str(), "complete", {
            let provider = provider.clone();
            let request = request.clone();
            move || {
                let provider = provider.clone();
                let request = request.clone();
                async move { provider.complete(request).await }
            }
        })
        .await
    }

    pub async fn complete_stream(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
        let provider_id = model.provider_id.to_string();
        let retry_policy = self.retry_policy;
        let replay_policy = self.stream_replay_policy;
        let provider_resume_policy = provider.stream_resume_policy();
        let replay_safe_enabled = replay_policy.enabled(provider_resume_policy);

        let stream = async_stream::try_stream! {
            let mut retry_index = 0_u32;
            let mut replay_retry_index = 0_u32;
            let mut emitted_history: Vec<CompletionStreamEvent> = Vec::new();
            let mut replay_buffer_exhausted = false;

            loop {
                let attempt = retry_index + 1;
                let attempt_started_at = Instant::now();
                let replay_mode_enabled = replay_safe_enabled && !emitted_history.is_empty();
                tracing::info!(
                    provider_id = provider_id.as_str(),
                    operation = "complete_stream",
                    attempt,
                    retries = retry_index,
                    status = "attempt_started",
                    resume_policy = stream_resume_policy_label(provider_resume_policy),
                    replay_mode = replay_mode_enabled,
                    tracked_events = emitted_history.len() as u64,
                    "provider stream attempt started"
                );

                let mut inner_stream = match provider.complete_stream(request.clone()).await {
                    Ok(stream) => {
                        tracing::debug!(
                            provider_id = provider_id.as_str(),
                            operation = "complete_stream",
                            attempt,
                            retries = retry_index,
                            latency_ms = elapsed_ms(attempt_started_at),
                            status = "startup_ok",
                            resume_policy = stream_resume_policy_label(provider_resume_policy),
                            "provider stream startup succeeded"
                        );
                        stream
                    }
                    Err(err) => {
                        let can_retry = err.retryable() && retry_index < retry_policy.max_retries;
                        let reason = retry_reason(&err);
                        if can_retry {
                            let delay = retry_policy.delay_for_retry(retry_index);
                            tracing::warn!(
                                provider_id = provider_id.as_str(),
                                operation = "complete_stream",
                                attempt,
                                retries = retry_index,
                                stage = "startup",
                                max_retries = retry_policy.max_retries,
                                latency_ms = elapsed_ms(attempt_started_at),
                                status = "retry_scheduled",
                                retry_reason = reason,
                                delay_ms = delay.as_millis() as u64,
                                error = %err,
                                "provider stream startup failed with retryable error; scheduling retry"
                            );
                            tokio::time::sleep(delay).await;
                            retry_index += 1;
                            continue;
                        }

                        tracing::error!(
                            provider_id = provider_id.as_str(),
                            operation = "complete_stream",
                            attempt,
                            retries = retry_index,
                            stage = "startup",
                            latency_ms = elapsed_ms(attempt_started_at),
                            status = "failed",
                            retry_reason = reason,
                            error = %err,
                            "provider stream startup failed"
                        );

                        Err(err)?;
                        continue;
                    }
                };

                let mut emitted_event_in_attempt = false;
                let mut should_restart_stream = false;
                let mut replay_cursor = 0_usize;
                let mut replay_mode = replay_mode_enabled;
                let mut emitted_events_in_attempt = 0_u64;
                let mut replayed_events_in_attempt = 0_u64;

                while let Some(item) = inner_stream.next().await {
                    match item {
                        Ok(event) => {
                            if replay_mode && replay_cursor < emitted_history.len() {
                                if event == emitted_history[replay_cursor] {
                                    replay_cursor += 1;
                                    replayed_events_in_attempt += 1;
                                    if replay_cursor == emitted_history.len() {
                                        replay_mode = false;
                                        tracing::debug!(
                                            provider_id = provider_id.as_str(),
                                            operation = "complete_stream",
                                            attempt,
                                            status = "replay_prefix_aligned",
                                            replayed_events = replayed_events_in_attempt,
                                            "provider stream replay prefix aligned"
                                        );
                                    }
                                    continue;
                                }

                                let err = AppError::Provider(format!(
                                    "provider stream replay prefix diverged at event index {replay_cursor}"
                                ));
                                tracing::error!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "replay_prefix",
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "failed",
                                    retry_reason = "replay_prefix_diverged",
                                    replayed_events = replayed_events_in_attempt,
                                    "provider stream replay prefix diverged; aborting to avoid duplicate output"
                                );
                                Err(err)?;
                            }

                            replay_mode = false;
                            emitted_event_in_attempt = true;
                            emitted_events_in_attempt += 1;

                            if replay_safe_enabled && !replay_buffer_exhausted {
                                if emitted_history.len() < replay_policy.max_tracked_events {
                                    emitted_history.push(event.clone());
                                } else {
                                    replay_buffer_exhausted = true;
                                    tracing::warn!(
                                        provider_id = provider_id.as_str(),
                                        operation = "complete_stream",
                                        attempt,
                                        status = "replay_buffer_exhausted",
                                        tracked_events = emitted_history.len() as u64,
                                        max_tracked_events = replay_policy.max_tracked_events as u64,
                                        "provider stream replay buffer exhausted; disabling post-output replay-safe restart"
                                    );
                                }
                            }

                            yield event;
                        }
                        Err(err) => {
                            let can_retry_now = err.retryable() && retry_index < retry_policy.max_retries;
                            let can_retry_early_stream_error = !emitted_event_in_attempt
                                && can_retry_now;

                            let can_retry_after_output = emitted_event_in_attempt
                                && can_retry_now
                                && replay_safe_enabled
                                && !replay_buffer_exhausted
                                && replay_retry_index < replay_policy.max_retries_after_output;

                            let reason = retry_reason(&err);

                            if can_retry_early_stream_error {
                                let delay = retry_policy.delay_for_retry(retry_index);
                                tracing::warn!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "before_first_event",
                                    max_retries = retry_policy.max_retries,
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "retry_scheduled",
                                    retry_reason = reason,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %err,
                                    "provider stream failed before first event with retryable error; restarting stream"
                                );
                                tokio::time::sleep(delay).await;
                                retry_index += 1;
                                should_restart_stream = true;
                                break;
                            }

                            if can_retry_after_output {
                                let delay = retry_policy.delay_for_retry(retry_index);
                                tracing::warn!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "after_output",
                                    max_retries = retry_policy.max_retries,
                                    replay_restarts = replay_retry_index,
                                    max_replay_restarts = replay_policy.max_retries_after_output,
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "replay_restart_scheduled",
                                    retry_reason = reason,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %err,
                                    "provider stream failed after output with replay-safe provider; scheduling replay-aware restart"
                                );
                                tokio::time::sleep(delay).await;
                                retry_index += 1;
                                replay_retry_index += 1;
                                should_restart_stream = true;
                                break;
                            }

                            tracing::error!(
                                provider_id = provider_id.as_str(),
                                operation = "complete_stream",
                                attempt,
                                retries = retry_index,
                                stage = if emitted_event_in_attempt { "after_output" } else { "before_first_event" },
                                latency_ms = elapsed_ms(attempt_started_at),
                                status = "failed",
                                retry_reason = reason,
                                replay_restarts = replay_retry_index,
                                error = %err,
                                "provider stream failed"
                            );

                            Err(err)?;
                        }
                    }
                }

                if replay_mode && replay_cursor < emitted_history.len() {
                    let err = AppError::Provider(
                        "provider stream replay ended before replay prefix alignment completed"
                            .to_owned(),
                    );
                    tracing::error!(
                        provider_id = provider_id.as_str(),
                        operation = "complete_stream",
                        attempt,
                        retries = retry_index,
                        stage = "replay_prefix",
                        latency_ms = elapsed_ms(attempt_started_at),
                        status = "failed",
                        retry_reason = "replay_prefix_incomplete",
                        replayed_events = replayed_events_in_attempt,
                        expected_events = emitted_history.len() as u64,
                        "provider stream replay ended before matching emitted prefix"
                    );
                    Err(err)?;
                }

                if should_restart_stream {
                    continue;
                }

                tracing::info!(
                    provider_id = provider_id.as_str(),
                    operation = "complete_stream",
                    attempt,
                    retries = retry_index,
                    replay_restarts = replay_retry_index,
                    latency_ms = elapsed_ms(attempt_started_at),
                    status = "completed",
                    emitted_events = emitted_events_in_attempt,
                    replayed_events = replayed_events_in_attempt,
                    "provider stream attempt completed"
                );

                break;
            }
        };

        Ok(Box::pin(stream))
    }
}

fn remap_event_provider_id(
    event: CompletionStreamEvent,
    provider_id: &str,
) -> CompletionStreamEvent {
    let provider_id = ProviderId::new(provider_id);
    match event {
        CompletionStreamEvent::TextDelta { model, delta, .. } => CompletionStreamEvent::TextDelta {
            provider_id: provider_id.clone(),
            model,
            delta,
        },
        CompletionStreamEvent::ToolCallDelta {
            model,
            stream_key,
            id,
            name,
            arguments_delta,
            ..
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
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
            provider_id,
            model,
            finish_reason,
            usage,
            provider_metadata,
        },
        CompletionStreamEvent::ThinkingDelta { model, delta, .. } => {
            CompletionStreamEvent::ThinkingDelta {
                provider_id,
                model,
                delta,
            }
        }
    }
}

fn validate_request_capabilities(
    model: &ModelRef,
    provider: &dyn ModelProvider,
    request: &CompletionRequest,
) -> Result<(), AppError> {
    let capabilities = provider.model_capabilities(&model.model_id);

    let mut unsupported = Vec::new();
    for message in &request.messages {
        for part in utils::project_session_parts(message) {
            let utils::ProjectedSessionPart::Attachment { item } = part else {
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
mod tests {
    use super::*;
    use crate::message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent};
    use crate::model::{ModelId, ModelRef, ProviderId};
    use crate::provider::{
        CapabilityOverrideMatchMode, CapabilitySupport, CompletionFinishReason, CompletionRequest,
        CompletionResponse, ModelCapabilities, ModelCapabilityPatch,
        ProviderCapabilityOverrideRule, ProviderModel,
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

            response_format: None,
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for FlakyProvider {
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
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
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
    impl ModelProvider for FlakyStreamProvider {
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
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
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

    struct UnsupportedImageProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for UnsupportedImageProvider {
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

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
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
        assert!(
            matches!(err, AppError::Config(message) if message.contains("must be greater than 0"))
        );
    }

    #[test]
    fn provider_http_client_config_rejects_zero_connect_timeout() {
        let err = ProviderHttpClientConfig {
            timeout: Duration::from_secs(1),
            connect_timeout: Duration::ZERO,
        }
        .build_client()
        .expect_err("zero connect timeout should be rejected");
        assert!(
            matches!(err, AppError::Config(message) if message.contains("must be greater than 0"))
        );
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

    #[test]
    fn register_alias_requires_existing_target_provider() {
        let mut registry = ProviderRegistry::new();
        let err = registry
            .register_alias(ProviderAliasRegistration::new(
                "alias-provider",
                "missing-provider",
            ))
            .expect_err("alias registration should fail when target is missing");
        assert!(matches!(err, AppError::Config(message) if message.contains("target not found")));
    }

    #[test]
    fn resolve_model_target_parses_explicit_model_reference() {
        let registry = ProviderRegistry::new();
        let resolved = registry
            .resolve_model_target("openai/gpt-5", None)
            .expect("model reference should parse");
        assert_eq!(resolved, model_ref("openai", "gpt-5"));
    }

    #[test]
    fn resolve_model_target_uses_provider_default_model() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let provider = FlakyProvider {
            provider_id: "target-default",
            attempts,
            fail_attempts: 0,
            retryable: false,
        };
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        registry
            .register_alias(
                ProviderAliasRegistration::new("target-alias", "target-default")
                    .with_default_model("alias-model"),
            )
            .expect("alias registration should succeed");

        let resolved = registry
            .resolve_model_target("target-alias", None)
            .expect("target should resolve using alias default");
        assert_eq!(resolved, model_ref("target-alias", "alias-model"));
    }

    #[tokio::test]
    async fn alias_provider_remaps_complete_and_stream_ids() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let provider = FlakyProvider {
            provider_id: "internal-provider",
            attempts,
            fail_attempts: 0,
            retryable: false,
        };
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        registry
            .register_alias(
                ProviderAliasRegistration::new("custom-provider", "internal-provider")
                    .with_default_model("alias-model"),
            )
            .expect("alias registration should succeed");
        let response = registry
            .complete(
                &model_ref("custom-provider", "alias-model"),
                completion_request("alias-model"),
            )
            .await
            .expect("alias completion should succeed");
        assert_eq!(response.provider_id, pid("custom-provider"));
        assert_eq!(response.model, mid("alias-model"));
        let mut stream = registry
            .complete_stream(
                &model_ref("custom-provider", "alias-model"),
                completion_request("alias-model"),
            )
            .await
            .expect("alias stream should start");
        let first = stream
            .next()
            .await
            .expect("first stream event should exist")
            .expect("first stream event should be success");
        assert!(
            matches!(first, CompletionStreamEvent::TextDelta { ref provider_id, .. } if provider_id == &pid("custom-provider"))
        );
        let second = stream
            .next()
            .await
            .expect("second stream event should exist")
            .expect("second stream event should be success");
        assert!(
            matches!(second, CompletionStreamEvent::Completed { ref provider_id, .. } if provider_id == &pid("custom-provider"))
        );
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

    #[tokio::test]
    async fn resolve_model_for_alias_unlisted_model_preserves_alias_identity() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let provider = FlakyProvider {
            provider_id: "internal-model-info",
            attempts,
            fail_attempts: 0,
            retryable: false,
        };
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        registry
            .register_alias(
                ProviderAliasRegistration::new("alias-model-info", "internal-model-info")
                    .with_default_model("alias-model"),
            )
            .expect("alias registration should succeed");

        let model = registry
            .resolve_model(&model_ref("alias-model-info", "alias-model"))
            .await
            .expect("alias provider model should resolve");

        assert_eq!(model.provider_id, pid("alias-model-info"));
        assert_eq!(model.id, mid("alias-model"));
        assert_eq!(model.display_name, None);
        assert_eq!(
            model.capabilities.tool_calling,
            CapabilitySupport::Supported
        );
    }

    #[tokio::test]
    async fn alias_capability_overrides_change_exposed_model_capabilities() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let provider = FlakyProvider {
            provider_id: "override-target",
            attempts,
            fail_attempts: 0,
            retryable: false,
        };
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        registry
            .register_alias(
                ProviderAliasRegistration::new("override-alias", "override-target")
                    .with_default_model("alias-model")
                    .with_capability_overrides(vec![ProviderCapabilityOverrideRule {
                        model: "alias-model".to_owned(),
                        match_mode: CapabilityOverrideMatchMode::Exact,
                        capabilities: ModelCapabilityPatch {
                            streaming: Some(CapabilitySupport::Unsupported),
                            ..ModelCapabilityPatch::default()
                        },
                    }]),
            )
            .expect("alias registration should succeed");

        let model = registry
            .resolve_model(&model_ref("override-alias", "alias-model"))
            .await
            .expect("alias provider model should resolve");

        assert_eq!(model.provider_id, pid("override-alias"));
        assert_eq!(model.id, mid("alias-model"));
        assert_eq!(model.capabilities.streaming, CapabilitySupport::Unsupported);
        assert_eq!(
            model.capabilities.tool_calling,
            CapabilitySupport::Supported
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
    async fn alias_capability_overrides_participate_in_request_validation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = FlakyProvider {
            provider_id: "validation-target",
            attempts: Arc::clone(&calls),
            fail_attempts: 0,
            retryable: false,
        };
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        registry
            .register_alias(
                ProviderAliasRegistration::new("validation-alias", "validation-target")
                    .with_default_model("alias-model")
                    .with_capability_overrides(vec![ProviderCapabilityOverrideRule {
                        model: "alias-model".to_owned(),
                        match_mode: CapabilityOverrideMatchMode::Exact,
                        capabilities: ModelCapabilityPatch {
                            image_input: Some(CapabilitySupport::Unsupported),
                            ..ModelCapabilityPatch::default()
                        },
                    }]),
            )
            .expect("alias registration should succeed");

        let err = registry
            .complete(
                &model_ref("validation-alias", "alias-model"),
                CompletionRequest {
                    model: mid("alias-model"),
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
                    max_output_tokens: None,
                    temperature: None,
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                response_format: None,
                },
            )
            .await
            .expect_err("explicitly unsupported image input should be rejected");

        assert!(
            matches!(err, AppError::Provider(message) if message.contains("validation-alias") && message.contains("image"))
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
}
