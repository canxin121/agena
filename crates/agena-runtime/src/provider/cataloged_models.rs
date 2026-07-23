use agena_domain::*;
use agena_provider::{
    AgenaToolMode, PromptCacheShape, ProviderCompactionOutput, ProviderNativeToolsConfig,
    StreamResumePolicy, apply_configured_definition_as_baseline,
    merge_catalog_baseline_speed_modes, merge_catalog_baseline_thinking_modes,
};

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;

use crate::error::AppError;
use agena_provider::catalog_model_id_for_raw;

use super::core::{
    ForwardingModelRuntime, impl_model_runtime_base_via_adapter_methods,
    impl_model_runtime_target_defaults, impl_model_runtime_target_methods,
};
use super::{CompletionResponse, ModelRuntime};
use agena_provider::{
    CompletionRequest, CompletionStreamEvent, ConfiguredModelDefinition, ProviderModelCatalog,
};

#[derive(Clone)]
pub(crate) struct CatalogedModelsProvider {
    target: Arc<dyn ModelRuntime>,
    provider: Arc<ProviderModelCatalog>,
}

impl CatalogedModelsProvider {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        target: Arc<dyn ModelRuntime>,
        provider: ProviderModelCatalog,
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

    fn provider_definition(&self, model: &ModelId) -> Option<&ConfiguredModelDefinition> {
        self.provider.models.get(model.as_ref()).or_else(|| {
            catalog_model_id_for_raw(model.as_ref())
                .as_deref()
                .and_then(|catalog_model_id| self.provider.models.get(catalog_model_id))
        })
    }

    fn using_configured_definition<T>(
        &self,
        model: &ModelId,
        base: T,
        map: impl FnOnce(T, &ConfiguredModelDefinition) -> T,
    ) -> T {
        match self.provider_definition(model) {
            Some(configured) => map(base, configured),
            None => base,
        }
    }

    fn apply_catalog_model_id(&self, model_id: &ModelId, model: &mut Model) -> Option<String> {
        let catalog_model_id = catalog_model_id_for_raw(model_id.as_ref())?;
        model.catalog_model_id = Some(ModelId::new(catalog_model_id.clone()));
        Some(catalog_model_id)
    }

    fn apply_to_model(&self, model_id: &ModelId, mut model: Model) -> Model {
        self.apply_catalog_model_id(model_id, &mut model);
        if let Some(definition) = self.provider_definition(model_id) {
            if model.display_name.is_none() {
                model.display_name = definition.display_name.clone();
            }
            let capability_fallback = self
                .target
                .model_capabilities_for_adapter(model.adapter_id.as_ref(), model_id);
            let metadata_fallback = self
                .target
                .model_metadata_for_adapter(model.adapter_id.as_ref(), model_id);
            apply_configured_definition_as_baseline(
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
    ) -> Vec<ModelThinkingMode> {
        let Some(family) = self.target.capability_family() else {
            return Vec::new();
        };
        agena_provider::default_model_mode_registry().thinking_modes_for_family(
            family,
            adapter_id,
            model.as_ref(),
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

    impl_model_runtime_target_defaults!();

    impl_model_runtime_base_via_adapter_methods! {
        fn model_capabilities / model_capabilities_for_adapter (&self, model: &ModelId) -> ModelCapabilities;
        fn model_metadata / model_metadata_for_adapter (&self, model: &ModelId) -> ModelMetadata;
        fn model_thinking_modes / model_thinking_modes_for_adapter (&self, model: &ModelId) -> Vec<ModelThinkingMode>;
        fn model_speed_modes / model_speed_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode>;
        fn supports_prompt_continuation / supports_prompt_continuation_for_adapter (&self, model: &ModelId) -> bool;
        fn prompt_cache_shape / prompt_cache_shape_for_adapter (&self, model: &ModelId) -> Option<PromptCacheShape>;
        fn provider_native_tools_config / provider_native_tools_config_for_adapter (&self, model: &ModelId) -> ProviderNativeToolsConfig;
    }

    impl_model_runtime_target_methods! {
        fn native_compaction_enabled / native_compaction_enabled_for_adapter (&self, model: &ModelId) -> bool;
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

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let primary = self
            .target
            .model_capabilities_for_adapter(adapter_id, model);
        self.using_configured_definition(model, primary, |primary, configured| {
            primary.merged_with_fallbacks_from(
                &configured
                    .capabilities
                    .apply_to(ModelCapabilities::default()),
            )
        })
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        let metadata = self.target.model_metadata_for_adapter(adapter_id, model);
        self.using_configured_definition(model, metadata, |metadata, configured| {
            metadata.merged_with_fallbacks_from(&configured.metadata())
        })
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        let mut modes = self
            .target
            .model_thinking_modes_for_adapter(adapter_id, model);
        for mode in self.synthesize_thinking_modes_from_metadata(adapter_id, model) {
            let selector = mode.selector().map(|selector| selector.into_owned());
            if selector.is_some_and(|selector| {
                modes
                    .iter()
                    .any(|existing| existing.selector().as_deref() == Some(selector.as_str()))
            }) {
                continue;
            }
            modes.push(mode);
        }
        self.using_configured_definition(model, modes, |modes, configured| {
            merge_catalog_baseline_thinking_modes(modes, &configured.thinking_modes)
        })
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.using_configured_definition(
            model,
            self.target.model_speed_modes_for_adapter(adapter_id, model),
            |modes, configured| merge_catalog_baseline_speed_modes(modes, &configured.speed_modes),
        )
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.target
            .supports_prompt_continuation_for_adapter(adapter_id, model)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        self.target
            .prompt_cache_shape_for_adapter(adapter_id, model)
    }

    fn provider_native_tools_config_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ProviderNativeToolsConfig {
        self.target
            .provider_native_tools_config_for_adapter(adapter_id, model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let mut models = self.target.list_models().await?;
        let mut listed = std::collections::BTreeSet::new();
        let mut listed_catalog_ids = std::collections::BTreeSet::new();
        for model in &mut models {
            listed.insert(model.id.to_string());
            if let Some(catalog_model_id) = self.apply_catalog_model_id(&model.id.clone(), model) {
                listed_catalog_ids.insert(catalog_model_id);
            }
            *model = self.apply_to_model(&model.id.clone(), model.clone());
        }

        for model_id in &self.provider.appendable_model_ids {
            if listed.contains(model_id.as_str())
                || listed_catalog_ids.contains(model_id.as_str())
                || models.iter().any(|model| {
                    model.id.as_ref() == AsRef::<str>::as_ref(model_id)
                        || model.catalog_model_id.as_ref().map(AsRef::<str>::as_ref)
                            == Some(AsRef::<str>::as_ref(model_id))
                })
            {
                continue;
            }
            let model_id = ModelId::new(model_id.clone());
            let base = Model {
                provider_id: ProviderId::new(self.target.id()),
                adapter_id: None,
                id: ModelId::new(model_id.as_ref()),
                catalog_model_id: Some(model_id.clone()),
                display_name: None,
                native_compaction: self.native_compaction_enabled(&model_id),
                capabilities: self.model_capabilities(&model_id),
                metadata: self.model_metadata(&model_id),
                thinking_modes: self.model_thinking_modes(&model_id),
                speed_modes: self.model_speed_modes(&model_id),
            };
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
    ) -> Result<Option<ProviderCompactionOutput>, AppError> {
        self.forward_compact_conversation(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, AppError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct ToolModeRuntime {
        default_model: ModelId,
    }

    #[async_trait]
    impl ModelRuntime for ToolModeRuntime {
        fn id(&self) -> &str {
            "test"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        fn agena_tool_mode(&self, _model: &ModelId) -> AgenaToolMode {
            AgenaToolMode::ProviderProtocol
        }

        fn agena_tool_mode_for_adapter(
            &self,
            adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> AgenaToolMode {
            if adapter_id.is_some() {
                AgenaToolMode::PromptEnvelope
            } else {
                AgenaToolMode::ProviderProtocol
            }
        }

        fn native_compaction_enabled_for_adapter(
            &self,
            adapter_id: Option<&AdapterId>,
            _model: &ModelId,
        ) -> bool {
            adapter_id.is_none()
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            unreachable!("tool-mode forwarding does not perform a completion")
        }
    }

    #[test]
    fn catalog_wrapper_forwards_base_and_adapter_tool_modes() {
        let model = ModelId::new("model");
        let target: Arc<dyn ModelRuntime> = Arc::new(ToolModeRuntime {
            default_model: model.clone(),
        });
        let provider = CatalogedModelsProvider::new(
            target,
            ProviderModelCatalog {
                models: [(model.to_string(), ConfiguredModelDefinition::default())]
                    .into_iter()
                    .collect(),
                appendable_model_ids: Default::default(),
            },
        );

        assert_eq!(
            provider.agena_tool_mode(&model),
            AgenaToolMode::ProviderProtocol
        );
        assert_eq!(
            provider.agena_tool_mode_for_adapter(Some(&AdapterId::new("adapter")), &model),
            AgenaToolMode::PromptEnvelope
        );
        assert!(provider.native_compaction_enabled(&model));
        assert!(
            !provider
                .native_compaction_enabled_for_adapter(Some(&AdapterId::new("adapter")), &model,)
        );
    }

    #[test]
    fn catalog_model_id_projection_uses_core_canonicalization_once() {
        assert_eq!(
            catalog_model_id_for_raw("OpenAI/gpt-4o@default").as_deref(),
            Some("gpt-4o")
        );
        assert_eq!(catalog_model_id_for_raw("  "), None);
    }

    #[test]
    fn catalog_wrapper_appends_unlisted_catalog_models() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        runtime.block_on(async {
            let target: Arc<dyn ModelRuntime> = Arc::new(ToolModeRuntime {
                default_model: ModelId::new("base"),
            });
            let provider = CatalogedModelsProvider::new(
                target,
                ProviderModelCatalog {
                    models: [(
                        "catalog-model".to_owned(),
                        ConfiguredModelDefinition::default(),
                    )]
                    .into_iter()
                    .collect(),
                    appendable_model_ids: ["catalog-model".to_owned()].into_iter().collect(),
                },
            );

            let models = provider.list_models().await.expect("list catalog models");
            assert_eq!(models.len(), 1);
            assert_eq!(models[0].id.as_ref(), "catalog-model");
            assert_eq!(
                models[0]
                    .catalog_model_id
                    .as_ref()
                    .map(AsRef::<str>::as_ref),
                Some("catalog-model")
            );
        });
    }
}
