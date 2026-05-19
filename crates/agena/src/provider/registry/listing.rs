use super::*;

impl ProviderRegistry {
    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<Model>, AppError> {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
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
                        model.provider_id = ProviderId::new(provider_id.clone());
                        assign_catalog_model_id(model);
                        let fallback = provider
                            .model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.id);
                        let current_capabilities = std::mem::take(&mut model.capabilities);
                        model.capabilities = if current_capabilities.is_default_placeholder() {
                            fallback.clone()
                        } else {
                            current_capabilities.with_fallbacks_from(&fallback)
                        };
                        let metadata_fallback = provider
                            .model_metadata_for_adapter(model.adapter_id.as_ref(), &model.id);
                        model.metadata = model
                            .metadata
                            .clone()
                            .with_fallbacks_from(&metadata_fallback);
                        if model.thinking_modes.is_empty() {
                            model.thinking_modes = provider.model_thinking_modes_for_adapter(
                                model.adapter_id.as_ref(),
                                &model.id,
                            );
                        }
                        if model.speed_modes.is_empty() {
                            model.speed_modes = provider.model_speed_modes_for_adapter(
                                model.adapter_id.as_ref(),
                                &model.id,
                            );
                        }
                    }
                    Ok(models)
                }
            }
        })
        .await
    }

    pub fn model_capabilities(&self, model: &ModelRef) -> Result<ModelCapabilities, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub fn model_metadata(&self, model: &ModelRef) -> Result<ModelMetadata, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_metadata_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub fn model_thinking_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelThinkingMode>, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_thinking_modes_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub fn model_speed_modes(
        &self,
        model: &ModelRef,
    ) -> Result<BTreeMap<String, ModelSpeedMode>, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.model_speed_modes_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub async fn resolve_model(&self, model: &ModelRef) -> Result<Model, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;

        let listed = self.list_models(model.provider_id.as_str()).await?;
        if let Some(entry) = listed.into_iter().find(|entry| {
            entry.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .map(|adapter_id| entry.adapter_id.as_ref() == Some(adapter_id))
                    .unwrap_or(true)
        }) {
            let adapter_id = model.adapter_id.as_ref().or(entry.adapter_id.as_ref());
            let fallback = provider.model_capabilities_for_adapter(adapter_id, &entry.id);
            let metadata_fallback = provider.model_metadata_for_adapter(adapter_id, &entry.id);
            let thinking_modes = provider.model_thinking_modes_for_adapter(adapter_id, &entry.id);
            let speed_modes = provider.model_speed_modes_for_adapter(adapter_id, &entry.id);
            let mut resolved = entry
                .with_capability_fallbacks(&fallback)
                .with_metadata_fallbacks(&metadata_fallback);
            if !thinking_modes.is_empty() {
                resolved.thinking_modes = thinking_modes;
            }
            if !speed_modes.is_empty() {
                resolved.speed_modes = speed_modes;
            }
            return Ok(resolved);
        }

        Ok({
            let mut fallback_model =
                Model::new(model.provider_id.as_str(), model.model_id.as_str());
            fallback_model.adapter_id = model.adapter_id.clone();
            assign_catalog_model_id(&mut fallback_model);
            fallback_model
        }
        .with_capabilities(
            provider.model_capabilities_for_adapter(model.adapter_id.as_ref(), &model.model_id),
        )
        .with_metadata(
            provider.model_metadata_for_adapter(model.adapter_id.as_ref(), &model.model_id),
        )
        .with_thinking_modes(
            provider.model_thinking_modes_for_adapter(model.adapter_id.as_ref(), &model.model_id),
        )
        .with_speed_modes(
            provider.model_speed_modes_for_adapter(model.adapter_id.as_ref(), &model.model_id),
        ))
    }
}
