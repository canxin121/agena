use super::*;

impl ProviderRegistry {
    pub fn resolve_default_model_selection(
        &self,
        selection: &crate::execution_prefs::ExecutionSelection,
    ) -> Result<Option<ModelRef>, AppError> {
        let Some(provider_id) = selection.provider.as_deref() else {
            return self.resolve_first_provider_default_model();
        };

        self.resolve_model_selection(
            provider_id,
            selection.adapter.as_deref(),
            selection.model.as_deref(),
        )
        .map(Some)
    }

    pub fn resolve_first_provider_default_model(&self) -> Result<Option<ModelRef>, AppError> {
        let mut providers = self.provider_ids();
        providers.sort();
        let Some(provider_id) = providers.first() else {
            return Ok(None);
        };
        self.resolve_model_target(provider_id, None).map(Some)
    }

    pub fn supports_prompt_continuation(&self, model: &ModelRef) -> Result<bool, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider
            .supports_prompt_continuation_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub fn prompt_cache_shape_fingerprint(
        &self,
        model: &ModelRef,
    ) -> Result<Option<String>, AppError> {
        Ok(self
            .prompt_cache_shape(model)?
            .map(|shape| shape.fingerprint()))
    }

    pub fn prompt_cache_shape(
        &self,
        model: &ModelRef,
    ) -> Result<Option<crate::provider::PromptCacheShape>, AppError> {
        let provider = self.get(model.provider_id.as_str()).ok_or_else(|| {
            AppError::Config(format!("provider not found: {}", model.provider_id))
        })?;
        Ok(provider.prompt_cache_shape_for_adapter(model.adapter_id.as_ref(), &model.model_id))
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(AppError::Config(
                "provider or model reference cannot be empty".to_owned(),
            ));
        }

        let requested_model = model.map(str::trim).filter(|value| !value.is_empty());
        if target.contains('/') {
            if requested_model.is_some() {
                return Err(AppError::Config(format!(
                    "model reference `{target}` already includes a model; omit `--model`"
                )));
            }
            let mut parsed = target.parse::<ModelRef>().map_err(|err| {
                AppError::Config(format!("invalid model reference `{target}`: {err}"))
            })?;
            if parsed.adapter_id.is_none()
                && let Some(provider) = self.get(parsed.provider_id.as_str())
            {
                parsed.adapter_id = provider.default_adapter().cloned();
            }
            return Ok(parsed);
        }

        let provider = self
            .get(target)
            .ok_or_else(|| AppError::Config(format!("provider not found: {target}")))?;
        let provider_id = ProviderId::try_new(target)
            .map_err(|err| AppError::Config(format!("invalid provider id `{target}`: {err}")))?;
        let model_id = match requested_model {
            Some(requested_model) => ModelId::try_new(requested_model).map_err(|err| {
                AppError::Config(format!("invalid model id `{requested_model}`: {err}"))
            })?,
            None => provider.default_model().clone(),
        };

        Ok(ModelRef {
            provider_id,
            adapter_id: provider.default_adapter().cloned(),
            model_id,
        })
    }

    pub fn resolve_model_selection(
        &self,
        provider_id: &str,
        adapter_id: Option<&str>,
        model_id: Option<&str>,
    ) -> Result<ModelRef, AppError> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err(AppError::Config("provider id cannot be empty".to_owned()));
        }
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        let provider_id = ProviderId::try_new(provider_id).map_err(|err| {
            AppError::Config(format!("invalid provider id `{provider_id}`: {err}"))
        })?;
        let adapter_id = match adapter_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(adapter_id) => Some(AdapterId::try_new(adapter_id).map_err(|err| {
                AppError::Config(format!("invalid adapter id `{adapter_id}`: {err}"))
            })?),
            None => provider.default_adapter().cloned(),
        };
        let model_id = match model_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(model_id) => ModelId::try_new(model_id)
                .map_err(|err| AppError::Config(format!("invalid model id `{model_id}`: {err}")))?,
            None => provider.default_model().clone(),
        };

        Ok(ModelRef {
            provider_id,
            adapter_id,
            model_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::model::{AdapterId, Model, ModelId};
    use crate::provider::{CompletionRequest, CompletionResponse, ModelRuntime};

    struct StaticProvider {
        id: String,
        default_model: ModelId,
        default_adapter: Option<AdapterId>,
    }

    impl StaticProvider {
        fn new(
            id: impl Into<String>,
            default_model: impl Into<String>,
            default_adapter: Option<&str>,
        ) -> Self {
            Self {
                id: id.into(),
                default_model: ModelId::new(default_model),
                default_adapter: default_adapter.map(AdapterId::new),
            }
        }
    }

    #[async_trait]
    impl ModelRuntime for StaticProvider {
        fn id(&self) -> &str {
            self.id.as_str()
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        fn default_adapter(&self) -> Option<&AdapterId> {
            self.default_adapter.as_ref()
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            let mut model = Model::new(self.id.as_str(), self.default_model.to_string());
            if let Some(adapter_id) = self.default_adapter.as_ref() {
                model = model.with_adapter_id(adapter_id.as_str());
            }
            Ok(vec![model])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Internal(
                "complete is not used in this test".to_owned(),
            ))
        }
    }

    #[test]
    fn resolve_first_provider_default_model_uses_sorted_provider_id() {
        let mut registry = ProviderRegistry::new();
        registry.register(StaticProvider::new("zeta", "zeta-default", None));
        registry.register(StaticProvider::new("alpha", "alpha-default", Some("chat")));

        let resolved = registry
            .resolve_first_provider_default_model()
            .expect("resolve first provider default model")
            .expect("model should exist");

        assert_eq!(resolved.provider_id.as_str(), "alpha");
        assert_eq!(
            resolved.adapter_id.as_ref().map(|value| value.as_str()),
            Some("chat")
        );
        assert_eq!(resolved.model_id.as_str(), "alpha-default");
    }

    #[test]
    fn resolve_default_model_selection_prefers_explicit_provider_selection() {
        let mut registry = ProviderRegistry::new();
        registry.register(StaticProvider::new("alpha", "alpha-default", Some("chat")));
        registry.register(StaticProvider::new("zeta", "zeta-default", None));

        let selection = crate::execution_prefs::ExecutionSelection {
            provider: Some("zeta".to_owned()),
            adapter: None,
            model: Some("custom-zeta".to_owned()),
            ..crate::execution_prefs::ExecutionSelection::default()
        };

        let resolved = registry
            .resolve_default_model_selection(&selection)
            .expect("resolve selected model")
            .expect("model should exist");

        assert_eq!(resolved.provider_id.as_str(), "zeta");
        assert_eq!(resolved.adapter_id, None);
        assert_eq!(resolved.model_id.as_str(), "custom-zeta");
    }
}
