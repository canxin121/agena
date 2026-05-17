use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{StreamExt, stream::BoxStream};

use crate::{
    error::AppError,
    model::{
        AdapterId, Model, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode, ProviderId,
    },
};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ConfiguredModelDefinition,
    ModelCapabilities, ModelProvider, PromptCacheShape, StreamResumePolicy,
};

#[derive(Debug, Clone)]
pub struct ProviderModelRoute {
    pub enabled: bool,
    pub definition: ConfiguredModelDefinition,
}

pub type ProviderModelRouteKey = (String, String);

#[derive(Clone)]
pub struct MultiAdapterProvider {
    id: String,
    default_adapter: AdapterId,
    default_model: ModelId,
    adapters: BTreeMap<String, Arc<dyn ModelProvider>>,
    routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
}

impl MultiAdapterProvider {
    pub fn new(
        id: impl Into<String>,
        default_adapter: impl Into<String>,
        default_model: impl Into<String>,
        adapters: BTreeMap<String, Arc<dyn ModelProvider>>,
        routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
    ) -> Self {
        Self {
            id: id.into(),
            default_adapter: AdapterId::new(default_adapter),
            default_model: ModelId::new(default_model),
            adapters,
            routes,
        }
    }

    fn adapter(&self, adapter_id: &str) -> Result<Arc<dyn ModelProvider>, AppError> {
        self.adapters.get(adapter_id).cloned().ok_or_else(|| {
            AppError::Config(format!(
                "provider `{}` has no enabled adapter `{adapter_id}`",
                self.id
            ))
        })
    }

    fn selected_adapter(&self, adapter_id: Option<&AdapterId>) -> AdapterId {
        adapter_id
            .cloned()
            .unwrap_or_else(|| self.default_adapter.clone())
    }

    fn resolve_route(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Result<(AdapterId, ModelId, ConfiguredModelDefinition), AppError> {
        let adapter_id = self.selected_adapter(adapter_id);
        let target_model = model.clone();
        let key = (adapter_id.to_string(), target_model.to_string());
        if let Some(route) = self.routes.get(&key) {
            if !route.enabled {
                return Err(AppError::Config(format!(
                    "provider `{}` adapter `{adapter_id}` model `{}` is disabled",
                    self.id, model
                )));
            }
            return Ok((adapter_id, target_model, route.definition.clone()));
        }

        self.adapter(adapter_id.as_str())?;
        Ok((
            adapter_id,
            target_model,
            ConfiguredModelDefinition::default(),
        ))
    }

    fn rewrite_model(
        &self,
        target_model: &ModelId,
        mut model: Model,
        adapter_id: &AdapterId,
        adapter: &dyn ModelProvider,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        model.provider_id = ProviderId::new(self.id.clone());
        model.adapter_id = Some(adapter_id.clone());
        model.id = target_model.clone();
        definition.apply_to_model(
            model,
            &adapter.model_capabilities(target_model),
            &adapter.model_metadata(target_model),
        )
    }

    fn synthesize_model(
        &self,
        target_model: &ModelId,
        adapter_id: &AdapterId,
        adapter: &dyn ModelProvider,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        definition.apply_to_model(
            Model::new(self.id.as_str(), target_model.as_str())
                .with_adapter_id(adapter_id.as_str()),
            &adapter.model_capabilities(target_model),
            &adapter.model_metadata(target_model),
        )
    }
}

#[async_trait]
impl ModelProvider for MultiAdapterProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn default_adapter(&self) -> Option<&AdapterId> {
        Some(&self.default_adapter)
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.model_capabilities_for_adapter(None, model)
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, definition)| {
                self.adapter(adapter_id.as_str()).ok().map(|adapter| {
                    definition
                        .capabilities
                        .apply_to(adapter.model_capabilities(&target_model))
                })
            })
            .unwrap_or_default()
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        self.model_metadata_for_adapter(None, model)
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, definition)| {
                self.adapter(adapter_id.as_str()).ok().map(|adapter| {
                    definition
                        .metadata()
                        .with_fallbacks_from(&adapter.model_metadata(&target_model))
                })
            })
            .unwrap_or_default()
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        self.model_thinking_modes_for_adapter(None, model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, definition)| {
                self.adapter(adapter_id.as_str()).ok().map(|adapter| {
                    let mut modes = adapter.model_thinking_modes(&target_model);
                    for (name, configured) in &definition.thinking_modes {
                        match configured.apply_to_mode(modes.get(name)) {
                            Some(mode) => {
                                modes.insert(name.clone(), mode);
                            }
                            None => {
                                modes.remove(name);
                            }
                        }
                    }
                    modes
                })
            })
            .unwrap_or_default()
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        self.model_speed_modes_for_adapter(None, model)
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, definition)| {
                self.adapter(adapter_id.as_str()).ok().map(|adapter| {
                    let mut modes = adapter.model_speed_modes(&target_model);
                    for (name, configured) in &definition.speed_modes {
                        match configured.apply_to_mode(modes.get(name)) {
                            Some(mode) => {
                                modes.insert(name.clone(), mode);
                            }
                            None => {
                                modes.remove(name);
                            }
                        }
                    }
                    modes
                })
            })
            .unwrap_or_default()
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        (self.adapters.len() == 1)
            .then(|| self.adapters.values().next().cloned())
            .flatten()
            .map(|adapter| adapter.stream_resume_policy())
            .unwrap_or(StreamResumePolicy::Disabled)
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.supports_prompt_continuation_for_adapter(None, model)
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, _)| {
                self.adapter(adapter_id.as_str())
                    .ok()
                    .map(|adapter| adapter.supports_prompt_continuation(&target_model))
            })
            .unwrap_or(false)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.prompt_cache_shape_for_adapter(None, model)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        self.resolve_route(adapter_id, model)
            .ok()
            .and_then(|(adapter_id, target_model, _)| {
                self.adapter(adapter_id.as_str())
                    .ok()
                    .and_then(|adapter| adapter.prompt_cache_shape(&target_model))
            })
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut visible = Vec::new();
        let mut seen = BTreeSet::new();
        let mut errors = Vec::new();

        for (adapter_id, adapter) in &self.adapters {
            let adapter_id = AdapterId::new(adapter_id.clone());
            let listed = match adapter.list_models().await {
                Ok(models) => models,
                Err(error) => {
                    errors.push(format!("{adapter_id}: {error}"));
                    Vec::new()
                }
            };
            for model in listed {
                let target_model = model.id.clone();
                let route_key = (adapter_id.to_string(), target_model.to_string());
                let route = self.routes.get(&route_key);
                if matches!(route, Some(route) if !route.enabled) {
                    continue;
                }
                let definition = route
                    .map(|route| route.definition.clone())
                    .unwrap_or_default();
                seen.insert(route_key);
                visible.push(self.rewrite_model(
                    &target_model,
                    model,
                    &adapter_id,
                    adapter.as_ref(),
                    &definition,
                ));
            }
        }

        for ((adapter_id, target_model), route) in &self.routes {
            if !route.enabled || seen.contains(&(adapter_id.clone(), target_model.clone())) {
                continue;
            }

            let adapter_id = AdapterId::new(adapter_id.clone());
            let target_model = ModelId::new(target_model.clone());
            let adapter = self.adapter(adapter_id.as_str())?;
            visible.push(self.synthesize_model(
                &target_model,
                &adapter_id,
                adapter.as_ref(),
                &route.definition,
            ));
        }

        if visible.is_empty() && !errors.is_empty() {
            return Err(AppError::Provider(format!(
                "adapter model discovery failed: {}",
                errors.join("; ")
            )));
        }

        Ok(visible)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.complete_for_adapter(None, request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let visible_model = request.model.clone();
        let (adapter_id, target_model, _) = self.resolve_route(adapter_id, &visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        let mut response = adapter.complete(request).await?;
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.complete_stream_for_adapter(None, request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let visible_model = request.model.clone();
        let (adapter_id, target_model, _) = self.resolve_route(adapter_id, &visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        let provider_id = self.id.clone();
        let stream = adapter.complete_stream(request).await?;
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
            Box::pin(stream.map(move |item| {
                item.map(|event| match event {
                    CompletionStreamEvent::TextDelta { delta, .. } => {
                        CompletionStreamEvent::TextDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model: visible_model.clone(),
                            delta,
                        }
                    }
                    CompletionStreamEvent::ThinkingDelta { delta, .. } => {
                        CompletionStreamEvent::ThinkingDelta {
                            provider_id: ProviderId::new(provider_id.clone()),
                            model: visible_model.clone(),
                            delta,
                        }
                    }
                    CompletionStreamEvent::ToolCallDelta {
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                        ..
                    } => CompletionStreamEvent::ToolCallDelta {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model: visible_model.clone(),
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                    },
                    CompletionStreamEvent::Completed {
                        finish_reason,
                        usage,
                        provider_metadata,
                        ..
                    } => CompletionStreamEvent::Completed {
                        provider_id: ProviderId::new(provider_id.clone()),
                        model: visible_model.clone(),
                        finish_reason,
                        usage,
                        provider_metadata,
                    },
                })
            }));
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CompletionFinishReason, CompletionResponse, ModelCapabilities};

    #[derive(Clone)]
    struct StaticProvider {
        id: String,
        default_model: ModelId,
        models: Vec<Model>,
        list_models_error: Option<String>,
    }

    #[async_trait]
    impl ModelProvider for StaticProvider {
        fn id(&self) -> &str {
            self.id.as_str()
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
            ModelCapabilities::default()
        }

        fn model_metadata(&self, _model: &ModelId) -> ModelMetadata {
            ModelMetadata::default()
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            if let Some(error) = &self.list_models_error {
                Err(AppError::Provider(error.clone()))
            } else {
                Ok(self.models.clone())
            }
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: ProviderId::new(self.id.clone()),
                model: request.model.clone(),
                text: format!("{}:{}", self.id, request.model),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }
    }

    #[tokio::test]
    async fn multi_adapter_provider_routes_explicit_models() {
        let provider = MultiAdapterProvider::new(
            "shared",
            "api",
            "gpt-4.1-mini",
            BTreeMap::from([
                (
                    "api".to_owned(),
                    Arc::new(StaticProvider {
                        id: "shared::api".to_owned(),
                        default_model: ModelId::new("gpt-4.1"),
                        models: vec![Model::new("shared::api", "gpt-4.1-mini")],
                        list_models_error: None,
                    }) as Arc<dyn ModelProvider>,
                ),
                (
                    "codex".to_owned(),
                    Arc::new(StaticProvider {
                        id: "shared::codex".to_owned(),
                        default_model: ModelId::new("gpt-5-codex"),
                        models: vec![Model::new("shared::codex", "gpt-5-codex")],
                        list_models_error: None,
                    }) as Arc<dyn ModelProvider>,
                ),
            ]),
            BTreeMap::from([
                (
                    ("api".to_owned(), "gpt-4.1-mini".to_owned()),
                    ProviderModelRoute {
                        enabled: true,
                        definition: ConfiguredModelDefinition::default(),
                    },
                ),
                (
                    ("codex".to_owned(), "gpt-5-codex".to_owned()),
                    ProviderModelRoute {
                        enabled: true,
                        definition: ConfiguredModelDefinition::default(),
                    },
                ),
            ]),
        );

        let models = provider.list_models().await.expect("models should list");
        let ids = models
            .iter()
            .map(|model| model.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert!(
            models
                .iter()
                .any(
                    |model| model.adapter_id.as_ref().map(AdapterId::as_str) == Some("api")
                        && model.id.as_str() == "gpt-4.1-mini"
                )
        );
        assert!(
            models
                .iter()
                .any(
                    |model| model.adapter_id.as_ref().map(AdapterId::as_str) == Some("codex")
                        && model.id.as_str() == "gpt-5-codex"
                )
        );

        let response = provider
            .complete_for_adapter(
                Some(&AdapterId::new("codex")),
                CompletionRequest {
                    model: ModelId::new("gpt-5-codex"),
                    system: None,
                    messages: Vec::new(),
                    tools: Vec::new(),
                    temperature: None,
                    max_output_tokens: None,
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    request_override: Default::default(),
                    response_format: None,
                },
            )
            .await
            .expect("completion should route");
        assert_eq!(response.provider_id.as_str(), "shared");
        assert_eq!(response.model.as_str(), "gpt-5-codex");
        assert_eq!(response.text, "shared::codex:gpt-5-codex");
    }

    #[tokio::test]
    async fn multi_adapter_provider_supports_single_adapter_passthrough() {
        let provider = MultiAdapterProvider::new(
            "openai",
            "default",
            "gpt-4.1",
            BTreeMap::from([(
                "default".to_owned(),
                Arc::new(StaticProvider {
                    id: "openai".to_owned(),
                    default_model: ModelId::new("gpt-4.1"),
                    models: vec![Model::new("openai", "gpt-4.1")],
                    list_models_error: None,
                }) as Arc<dyn ModelProvider>,
            )]),
            BTreeMap::new(),
        );

        let models = provider.list_models().await.expect("models should list");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider_id.as_str(), "openai");
        assert_eq!(
            models[0].adapter_id.as_ref().map(AdapterId::as_str),
            Some("default")
        );
        assert_eq!(models[0].id.as_str(), "gpt-4.1");

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-4.1"),
                system: None,
                messages: Vec::new(),
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
                prompt_window_generation: None,
                stop_sequences: Vec::new(),
                top_p: None,
                top_k: None,
                seed: None,
                thinking: None,
                request_override: Default::default(),
                response_format: None,
            })
            .await
            .expect("completion should pass through");
        assert_eq!(response.provider_id.as_str(), "openai");
        assert_eq!(response.model.as_str(), "gpt-4.1");
        assert_eq!(response.text, "openai:gpt-4.1");
    }

    #[tokio::test]
    async fn multi_adapter_provider_lists_configured_routes_when_live_listing_fails() {
        let provider = MultiAdapterProvider::new(
            "shared",
            "gitlab",
            "claude-sonnet-4-5",
            BTreeMap::from([(
                "gitlab".to_owned(),
                Arc::new(StaticProvider {
                    id: "shared::gitlab".to_owned(),
                    default_model: ModelId::new("claude-sonnet-4-5"),
                    models: Vec::new(),
                    list_models_error: Some("401 Unauthorized".to_owned()),
                }) as Arc<dyn ModelProvider>,
            )]),
            BTreeMap::from([(
                ("gitlab".to_owned(), "claude-sonnet-4-5".to_owned()),
                ProviderModelRoute {
                    enabled: true,
                    definition: ConfiguredModelDefinition::default(),
                },
            )]),
        );

        let models = provider
            .list_models()
            .await
            .expect("configured routes should still list");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "claude-sonnet-4-5");
        assert_eq!(
            models[0].adapter_id.as_ref().map(AdapterId::as_str),
            Some("gitlab")
        );
    }
}
