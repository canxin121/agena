use std::collections::BTreeMap;

use agena_domain::{
    AdapterId, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
};

use super::ModelRuntime;

/// Runtime's concrete adapter for the provider-owned catalog decoration
/// algorithm.  Provider SDK/runtime mechanics remain here; the merge and
/// append policy itself lives in `agena-provider`.
pub(crate) struct ModelRuntimeCatalogDecorationSource<'a> {
    provider: &'a dyn ModelRuntime,
}

pub(crate) fn catalog_decoration_source(
    provider: &dyn ModelRuntime,
) -> ModelRuntimeCatalogDecorationSource<'_> {
    ModelRuntimeCatalogDecorationSource { provider }
}

impl agena_provider::CatalogModelDecorationSource for ModelRuntimeCatalogDecorationSource<'_> {
    fn provider_id(&self) -> &str {
        self.provider.id()
    }
    fn catalog_model_id_for_raw(&self, raw_model_id: &str) -> Option<String> {
        agena_provider::catalog_model_id_for_raw(raw_model_id)
    }
    fn native_compaction_enabled_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.provider
            .native_compaction_enabled_for_adapter(adapter_id, model)
    }
    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        self.provider
            .model_capabilities_for_adapter(adapter_id, model)
    }
    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        self.provider.model_metadata_for_adapter(adapter_id, model)
    }
    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        self.provider
            .model_thinking_modes_for_adapter(adapter_id, model)
    }
    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.provider
            .model_speed_modes_for_adapter(adapter_id, model)
    }
}
