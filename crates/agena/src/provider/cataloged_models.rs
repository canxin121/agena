use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    model::{AdapterId, Model, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode},
    model_catalog::{
        ModelCatalogProviderRecord, apply_catalog_definition_as_baseline,
        apply_catalog_display_name_as_fallback, canonical_model_catalog_id,
        catalog_definition_to_provider_definition, merge_catalog_baseline_speed_modes,
        merge_catalog_baseline_thinking_modes,
    },
};

use super::core::ForwardingModelRuntime;
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelCapabilities, ModelRuntime,
    PromptCacheShape, StreamResumePolicy,
};

#[derive(Clone)]
pub struct CatalogedModelsProvider {
    target: Arc<dyn ModelRuntime>,
    provider: Arc<ModelCatalogProviderRecord>,
}

impl CatalogedModelsProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        target: Arc<dyn ModelRuntime>,
        provider: ModelCatalogProviderRecord,
    ) -> Arc<dyn ModelRuntime> {
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
            .or_else(|| {
                catalog_model_id_for_raw(model.as_str())
                    .as_ref()
                    .and_then(|catalog_model_id| self.provider.models.get(catalog_model_id))
            })
            .map(catalog_definition_to_provider_definition)
    }

    fn apply_to_model(&self, model_id: &ModelId, mut model: Model) -> Model {
        if let Some(catalog_model_id) = catalog_model_id_for_raw(model_id.as_str()) {
            model.catalog_model_id = Some(ModelId::new(catalog_model_id));
        }
        if let Some(definition) = self.provider.models.get(model_id.as_str()).or_else(|| {
            catalog_model_id_for_raw(model_id.as_str())
                .as_ref()
                .and_then(|catalog_model_id| self.provider.models.get(catalog_model_id))
        }) {
            apply_catalog_display_name_as_fallback(&mut model, definition);
            let capability_fallback = self
                .target
                .model_capabilities_for_adapter(model.adapter_id.as_ref(), model_id);
            let metadata_fallback = self
                .target
                .model_metadata_for_adapter(model.adapter_id.as_ref(), model_id);
            apply_catalog_definition_as_baseline(
                definition,
                &capability_fallback,
                &metadata_fallback,
                model,
            )
        } else {
            model
        }
    }

    fn synthesize_thinking_modes_from_metadata(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let Some(family) = self.target.capability_family() else {
            return BTreeMap::new();
        };
        crate::provider::default_model_mode_registry().thinking_modes_for_family(
            family,
            adapter_id,
            model.as_str(),
            &self.model_metadata_for_adapter(adapter_id, model),
        )
    }
}

#[async_trait]
impl ForwardingModelRuntime for CatalogedModelsProvider {
    fn target(&self) -> &dyn ModelRuntime {
        self.target.as_ref()
    }

    fn prepare_request(&self, adapter_id: Option<&AdapterId>, request: &mut CompletionRequest) {
        ModelRuntime::backfill_assistant_reasoning_field(self, adapter_id, request);
    }
}

#[async_trait]
impl ModelRuntime for CatalogedModelsProvider {
    fn id(&self) -> &str {
        self.target.id()
    }

    fn default_model(&self) -> &ModelId {
        self.target.default_model()
    }

    fn default_adapter(&self) -> Option<&AdapterId> {
        self.target.default_adapter()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.model_capabilities_for_adapter(None, model)
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let primary = self
            .target
            .model_capabilities_for_adapter(adapter_id, model);
        if let Some(configured) = self.configured_definition(model) {
            primary.with_fallbacks_from(
                &configured
                    .capabilities
                    .apply_to(ModelCapabilities::default()),
            )
        } else {
            primary
        }
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        self.model_metadata_for_adapter(None, model)
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        let metadata = self.target.model_metadata_for_adapter(adapter_id, model);
        if let Some(configured) = self.configured_definition(model) {
            metadata.with_fallbacks_from(&configured.metadata())
        } else {
            metadata
        }
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        self.model_thinking_modes_for_adapter(None, model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let mut modes = self
            .target
            .model_thinking_modes_for_adapter(adapter_id, model);
        for (name, mode) in self.synthesize_thinking_modes_from_metadata(adapter_id, model) {
            modes.entry(name).or_insert(mode);
        }
        if let Some(configured) = self.configured_definition(model) {
            modes = merge_catalog_baseline_thinking_modes(modes, &configured.thinking_modes);
        }
        modes
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        self.model_speed_modes_for_adapter(None, model)
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.configured_definition(model)
            .map(|configured| {
                merge_catalog_baseline_speed_modes(
                    self.target.model_speed_modes_for_adapter(adapter_id, model),
                    &configured.speed_modes,
                )
            })
            .unwrap_or_else(|| self.target.model_speed_modes_for_adapter(adapter_id, model))
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.target.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.supports_prompt_continuation_for_adapter(None, model)
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.target
            .supports_prompt_continuation_for_adapter(adapter_id, model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.prompt_cache_shape_for_adapter(None, model)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        self.target
            .prompt_cache_shape_for_adapter(adapter_id, model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        let mut listed = std::collections::BTreeSet::new();
        let mut listed_catalog_ids = std::collections::BTreeSet::new();
        for model in &mut models {
            listed.insert(model.id.to_string());
            if let Some(catalog_model_id) = catalog_model_id_for_raw(model.id.as_str()) {
                listed_catalog_ids.insert(catalog_model_id.clone());
                model.catalog_model_id = Some(ModelId::new(catalog_model_id));
            }
            *model = self.apply_to_model(&model.id.clone(), model.clone());
        }

        for model_id in &self.provider.appendable_model_ids {
            if listed.contains(model_id.as_str())
                || listed_catalog_ids.contains(model_id.as_str())
                || models.iter().any(|model| {
                    model.id.as_str() == model_id.as_str()
                        || model.catalog_model_id.as_ref().map(ModelId::as_str)
                            == Some(model_id.as_str())
                })
            {
                continue;
            }
            let model_id = ModelId::new(model_id.clone());
            let base = Model::new(self.target.id(), model_id.as_str())
                .with_catalog_model_id(model_id.as_str())
                .with_capabilities(self.model_capabilities(&model_id))
                .with_metadata(self.model_metadata(&model_id))
                .with_thinking_modes(self.model_thinking_modes(&model_id))
                .with_speed_modes(self.model_speed_modes(&model_id));
            models.push(self.apply_to_model(&model_id, base));
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

fn catalog_model_id_for_raw(raw_model_id: &str) -> Option<String> {
    let canonical = canonical_model_catalog_id(raw_model_id);
    let trimmed = canonical.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
