use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{StreamExt, stream::BoxStream};

use crate::{
    error::AppError,
    model::{Model, ModelId, ModelMetadata, ModelVariant, ProviderId},
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

#[derive(Clone)]
pub struct MultiAdapterProvider {
    id: String,
    default_model: ModelId,
    adapters: BTreeMap<String, Arc<dyn ModelProvider>>,
    routes: BTreeMap<String, ProviderModelRoute>,
}

impl MultiAdapterProvider {
    pub fn new(
        id: impl Into<String>,
        default_model: impl Into<String>,
        adapters: BTreeMap<String, Arc<dyn ModelProvider>>,
        routes: BTreeMap<String, ProviderModelRoute>,
    ) -> Self {
        Self {
            id: id.into(),
            default_model: ModelId::new(default_model),
            adapters,
            routes,
        }
    }

    fn adapter(&self, adapter_id: &str) -> Result<Arc<dyn ModelProvider>, AppError> {
        self.adapters
            .get(adapter_id)
            .cloned()
            .ok_or_else(|| {
                AppError::Config(format!(
                    "provider `{}` has no enabled adapter `{adapter_id}`",
                    self.id
                ))
            })
    }

    fn parse_visible_model_id(&self, visible_model_id: &str) -> Result<(String, ModelId), AppError> {
        let Some((adapter_id, model_id)) = visible_model_id.split_once('/') else {
            return Err(AppError::Config(format!(
                "provider `{}` model `{visible_model_id}` must be in `<adapter>/<model>` format",
                self.id
            )));
        };

        let adapter_id = adapter_id.trim();
        let model_id = model_id.trim();
        if adapter_id.is_empty() || model_id.is_empty() {
            return Err(AppError::Config(format!(
                "provider `{}` model `{visible_model_id}` must be in `<adapter>/<model>` format",
                self.id
            )));
        }

        Ok((adapter_id.to_owned(), ModelId::new(model_id)))
    }

    fn resolve_route(
        &self,
        model: &ModelId,
    ) -> Result<(String, ModelId, ConfiguredModelDefinition), AppError> {
        let (adapter_id, target_model) = self.parse_visible_model_id(model.as_str())?;
        if let Some(route) = self.routes.get(model.as_str()) {
            if !route.enabled {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` is disabled",
                    self.id, model
                )));
            }
            return Ok((
                adapter_id,
                target_model,
                route.definition.clone(),
            ));
        }

        self.adapter(adapter_id.as_str())?;
        Ok((adapter_id, target_model, ConfiguredModelDefinition::default()))
    }

    fn rewrite_model(
        &self,
        visible_model: &ModelId,
        target_model: &ModelId,
        mut model: Model,
        adapter: &dyn ModelProvider,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        model.provider_id = ProviderId::new(self.id.clone());
        model.id = visible_model.clone();
        definition.apply_to_model(
            model,
            &adapter.model_capabilities(target_model),
            &adapter.model_metadata(target_model),
        )
    }

    fn synthesize_model(
        &self,
        visible_model: &ModelId,
        target_model: &ModelId,
        adapter: &dyn ModelProvider,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        definition.apply_to_model(
            Model::new(self.id.as_str(), visible_model.as_str()),
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

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.resolve_route(model)
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
        self.resolve_route(model)
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

    fn model_variants(&self, model: &ModelId) -> BTreeMap<String, ModelVariant> {
        self.resolve_route(model)
            .ok()
            .and_then(|(adapter_id, target_model, definition)| {
                self.adapter(adapter_id.as_str()).ok().map(|adapter| {
                    let mut variants = adapter.model_variants(&target_model);
                    for (name, configured) in &definition.variants {
                        match configured.apply_to_variant(variants.get(name)) {
                            Some(variant) => {
                                variants.insert(name.clone(), variant);
                            }
                            None => {
                                variants.remove(name);
                            }
                        }
                    }
                    variants
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
        self.resolve_route(model)
            .ok()
            .and_then(|(adapter_id, target_model, _)| {
                self.adapter(adapter_id.as_str())
                    .ok()
                    .map(|adapter| adapter.supports_prompt_continuation(&target_model))
            })
            .unwrap_or(false)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.resolve_route(model)
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

        for (adapter_id, adapter) in &self.adapters {
            let listed = adapter.list_models().await?;
            for model in listed {
                let target_model = model.id.clone();
                let visible_model = ModelId::new(format!("{adapter_id}/{}", target_model.as_str()));
                let route = self.routes.get(visible_model.as_str());
                if matches!(route, Some(route) if !route.enabled) {
                    continue;
                }
                let definition = route
                    .map(|route| route.definition.clone())
                    .unwrap_or_default();
                seen.insert(visible_model.to_string());
                visible.push(self.rewrite_model(
                    &visible_model,
                    &target_model,
                    model,
                    adapter.as_ref(),
                    &definition,
                ));
            }
        }

        for (visible_model_id, route) in &self.routes {
            if !route.enabled || seen.contains(visible_model_id) {
                continue;
            }

            let (adapter_id, target_model) = self.parse_visible_model_id(visible_model_id)?;
            let adapter = self.adapter(adapter_id.as_str())?;
            let visible_model = ModelId::new(visible_model_id.clone());
            visible.push(self.synthesize_model(
                &visible_model,
                &target_model,
                adapter.as_ref(),
                &route.definition,
            ));
        }

        Ok(visible)
    }

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let visible_model = request.model.clone();
        let (adapter_id, target_model, _) = self.resolve_route(&visible_model)?;
        let adapter = self.adapter(adapter_id.as_str())?;
        request.model = target_model;
        let mut response = adapter.complete(request).await?;
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn complete_stream(
        &self,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let visible_model = request.model.clone();
        let (adapter_id, target_model, _) = self.resolve_route(&visible_model)?;
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
            Ok(self.models.clone())
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
            "api/gpt-4.1-mini",
            BTreeMap::from([
                (
                    "api".to_owned(),
                    Arc::new(StaticProvider {
                        id: "shared::api".to_owned(),
                        default_model: ModelId::new("gpt-4.1"),
                        models: vec![Model::new("shared::api", "gpt-4.1-mini")],
                    }) as Arc<dyn ModelProvider>,
                ),
                (
                    "codex".to_owned(),
                    Arc::new(StaticProvider {
                        id: "shared::codex".to_owned(),
                        default_model: ModelId::new("gpt-5-codex"),
                        models: vec![Model::new("shared::codex", "gpt-5-codex")],
                    }) as Arc<dyn ModelProvider>,
                ),
            ]),
            BTreeMap::from([
                (
                    "api/gpt-4.1-mini".to_owned(),
                    ProviderModelRoute {
                        enabled: true,
                        definition: ConfiguredModelDefinition::default(),
                    },
                ),
                (
                    "codex/gpt-5-codex".to_owned(),
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
        assert!(ids.iter().any(|id| id == "api/gpt-4.1-mini"));
        assert!(ids.iter().any(|id| id == "codex/gpt-5-codex"));

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("codex/gpt-5-codex"),
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
                response_format: None,
            })
            .await
            .expect("completion should route");
        assert_eq!(response.provider_id.as_str(), "shared");
        assert_eq!(response.model.as_str(), "codex/gpt-5-codex");
        assert_eq!(response.text, "shared::codex:gpt-5-codex");
    }

    #[tokio::test]
    async fn multi_adapter_provider_supports_single_adapter_passthrough() {
        let provider = MultiAdapterProvider::new(
            "openai",
            "default/gpt-4.1",
            BTreeMap::from([(
                "default".to_owned(),
                Arc::new(StaticProvider {
                    id: "openai".to_owned(),
                    default_model: ModelId::new("gpt-4.1"),
                    models: vec![Model::new("openai", "gpt-4.1")],
                }) as Arc<dyn ModelProvider>,
            )]),
            BTreeMap::new(),
        );

        let models = provider.list_models().await.expect("models should list");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider_id.as_str(), "openai");
        assert_eq!(models[0].id.as_str(), "default/gpt-4.1");

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("default/gpt-4.1"),
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
                response_format: None,
            })
            .await
            .expect("completion should pass through");
        assert_eq!(response.provider_id.as_str(), "openai");
        assert_eq!(response.model.as_str(), "default/gpt-4.1");
        assert_eq!(response.text, "openai:gpt-4.1");
    }
}
