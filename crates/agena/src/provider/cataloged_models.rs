use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    model::{Model, ModelId, ModelMetadata, ModelVariant},
    model_catalog::{ModelCatalogProviderRecord, catalog_definition_to_provider_definition},
};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelCapabilities, ModelProvider,
    PromptCacheShape, StreamResumePolicy,
};

#[derive(Clone)]
pub struct CatalogedModelsProvider {
    target: Arc<dyn ModelProvider>,
    provider: Arc<ModelCatalogProviderRecord>,
}

impl CatalogedModelsProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        target: Arc<dyn ModelProvider>,
        provider: ModelCatalogProviderRecord,
    ) -> Arc<dyn ModelProvider> {
        if provider.models.is_empty() {
            target
        } else {
            Arc::new(Self {
                target,
                provider: Arc::new(provider),
            })
        }
    }

    fn configured_definition(
        &self,
        model: &ModelId,
    ) -> Option<crate::provider::ConfiguredModelDefinition> {
        self.provider
            .models
            .get(model.as_str())
            .map(catalog_definition_to_provider_definition)
    }

    fn display_name_for_model(&self, model: &ModelId) -> Option<String> {
        self.provider
            .models
            .get(model.as_str())
            .and_then(|definition| definition.display_name.clone())
    }

    fn apply_to_model(&self, model_id: &ModelId, mut model: Model) -> Model {
        if let Some(display_name) = self.display_name_for_model(model_id) {
            model.display_name = Some(display_name);
        }
        if let Some(configured) = self.configured_definition(model_id) {
            let capability_fallback = self.target.model_capabilities(model_id);
            let metadata_fallback = self.target.model_metadata(model_id);
            configured.apply_to_model(model, &capability_fallback, &metadata_fallback)
        } else {
            model
        }
    }
}

#[async_trait]
impl ModelProvider for CatalogedModelsProvider {
    fn id(&self) -> &str {
        self.target.id()
    }

    fn default_model(&self) -> &ModelId {
        self.target.default_model()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.configured_definition(model)
            .map(|configured| {
                configured
                    .capabilities
                    .apply_to(self.target.model_capabilities(model))
            })
            .unwrap_or_else(|| self.target.model_capabilities(model))
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        let mut metadata = self.target.model_metadata(model);
        if let Some(definition) = self.provider.models.get(model.as_str()) {
            if let Some(family) = definition.family {
                metadata.family = Some(family);
            }
        }
        self.configured_definition(model)
            .map(|configured| configured.metadata().with_fallbacks_from(&metadata))
            .unwrap_or(metadata)
    }

    fn model_variants(&self, model: &ModelId) -> BTreeMap<String, ModelVariant> {
        self.configured_definition(model)
            .map(|configured| {
                let mut variants = self.target.model_variants(model);
                for (name, configured_variant) in &configured.variants {
                    match configured_variant.apply_to_variant(variants.get(name)) {
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
            .unwrap_or_else(|| self.target.model_variants(model))
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.target.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.target.prompt_cache_shape(model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        let mut listed = std::collections::BTreeSet::new();
        for model in &mut models {
            listed.insert(model.id.to_string());
            *model = self.apply_to_model(&model.id.clone(), model.clone());
        }

        for model_id in self.provider.models.keys() {
            if listed.contains(model_id.as_str()) {
                continue;
            }
            let model_id = ModelId::new(model_id.clone());
            let base = Model::new(self.target.id(), model_id.as_str())
                .with_capabilities(self.model_capabilities(&model_id))
                .with_metadata(self.model_metadata(&model_id))
                .with_variants(self.model_variants(&model_id));
            models.push(self.apply_to_model(&model_id, base));
        }
        Ok(models)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.target.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.target.complete_stream(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{ModelFamily, ModelLifecycle},
        provider::{
            CapabilitySupport, CompletionFinishReason, CompletionResponse, ConfiguredModelVariant,
            ThinkingRequest,
        },
    };

    #[derive(Clone)]
    struct StaticProvider {
        default_model: ModelId,
        listed: Vec<Model>,
    }

    #[async_trait]
    impl ModelProvider for StaticProvider {
        fn id(&self) -> &str {
            "openai"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        fn model_capabilities(&self, _model: &ModelId) -> ModelCapabilities {
            ModelCapabilities::default().with_streaming(CapabilitySupport::Supported)
        }

        fn model_metadata(&self, _model: &ModelId) -> ModelMetadata {
            ModelMetadata::default().with_family(ModelFamily::Gpt)
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(self.listed.clone())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: crate::model::ProviderId::new("openai"),
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

    #[tokio::test]
    async fn catalog_wrapper_lists_custom_models_without_overriding_default() {
        let target: Arc<dyn ModelProvider> = Arc::new(StaticProvider {
            default_model: ModelId::new("gpt-4.1"),
            listed: vec![Model::new("openai", "gpt-4.1").with_display_name("GPT 4.1")],
        });
        let provider = CatalogedModelsProvider::new(
            target,
            ModelCatalogProviderRecord {
                models: BTreeMap::from([(
                    "gpt-5".to_owned(),
                    crate::model_catalog::CatalogModelDefinition {
                        display_name: Some("GPT 5".to_owned()),
                        lifecycle: Some(ModelLifecycle::Preview),
                        variants: BTreeMap::from([(
                            "deep".to_owned(),
                            ConfiguredModelVariant {
                                display_name: Some("Deep".to_owned()),
                                description: None,
                                thinking: Some(ThinkingRequest::Budget {
                                    budget_tokens: 20_000,
                                }),
                                disabled: false,
                            },
                        )]),
                        ..crate::model_catalog::CatalogModelDefinition::default()
                    },
                )]),
            },
        );

        assert_eq!(provider.default_model().as_str(), "gpt-4.1");
        let models = provider
            .list_models()
            .await
            .expect("list models should work");
        assert!(models.iter().any(|model| model.id.as_str() == "gpt-5"));
        let gpt5 = models
            .iter()
            .find(|model| model.id.as_str() == "gpt-5")
            .expect("gpt-5 should be present");
        assert_eq!(gpt5.display_name.as_deref(), Some("GPT 5"));
        assert_eq!(gpt5.metadata.lifecycle, Some(ModelLifecycle::Preview));
        assert!(gpt5.variants.contains_key("deep"));
    }
}
