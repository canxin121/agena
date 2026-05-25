use super::*;
use crate::config::ProviderNativeToolsConfig;

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
        self.with_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_capabilities_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_metadata(&self, model: &ModelRef) -> Result<ModelMetadata, AppError> {
        self.with_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_metadata_for_adapter(adapter_id, model_id)
        })
    }

    pub fn native_tools_config(
        &self,
        model: &ModelRef,
    ) -> Result<ProviderNativeToolsConfig, AppError> {
        self.with_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.native_tools_config_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_thinking_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelThinkingMode>, AppError> {
        self.with_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_thinking_modes_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_speed_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelSpeedMode>, AppError> {
        self.with_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_speed_modes_for_adapter(adapter_id, model_id)
        })
    }

    pub async fn resolve_model(&self, model: &ModelRef) -> Result<Model, AppError> {
        let provider = self.provider_for_model_ref(model)?;

        let listed = self.list_models(model.provider_id.as_str()).await?;
        if let Some(entry) = listed.into_iter().find(|entry| {
            entry.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .map(|adapter_id| entry.adapter_id.as_ref() == Some(adapter_id))
                    .unwrap_or(true)
        }) {
            let mut resolved = entry;
            hydrate_model_from_provider(
                provider.as_ref(),
                &mut resolved,
                model.adapter_id.as_ref(),
                ModeHydration::OverrideIfPresent,
            );
            return Ok(resolved);
        }

        let mut fallback_model = {
            let mut fallback_model =
                Model::new(model.provider_id.as_str(), model.model_id.as_str());
            fallback_model.adapter_id = model.adapter_id.clone();
            assign_catalog_model_id(&mut fallback_model);
            fallback_model
        };
        hydrate_model_from_provider(
            provider.as_ref(),
            &mut fallback_model,
            model.adapter_id.as_ref(),
            ModeHydration::FillEmpty,
        );
        Ok(fallback_model)
    }
}
