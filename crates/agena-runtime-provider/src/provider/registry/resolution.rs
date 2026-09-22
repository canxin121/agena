use super::{AdapterId, ModelId, ModelRef, ProviderError, ProviderId, ProviderRegistry};

impl ProviderRegistry {
    /// Resolve the actual model adapter without discovery or a network call.
    pub fn tool_execution_adapter(
        &self,
        selection: &agena_domain::ExecutionSelection,
    ) -> Result<Option<AdapterId>, ProviderError> {
        let Some(model) = self.resolve_default_model_selection(selection)? else {
            return Ok(None);
        };
        self.use_model_ref_provider(&model, |provider, adapter_id, model_id| {
            provider.resolved_execution_adapter(adapter_id, model_id)
        })?
    }

    pub fn resolve_default_model_selection(
        &self,
        selection: &agena_domain::ExecutionSelection,
    ) -> Result<Option<ModelRef>, ProviderError> {
        let Some(provider_id) = selection.provider.as_deref() else {
            return Ok(None);
        };

        self.resolve_model_selection(
            provider_id,
            selection.adapter.as_deref(),
            selection.model.as_deref(),
        )
        .map(Some)
    }

    pub fn supports_prompt_continuation(&self, model: &ModelRef) -> Result<bool, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.supports_prompt_continuation_for_adapter(adapter_id, model_id)
        })
    }

    pub fn native_compaction_enabled(&self, model: &ModelRef) -> Result<bool, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.native_compaction_enabled_for_adapter(adapter_id, model_id)
        })
    }

    pub fn prompt_cache_shape(
        &self,
        model: &ModelRef,
    ) -> Result<Option<agena_provider::PromptCacheShape>, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            provider.prompt_cache_shape_for_adapter(adapter_id, model_id)
        })
    }

    pub fn image_capabilities(
        &self,
        model: &ModelRef,
    ) -> Result<Option<agena_provider::ProviderImageCapabilities>, ProviderError> {
        self.use_model_ref_provider(model, |provider, adapter_id, model_id| {
            let route = provider
                .provider_native_tools_config_for_adapter(adapter_id, model_id)
                .routes
                .route_for(agena_provider::ProviderNativeToolKind::ImageGeneration);
            if route != Some(agena_provider::ProviderNativeToolRoute::ProviderHosted) {
                return None;
            }
            provider.image_capabilities_for_adapter(adapter_id, model_id)
        })
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, ProviderError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(ProviderError::Config(
                "provider or model reference cannot be empty".to_owned(),
            ));
        }

        let requested_model = model.map(str::trim).filter(|value| !value.is_empty());
        if target.contains('/') {
            if requested_model.is_some() {
                return Err(ProviderError::Config(format!(
                    "model reference `{target}` already includes a model; omit `--model`"
                )));
            }
            let parsed = target.parse::<ModelRef>().map_err(|err| {
                ProviderError::Config(format!("invalid model reference `{target}`: {err}"))
            })?;
            return Ok(parsed);
        }

        self.require_provider(target)?;
        let provider_id = ProviderId::try_new(target).map_err(|err| {
            ProviderError::Config(format!("invalid provider id `{target}`: {err}"))
        })?;
        let requested_model = requested_model.ok_or_else(|| {
            ProviderError::Config(format!(
                "model is required for provider `{target}`; select a model explicitly"
            ))
        })?;
        let model_id = ModelId::try_new(requested_model).map_err(|err| {
            ProviderError::Config(format!("invalid model id `{requested_model}`: {err}"))
        })?;

        Ok(ModelRef {
            provider_id,
            adapter_id: None,
            model_id,
        })
    }

    pub fn resolve_model_selection(
        &self,
        provider_id: &str,
        adapter_id: Option<&str>,
        model_id: Option<&str>,
    ) -> Result<ModelRef, ProviderError> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err(ProviderError::Config(
                "provider id cannot be empty".to_owned(),
            ));
        }
        self.require_provider(provider_id)?;
        let provider_id = ProviderId::try_new(provider_id).map_err(|err| {
            ProviderError::Config(format!("invalid provider id `{provider_id}`: {err}"))
        })?;
        let adapter_id = match adapter_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(adapter_id) => Some(AdapterId::try_new(adapter_id).map_err(|err| {
                ProviderError::Config(format!("invalid adapter id `{adapter_id}`: {err}"))
            })?),
            None => None,
        };
        let model_id = model_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProviderError::Config(format!(
                    "model is required for provider `{provider_id}`; select a model explicitly"
                ))
            })
            .and_then(|model_id| {
                ModelId::try_new(model_id).map_err(|err| {
                    ProviderError::Config(format!("invalid model id `{model_id}`: {err}"))
                })
            })?;

        Ok(ModelRef {
            provider_id,
            adapter_id,
            model_id,
        })
    }
}

#[cfg(test)]
mod adapter_gate_tests {
    use super::*;
    use crate::provider::{
        CatalogedModelsProvider, ModelRuntime, MultiAdapterProvider,
        multi_adapter::ProviderModelRoute,
    };
    use agena_provider::{
        CompletionRequest, CompletionResponse, ConfiguredModelDefinition, ProviderModelCatalog,
    };
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    struct NoNetwork {
        model: ModelId,
    }
    #[async_trait::async_trait]
    impl ModelRuntime for NoNetwork {
        fn id(&self) -> &str {
            "adapter-fixture"
        }
        fn default_model(&self) -> &ModelId {
            &self.model
        }
        async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
            panic!("adapter gate must never perform discovery")
        }
        async fn complete(
            &self,
            _: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            panic!("adapter gate must never call a provider")
        }
    }
    fn provider() -> Arc<dyn ModelRuntime> {
        let adapters = ["openai_responses", "anthropic", "gemini"]
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    Arc::new(NoNetwork {
                        model: ModelId::new("fixture"),
                    }) as Arc<dyn ModelRuntime>,
                )
            })
            .collect();
        let route = |enabled| ProviderModelRoute {
            enabled,
            native_compaction: true,
            agena_tool_mode: Default::default(),
            provider_native_tools: Default::default(),
            definition: ConfiguredModelDefinition::default(),
        };
        Arc::new(MultiAdapterProvider::new(
            "arbitrary-provider-label",
            "openai_responses",
            "fixture",
            adapters,
            BTreeMap::from([
                (("gemini".into(), "vision".into()), route(true)),
                (("anthropic".into(), "disabled-model".into()), route(false)),
            ]),
            BTreeSet::new(),
        ))
    }
    fn selection(adapter: Option<&str>, model: &str) -> agena_domain::ExecutionSelection {
        agena_domain::ExecutionSelection {
            provider: Some("arbitrary-provider-label".into()),
            adapter: adapter.map(str::to_owned),
            model: Some(model.into()),
            ..Default::default()
        }
    }
    #[test]
    fn adapter_gate_resolution_uses_actual_default_model_route_and_explicit_adapter() {
        let mut registry = ProviderRegistry::new();
        registry.register_arc(provider());
        for (requested, model, expected) in [
            (None, "fixture", "openai_responses"),
            (None, "vision", "gemini"),
            (Some("anthropic"), "vision", "anthropic"),
        ] {
            assert_eq!(
                registry
                    .tool_execution_adapter(&selection(requested, model))
                    .unwrap()
                    .unwrap()
                    .as_ref(),
                expected
            );
        }
    }
    #[test]
    fn adapter_gate_resolution_does_not_invent_unconfigured_or_disabled_routes() {
        let mut registry = ProviderRegistry::new();
        registry.register_arc(provider());
        assert!(
            registry
                .tool_execution_adapter(&selection(Some("missing"), "fixture"))
                .is_err()
        );
        assert!(
            registry
                .tool_execution_adapter(&selection(None, "disabled-model"))
                .unwrap()
                .is_some()
        );
        assert!(
            registry
                .tool_execution_adapter(&selection(Some("anthropic"), "disabled-model"))
                .is_err()
        );
        assert!(
            registry
                .tool_execution_adapter(&agena_domain::ExecutionSelection::default())
                .unwrap()
                .is_none()
        );
        let unknown = agena_domain::ExecutionSelection {
            provider: Some("chatgpt".into()),
            model: Some("gpt-named-but-not-configured".into()),
            ..Default::default()
        };
        assert!(registry.tool_execution_adapter(&unknown).is_err());
    }
    #[test]
    fn adapter_gate_resolution_survives_catalogue_wrapping() {
        let catalog = ProviderModelCatalog {
            models: BTreeMap::from([("vision".into(), ConfiguredModelDefinition::default())]),
            ..Default::default()
        };
        let mut registry = ProviderRegistry::new();
        registry.register_arc(CatalogedModelsProvider::new(provider(), catalog));
        assert_eq!(
            registry
                .tool_execution_adapter(&selection(None, "vision"))
                .unwrap()
                .unwrap()
                .as_ref(),
            "gemini"
        );
        assert_eq!(
            registry
                .tool_execution_adapter(&selection(Some("anthropic"), "vision"))
                .unwrap()
                .unwrap()
                .as_ref(),
            "anthropic"
        );
    }
    #[test]
    fn adapter_gate_resolution_leaves_a_runtime_without_known_adapter_unresolved() {
        let mut registry = ProviderRegistry::new();
        registry.register_arc(Arc::new(NoNetwork {
            model: ModelId::new("claude-sounding-name"),
        }));
        let selection = agena_domain::ExecutionSelection {
            provider: Some("adapter-fixture".into()),
            model: Some("claude-sounding-name".into()),
            ..Default::default()
        };
        assert!(
            registry
                .tool_execution_adapter(&selection)
                .unwrap()
                .is_none()
        );
    }
}
