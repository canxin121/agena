use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    model::{AdapterId, Model, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode},
    model_catalog::{
        ModelCatalogProviderRecord, canonical_model_catalog_id,
        catalog_definition_to_provider_definition,
    },
};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelCapabilities, ModelProvider,
    PromptCacheShape, StreamResumePolicy, chat_wire,
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
            .or_else(|| {
                catalog_model_id_for_raw(model.as_str())
                    .as_ref()
                    .and_then(|catalog_model_id| self.provider.models.get(catalog_model_id))
            })
            .map(catalog_definition_to_provider_definition)
    }

    fn display_name_for_model(&self, model: &ModelId) -> Option<String> {
        self.provider
            .models
            .get(model.as_str())
            .or_else(|| {
                catalog_model_id_for_raw(model.as_str())
                    .as_ref()
                    .and_then(|catalog_model_id| self.provider.models.get(catalog_model_id))
            })
            .and_then(|definition| definition.display_name.clone())
    }

    fn apply_to_model(&self, model_id: &ModelId, mut model: Model) -> Model {
        if let Some(catalog_model_id) = catalog_model_id_for_raw(model_id.as_str()) {
            model.catalog_model_id = Some(ModelId::new(catalog_model_id));
        }
        if let Some(display_name) = self.display_name_for_model(model_id) {
            model.display_name = Some(display_name);
        }
        if let Some(configured) = self.configured_definition(model_id) {
            let capability_fallback = self
                .target
                .model_capabilities_for_adapter(model.adapter_id.as_ref(), model_id);
            let metadata_fallback = self
                .target
                .model_metadata_for_adapter(model.adapter_id.as_ref(), model_id);
            configured.apply_to_model(model, &capability_fallback, &metadata_fallback)
        } else {
            model
        }
    }

    fn backfill_assistant_reasoning_field(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &mut CompletionRequest,
    ) {
        let field = self
            .model_metadata_for_adapter(adapter_id, &request.model)
            .assistant_reasoning_field;
        chat_wire::backfill_assistant_reasoning_field_on_request(request, field.as_deref());
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
        self.configured_definition(model)
            .map(|configured| {
                configured.capabilities.apply_to(
                    self.target
                        .model_capabilities_for_adapter(adapter_id, model),
                )
            })
            .unwrap_or_else(|| {
                self.target
                    .model_capabilities_for_adapter(adapter_id, model)
            })
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
        self.configured_definition(model)
            .map(|configured| configured.metadata().with_fallbacks_from(&metadata))
            .unwrap_or(metadata)
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        self.model_thinking_modes_for_adapter(None, model)
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        self.configured_definition(model)
            .map(|configured| {
                let mut modes = self
                    .target
                    .model_thinking_modes_for_adapter(adapter_id, model);
                for (name, configured_mode) in &configured.thinking_modes {
                    match configured_mode.apply_to_mode(modes.get(name)) {
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
            .unwrap_or_else(|| {
                self.target
                    .model_thinking_modes_for_adapter(adapter_id, model)
            })
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
                let mut modes = self.target.model_speed_modes_for_adapter(adapter_id, model);
                for (name, configured_mode) in &configured.speed_modes {
                    match configured_mode.apply_to_mode(modes.get(name)) {
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

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        self.backfill_assistant_reasoning_field(None, &mut request);
        self.target.complete(request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        self.target.complete_for_adapter(adapter_id, request).await
    }

    async fn complete_stream(
        &self,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.backfill_assistant_reasoning_field(None, &mut request);
        self.target.complete_stream(request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        self.target
            .complete_stream_for_adapter(adapter_id, request)
            .await
    }
}

fn catalog_model_id_for_raw(raw_model_id: &str) -> Option<String> {
    let canonical = canonical_model_catalog_id(raw_model_id);
    let trimmed = canonical.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{Message, PartContent, ReasoningPart},
        model::ModelLifecycle,
        provider::{
            CapabilitySupport, CompletionFinishReason, CompletionResponse,
            ConfiguredModelThinkingMode, ThinkingRequest,
        },
        role::Role,
    };
    use std::sync::Mutex;

    #[derive(Clone)]
    struct StaticProvider {
        default_model: ModelId,
        listed: Vec<Model>,
        captured_requests: Arc<Mutex<Vec<CompletionRequest>>>,
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
            ModelMetadata::default()
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(self.listed.clone())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            self.captured_requests
                .lock()
                .expect("captured requests lock should not be poisoned")
                .push(request.clone());
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
            captured_requests: Arc::new(Mutex::new(Vec::new())),
        });
        let provider = CatalogedModelsProvider::new(
            target,
            ModelCatalogProviderRecord {
                models: BTreeMap::from([(
                    "gpt-5".to_owned(),
                    crate::model_catalog::CatalogModelDefinition {
                        display_name: Some("GPT 5".to_owned()),
                        lifecycle: Some(ModelLifecycle::Preview),
                        thinking_modes: BTreeMap::from([(
                            "deep".to_owned(),
                            ConfiguredModelThinkingMode {
                                display_name: Some("Deep".to_owned()),
                                description: None,
                                thinking: Some(ThinkingRequest::Budget {
                                    budget_tokens: 20_000,
                                }),
                                request_override: Default::default(),
                                adapter_overrides: BTreeMap::new(),
                                disabled: false,
                            },
                        )]),
                        ..crate::model_catalog::CatalogModelDefinition::default()
                    },
                )]),
                appendable_model_ids: std::collections::BTreeSet::from(["gpt-5".to_owned()]),
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
        assert!(gpt5.thinking_modes.contains_key("deep"));
    }

    #[tokio::test]
    async fn catalog_wrapper_backfills_assistant_reasoning_field_from_catalog_metadata() {
        let captured_requests = Arc::new(Mutex::new(Vec::new()));
        let target: Arc<dyn ModelProvider> = Arc::new(StaticProvider {
            default_model: ModelId::new("deepseek-v4-pro"),
            listed: vec![Model::new("openai", "deepseek-v4-pro")],
            captured_requests: Arc::clone(&captured_requests),
        });
        let provider = CatalogedModelsProvider::new(
            target,
            ModelCatalogProviderRecord {
                models: BTreeMap::from([(
                    "deepseek-v4-pro".to_owned(),
                    crate::model_catalog::CatalogModelDefinition {
                        assistant_reasoning_field: Some("reasoning_content".to_owned()),
                        ..crate::model_catalog::CatalogModelDefinition::default()
                    },
                )]),
                appendable_model_ids: Default::default(),
            },
        );

        provider
            .complete(CompletionRequest {
                model: ModelId::new("deepseek-v4-pro"),
                system: None,
                messages: vec![
                    Message::prompt_parts(
                        Role::Assistant,
                        vec![
                            PartContent::Reasoning(ReasoningPart {
                                summary: vec!["Prior chain".to_owned()],
                                raw_content: Vec::new(),
                                encrypted_content: None,
                            }),
                            PartContent::text("Prior answer"),
                        ],
                    ),
                    Message::prompt_text(Role::User, "continue"),
                ],
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
                verbosity: None,
                request_override: Default::default(),
                response_format: None,
            })
            .await
            .expect("catalog wrapper completion should succeed");

        let captured = captured_requests
            .lock()
            .expect("captured requests lock should not be poisoned");
        let assistant = captured[0]
            .messages
            .iter()
            .find(|message| matches!(message.role, Role::Assistant))
            .expect("assistant message should be present");
        assert_eq!(
            assistant
                .metadata
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("assistant_reasoning_field"))
                .and_then(|value| value.as_str()),
            Some("reasoning_content")
        );
    }
}
