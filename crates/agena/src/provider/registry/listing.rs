use super::{
    AppError, BTreeMap, ModeHydration, Model, ModelCapabilities, ModelMetadata, ModelRef,
    ModelSpeedMode, ModelThinkingMode, ProviderRegistry, catalog_model_id_for,
    hydrated_model_from_provider, prepare_listed_model,
};
use crate::config::{AgenaToolMode, ProviderToolsConfig};

impl ProviderRegistry {
    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<Model>, AppError> {
        let provider = self.require_provider(provider_id)?;
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
                        prepare_listed_model(provider.as_ref(), provider_id.as_str(), model, true);
                    }
                    Ok(models)
                }
            }
        })
        .await
    }

    pub fn model_capabilities(&self, model: &ModelRef) -> Result<ModelCapabilities, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_capabilities_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_metadata(&self, model: &ModelRef) -> Result<ModelMetadata, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_metadata_for_adapter(adapter_id, model_id)
        })
    }

    pub fn provider_tools_config(&self, model: &ModelRef) -> Result<ProviderToolsConfig, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.provider_tools_config_for_adapter(adapter_id, model_id)
        })
    }

    pub fn agena_tool_mode(&self, model: &ModelRef) -> Result<AgenaToolMode, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.agena_tool_mode_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_thinking_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelThinkingMode>, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_thinking_modes_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_speed_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelSpeedMode>, AppError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_speed_modes_for_adapter(adapter_id, model_id)
        })
    }

    pub async fn resolve_model(&self, model: &ModelRef) -> Result<Model, AppError> {
        let provider = self.provider_for_model_ref(model)?;

        let listed = self.list_models(model.provider_id.as_ref()).await?;
        if let Some(entry) = listed.into_iter().find(|entry| {
            entry.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .map(|adapter_id| entry.adapter_id.as_ref() == Some(adapter_id))
                    .unwrap_or(true)
        }) {
            return Ok(hydrated_model_from_provider(
                provider.as_ref(),
                entry,
                model.adapter_id.as_ref(),
                ModeHydration::OverrideIfPresent,
            ));
        }

        Ok(hydrated_model_from_provider(
            provider.as_ref(),
            Model {
                provider_id: model.provider_id.clone(),
                adapter_id: model.adapter_id.clone(),
                id: model.model_id.clone(),
                catalog_model_id: catalog_model_id_for(&model.model_id),
                display_name: None,
                capabilities: ModelCapabilities::default(),
                metadata: ModelMetadata::default(),
                thinking_modes: std::collections::BTreeMap::new(),
                speed_modes: std::collections::BTreeMap::new(),
            },
            model.adapter_id.as_ref(),
            ModeHydration::FillEmpty,
        ))
    }
}
