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

use super::core::remap_stream_event_provider_and_model;
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ConfiguredModelDefinition,
    ModelCapabilities, ModelRuntime, PromptCacheShape, StreamResumePolicy,
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
    adapters: BTreeMap<String, Arc<dyn ModelRuntime>>,
    routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
    configured_only_adapters: BTreeSet<String>,
}

impl MultiAdapterProvider {
    pub fn new(
        id: impl Into<String>,
        default_adapter: impl Into<String>,
        default_model: impl Into<String>,
        adapters: BTreeMap<String, Arc<dyn ModelRuntime>>,
        routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
    ) -> Self {
        Self {
            id: id.into(),
            default_adapter: AdapterId::new(default_adapter),
            default_model: ModelId::new(default_model),
            adapters,
            routes,
            configured_only_adapters: BTreeSet::new(),
        }
    }

    pub fn with_configured_only_adapters(mut self, adapters: BTreeSet<String>) -> Self {
        self.configured_only_adapters = adapters;
        self
    }

    fn adapter(&self, adapter_id: &str) -> Result<Arc<dyn ModelRuntime>, AppError> {
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
        adapter: &dyn ModelRuntime,
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
        adapter: &dyn ModelRuntime,
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
impl ModelRuntime for MultiAdapterProvider {
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
            let listed = if self.configured_only_adapters.contains(adapter_id.as_str()) {
                Vec::new()
            } else {
                match adapter.list_models().await {
                    Ok(models) => models,
                    Err(error) => {
                        errors.push(format!("{adapter_id}: {error}"));
                        Vec::new()
                    }
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
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (adapter_id, target_model, _) = self.resolve_route(adapter_id, &visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        let mut response = adapter.complete(request).await?;
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.compact_conversation_for_adapter(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let visible_model = request.model.clone();
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (adapter_id, target_model, _) = self.resolve_route(adapter_id, &visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        adapter.compact_conversation(request).await
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
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (adapter_id, target_model, _) = self.resolve_route(adapter_id, &visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        let provider_id = self.id.clone();
        let stream = adapter.complete_stream(request).await?;
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
            Box::pin(stream.map(move |item| {
                let provider_id = ProviderId::new(provider_id.clone());
                let visible_model = visible_model.clone();
                item.map(|event| {
                    remap_stream_event_provider_and_model(&provider_id, &visible_model, event)
                })
            }));
        Ok(Box::pin(stream))
    }
}
