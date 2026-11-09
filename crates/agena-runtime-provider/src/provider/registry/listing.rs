use super::{
    BTreeMap, Model, ModelCapabilities, ModelMetadata, ModelRef, ModelSpeedMode, ModelThinkingMode,
    ProviderError, ProviderRegistry, prepare_listed_model,
};
use agena_provider::{AgenaToolMode, ProviderModelSource};

impl ProviderRegistry {
    pub async fn discover_models(
        &self,
        provider_id: &str,
    ) -> Result<agena_provider::ProviderModelDiscovery, ProviderError> {
        let provider = self.require_provider(provider_id)?;
        let mut discovery = provider.discover_models().await?;
        for entry in &mut discovery.models {
            prepare_listed_model(provider.as_ref(), provider_id, &mut entry.model, true);
        }
        discovery.models.sort_by(|a, b| {
            (&a.model.adapter_id, &a.model.id).cmp(&(&b.model.adapter_id, &b.model.id))
        });
        discovery
            .models
            .dedup_by(|a, b| a.model.adapter_id == b.model.adapter_id && a.model.id == b.model.id);
        self.remember_model_inventory(
            provider_id,
            discovery
                .models
                .iter()
                .map(|entry| entry.model.clone())
                .collect(),
        );
        Ok(discovery)
    }

    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<Model>, ProviderError> {
        let provider = self.require_provider(provider_id)?;
        let provider_id = provider_id.to_owned();
        let models = self
            .call_with_retry(provider_id.as_str(), "list_models", {
                let provider = provider.clone();
                let provider_id = provider_id.clone();
                move || {
                    let provider = provider.clone();
                    let provider_id = provider_id.clone();
                    async move {
                        let mut models = provider.list_models().await?;
                        for model in &mut models {
                            prepare_listed_model(
                                provider.as_ref(),
                                provider_id.as_str(),
                                model,
                                true,
                            );
                        }
                        Ok(models)
                    }
                }
            })
            .await?;
        self.remember_model_inventory(&provider_id, models.clone());
        Ok(models)
    }

    fn remember_model_inventory(&self, provider_id: &str, models: Vec<Model>) {
        self.inventory
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(provider_id.into(), models);
    }

    /// Native configured routes plus the last successfully discovered inventory.
    /// Listing choices never performs network discovery and never appends a
    /// model merely because it appears in a public catalog or default string.
    pub fn selection_model_inventory(&self) -> Vec<Model> {
        let cached = self.inventory.lock().unwrap_or_else(|p| p.into_inner());
        let mut models = Vec::new();
        for (id, provider) in &self.providers {
            if let Some(entries) = cached.get(id) {
                models.extend(entries.clone());
            }
            for mut model in provider.configured_models() {
                prepare_listed_model(provider.as_ref(), id, &mut model, true);
                models.retain(|entry| entry.reference() != model.reference());
                models.push(model);
            }
        }
        models.sort_by_key(|model| model.reference().to_string());
        models.dedup_by(|a, b| a.reference() == b.reference());
        models
    }

    pub fn model_capabilities(&self, model: &ModelRef) -> Result<ModelCapabilities, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_capabilities_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_metadata(&self, model: &ModelRef) -> Result<ModelMetadata, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_metadata_for_adapter(adapter_id, model_id)
        })
    }

    pub fn agena_tool_mode(&self, model: &ModelRef) -> Result<AgenaToolMode, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.agena_tool_mode_for_adapter(adapter_id, model_id)
        })
    }

    pub fn model_thinking_modes(
        &self,
        model: &ModelRef,
    ) -> Result<Vec<ModelThinkingMode>, ProviderError> {
        let mut modes = self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_thinking_modes_for_adapter(adapter_id, model_id)
        })?;
        modes.sort_by(agena_domain::compare_thinking_mode_strength);
        Ok(modes)
    }

    pub fn model_speed_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelSpeedMode>, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.model_speed_modes_for_adapter(adapter_id, model_id)
        })
    }
}

#[async_trait::async_trait]
impl ProviderModelSource for ProviderRegistry {
    fn provider_ids(&self) -> Vec<agena_domain::ProviderId> {
        ProviderRegistry::provider_ids(self)
            .into_iter()
            .map(agena_domain::ProviderId::new)
            .collect()
    }

    async fn list_models(
        &self,
        provider_id: &agena_domain::ProviderId,
    ) -> Result<Vec<Model>, agena_provider::ProviderCatalogError> {
        ProviderRegistry::list_models(self, provider_id.as_ref())
            .await
            .map_err(|error| agena_provider::ProviderCatalogError::operation_error(&error))
    }
}
