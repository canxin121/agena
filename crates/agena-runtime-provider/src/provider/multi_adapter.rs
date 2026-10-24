use agena_domain::*;
use agena_provider::{
    CompletionStreamEvent, ConfiguredModelDefinition, PromptCacheShape, ProviderCompactionOutput,
    ProviderModelRouteKey, StreamResumePolicy,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{StreamExt, stream::BoxStream};

use crate::ProviderError;

use super::core::{
    impl_model_runtime_base_via_adapter_methods, remap_stream_event_provider_and_model,
};
use super::{CompletionResponse, ModelRuntime, tool_mode};
use agena_provider::CompletionRequest;
use agena_provider::ProviderNativeToolsConfig;
use agena_provider::{
    AgenaToolMode, ProviderHostedImageGenerationConfig, ProviderImageCapabilities,
    ProviderImageRequest, ProviderImageResponse, ProviderNativeToolKind, ProviderNativeToolRoute,
};

#[derive(Debug, Clone)]
/// Route of a provider model.
pub struct ProviderModelRoute {
    pub enabled: bool,
    pub native_compaction: bool,
    /// `None` inherits tool support instead of declaring an explicit mode.
    pub agena_tool_mode: Option<AgenaToolMode>,
    pub provider_native_tools: ProviderNativeToolsConfig,
    pub definition: ConfiguredModelDefinition,
}

#[derive(Clone)]
/// Provider dispatching across multiple adapters.
pub struct MultiAdapterProvider {
    id: String,
    default_adapter: AdapterId,
    default_model: ModelId,
    adapters: BTreeMap<String, Arc<dyn ModelRuntime>>,
    routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
    configured_only_adapters: BTreeSet<String>,
}

impl MultiAdapterProvider {
    pub fn new(
        id: impl Into<String>,
        default_adapter: impl Into<String>,
        default_model: impl Into<String>,
        adapters: BTreeMap<String, Arc<dyn ModelRuntime>>,
        routes: BTreeMap<ProviderModelRouteKey, ProviderModelRoute>,
        configured_only_adapters: BTreeSet<String>,
    ) -> Self {
        Self {
            id: id.into(),
            default_adapter: AdapterId::new(default_adapter),
            default_model: ModelId::new(default_model),
            adapters,
            routes,
            configured_only_adapters,
        }
    }

    fn adapter(&self, adapter_id: &str) -> Result<Arc<dyn ModelRuntime>, ProviderError> {
        self.adapters.get(adapter_id).cloned().ok_or_else(|| {
            ProviderError::Config(format!(
                "provider `{}` has no enabled adapter `{adapter_id}`",
                self.id
            ))
        })
    }

    fn selected_adapter(&self, adapter_id: Option<&AdapterId>, model: &ModelId) -> AdapterId {
        adapter_id
            .cloned()
            .or_else(|| {
                self.routes
                    .iter()
                    .find(|((_, route_model), route)| {
                        route.enabled && route_model.as_str() == model.as_ref()
                    })
                    .map(|((adapter_id, _), _)| AdapterId::new(adapter_id.clone()))
            })
            .unwrap_or_else(|| self.default_adapter.clone())
    }

    fn resolve_route(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Result<
        (
            AdapterId,
            ModelId,
            AgenaToolMode,
            ProviderNativeToolsConfig,
            ConfiguredModelDefinition,
        ),
        ProviderError,
    > {
        let adapter_id = self.selected_adapter(adapter_id, model);
        let target_model = model.clone();
        let key = (adapter_id.to_string(), target_model.to_string());
        if let Some(route) = self.routes.get(&key) {
            if !route.enabled {
                return Err(ProviderError::Config(format!(
                    "provider `{}` adapter `{adapter_id}` model `{}` is disabled",
                    self.id, model
                )));
            }
            let agena_tool_mode = self
                .agena_tool_mode_override_for_adapter(Some(&adapter_id), model)
                .unwrap_or(
                    self.adapter(adapter_id.as_ref())?
                        .agena_tool_mode(&target_model),
                );
            return Ok((
                adapter_id,
                target_model,
                agena_tool_mode,
                route.provider_native_tools.clone(),
                route.definition.clone(),
            ));
        }

        let adapter_mode = self
            .adapter(adapter_id.as_ref())?
            .agena_tool_mode(&target_model);
        Ok((
            adapter_id,
            target_model,
            adapter_mode,
            ProviderNativeToolsConfig::default(),
            ConfiguredModelDefinition::default(),
        ))
    }

    fn resolve_route_and_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Result<
        (
            AdapterId,
            ModelId,
            AgenaToolMode,
            ProviderNativeToolsConfig,
            ConfiguredModelDefinition,
            Arc<dyn ModelRuntime>,
        ),
        ProviderError,
    > {
        let (adapter_id, target_model, agena_tool_mode, provider_native_tools, definition) =
            self.resolve_route(adapter_id, model)?;
        let adapter = self.adapter(adapter_id.as_ref())?;
        Ok((
            adapter_id,
            target_model,
            agena_tool_mode,
            provider_native_tools,
            definition,
            adapter,
        ))
    }

    fn map_route_and_adapter<T>(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
        map: impl FnOnce(
            &AdapterId,
            &ModelId,
            AgenaToolMode,
            &ProviderNativeToolsConfig,
            &ConfiguredModelDefinition,
            &dyn ModelRuntime,
        ) -> T,
    ) -> Option<T> {
        let (adapter_id, target_model, agena_tool_mode, provider_native_tools, definition, adapter) =
            match self.resolve_route_and_adapter(adapter_id, model) {
                Ok(route) => route,
                Err(error) => {
                    tracing::warn!(
                        provider_id = %self.id,
                        requested_adapter = ?adapter_id,
                        requested_model = %model,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "resolve a provider route for model metadata projection",
                            &error,
                        ),
                        "provider model metadata fell back because its configured route could not be resolved"
                    );
                    return None;
                }
            };
        Some(map(
            &adapter_id,
            &target_model,
            agena_tool_mode,
            &provider_native_tools,
            &definition,
            adapter.as_ref(),
        ))
    }

    fn rewrite_model(
        &self,
        target_model: &ModelId,
        mut model: Model,
        adapter_id: &AdapterId,
        adapter: &dyn ModelRuntime,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        model.provider_id = ProviderId::new(self.id.clone());
        model.adapter_id = Some(adapter_id.clone());
        model.id = target_model.clone();
        model.native_compaction =
            self.native_compaction_enabled_for_adapter(Some(adapter_id), target_model);
        definition.apply_to_model(
            model,
            &adapter.model_capabilities(target_model),
            &adapter.model_metadata(target_model),
        )
    }

    fn synthesize_model(
        &self,
        target_model: &ModelId,
        adapter_id: &AdapterId,
        adapter: &dyn ModelRuntime,
        definition: &ConfiguredModelDefinition,
    ) -> Model {
        let model = Model {
            provider_id: ProviderId::new(self.id.as_str()),
            adapter_id: Some(adapter_id.clone()),
            id: target_model.clone(),
            catalog_model_id: None,
            display_name: None,
            native_compaction: self
                .native_compaction_enabled_for_adapter(Some(adapter_id), target_model),
            capabilities: ModelCapabilities::default(),
            metadata: ModelMetadata::default(),
            thinking_modes: Vec::new(),
            speed_modes: BTreeMap::new(),
        };
        definition.apply_to_model(
            model,
            &adapter.model_capabilities(target_model),
            &adapter.model_metadata(target_model),
        )
    }
}

#[async_trait]
impl ModelRuntime for MultiAdapterProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn default_adapter(&self) -> Option<&AdapterId> {
        Some(&self.default_adapter)
    }

    fn resolved_execution_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Result<Option<AdapterId>, ProviderError> {
        let (selected, ..) = self.resolve_route_and_adapter(adapter_id, model)?;
        Ok(Some(selected))
    }

    impl_model_runtime_base_via_adapter_methods! {
        fn model_capabilities / model_capabilities_for_adapter (&self, model: &ModelId) -> ModelCapabilities;
        fn model_metadata / model_metadata_for_adapter (&self, model: &ModelId) -> ModelMetadata;
        fn model_thinking_modes / model_thinking_modes_for_adapter (&self, model: &ModelId) -> Vec<ModelThinkingMode>;
        fn model_speed_modes / model_speed_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode>;
        fn supports_prompt_continuation / supports_prompt_continuation_for_adapter (&self, model: &ModelId) -> bool;
        fn native_compaction_enabled / native_compaction_enabled_for_adapter (&self, model: &ModelId) -> bool;
        fn prompt_cache_shape / prompt_cache_shape_for_adapter (&self, model: &ModelId) -> Option<PromptCacheShape>;
        fn provider_native_tools_config / provider_native_tools_config_for_adapter (&self, model: &ModelId) -> ProviderNativeToolsConfig;
        fn agena_tool_mode / agena_tool_mode_for_adapter (&self, model: &ModelId) -> AgenaToolMode;
        fn agena_tool_mode_override / agena_tool_mode_override_for_adapter (&self, model: &ModelId) -> Option<AgenaToolMode>;
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             _provider_native_tools,
             definition,
             adapter| {
                definition
                    .capabilities
                    .apply_to(adapter.model_capabilities(target_model))
            },
        )
        .unwrap_or_default()
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             _provider_native_tools,
             definition,
             adapter| {
                definition
                    .metadata()
                    .merged_with_fallbacks_from(&adapter.model_metadata(target_model))
            },
        )
        .unwrap_or_default()
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Vec<ModelThinkingMode> {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             _provider_native_tools,
             definition,
             adapter| {
                agena_provider::apply_configured_thinking_modes(
                    adapter.model_thinking_modes(target_model),
                    &definition.thinking_modes,
                )
            },
        )
        .unwrap_or_default()
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             _provider_native_tools,
             definition,
             adapter| {
                agena_provider::apply_configured_modes(
                    adapter.model_speed_modes(target_model),
                    definition.speed_modes.iter(),
                    |configured, existing| configured.apply_to_mode(existing),
                )
            },
        )
        .unwrap_or_default()
    }

    fn provider_native_tools_config_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ProviderNativeToolsConfig {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             _target_model,
             _agena_tool_mode,
             provider_native_tools,
             _definition,
             _adapter| { provider_native_tools.clone() },
        )
        .unwrap_or_default()
    }

    fn agena_tool_mode_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> AgenaToolMode {
        self.resolve_route(adapter_id, model)
            .map(|(_, _, mode, _, _)| mode)
            .unwrap_or_else(|_| {
                self.agena_tool_mode_override_for_adapter(adapter_id, model)
                    .unwrap_or_default()
            })
    }

    fn agena_tool_mode_override_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<AgenaToolMode> {
        let adapter_id = self.selected_adapter(adapter_id, model);
        self.routes
            .get(&(adapter_id.to_string(), model.to_string()))
            .and_then(|route| {
                route.agena_tool_mode.or_else(|| {
                    route
                        .definition
                        .capabilities
                        .feature_support(agena_provider::ModelCapabilityFeature::ToolCalling)
                        .map(|support| AgenaToolMode::from_tool_calling_support(Some(support)))
                })
            })
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        (self.adapters.len() == 1)
            .then(|| self.adapters.values().next().cloned())
            .flatten()
            .map(|adapter| adapter.stream_resume_policy())
            .unwrap_or(StreamResumePolicy::Disabled)
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             _provider_native_tools,
             _definition,
             adapter| { adapter.supports_prompt_continuation(target_model) },
        )
        .unwrap_or(false)
    }

    fn native_compaction_enabled_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        let adapter_id = self.selected_adapter(adapter_id, model);
        self.routes
            .get(&(adapter_id.to_string(), model.to_string()))
            .map(|route| route.native_compaction)
            .unwrap_or(true)
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             agena_tool_mode,
             _provider_native_tools,
             _definition,
             adapter| {
                let base = adapter.prompt_cache_shape(target_model);
                let mut shape = base.unwrap_or_else(|| PromptCacheShape::new(self.id.as_str()));
                shape.insert_string("agena.tools.mode", agena_tool_mode.as_str());
                Some(shape)
            },
        )
        .flatten()
    }

    fn validate_media_inputs_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
        attachments: &[agena_domain::AttachmentItem],
    ) -> Result<(), ProviderError> {
        let (_, target_model, _, _, _, adapter) =
            self.resolve_route_and_adapter(adapter_id, model)?;
        adapter.validate_media_inputs_for_adapter(None, &target_model, attachments)
    }

    fn validate_provider_native_tools_request(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), ProviderError> {
        if request.provider_native_tools.bindings().is_empty() {
            return Ok(());
        }
        let (
            _adapter_id,
            target_model,
            agena_tool_mode,
            _provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &request.model)?;
        if !agena_tool_mode.is_provider_protocol() {
            return Err(ProviderError::Config(format!(
                "provider model `{}` cannot use provider-native tools while agena_tools.mode is `{}`",
                request.model,
                agena_tool_mode.as_str(),
            )));
        }
        let mut delegated = request.clone();
        delegated.model = target_model;
        adapter.validate_provider_native_tools_request(None, &delegated)
    }

    fn image_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<ProviderImageCapabilities> {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_mode,
             provider_native_tools,
             _definition,
             adapter| {
                if provider_native_tools
                    .routes
                    .route_for(ProviderNativeToolKind::ImageGeneration)
                    != Some(ProviderNativeToolRoute::ProviderHosted)
                {
                    return None;
                }
                adapter.image_capabilities_for_adapter(None, target_model)
            },
        )
        .flatten()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        let mut visible = Vec::new();
        let mut seen = BTreeSet::new();
        let mut errors = Vec::new();

        for (adapter_id, adapter) in &self.adapters {
            let adapter_id = AdapterId::new(adapter_id.clone());
            let listed = if self.configured_only_adapters.contains(adapter_id.as_ref()) {
                Vec::new()
            } else {
                match adapter.list_models().await {
                    Ok(models) => models,
                    Err(error) => {
                        errors.push(format!("{adapter_id}: {error}"));
                        Vec::new()
                    }
                }
            };
            for model in listed {
                let target_model = model.id.clone();
                let route_key = (adapter_id.to_string(), target_model.to_string());
                let route = self.routes.get(&route_key);
                if matches!(route, Some(route) if !route.enabled) {
                    continue;
                }
                let definition = route
                    .map(|route| route.definition.clone())
                    .unwrap_or_default();
                seen.insert(route_key);
                visible.push(self.rewrite_model(
                    &target_model,
                    model,
                    &adapter_id,
                    adapter.as_ref(),
                    &definition,
                ));
            }
        }

        for ((adapter_id, target_model), route) in &self.routes {
            if !route.enabled || seen.contains(&(adapter_id.clone(), target_model.clone())) {
                continue;
            }

            let adapter_id = AdapterId::new(adapter_id.clone());
            let target_model = ModelId::new(target_model.clone());
            let adapter = self.adapter(adapter_id.as_ref())?;
            visible.push(self.synthesize_model(
                &target_model,
                &adapter_id,
                adapter.as_ref(),
                &route.definition,
            ));
        }

        if visible.is_empty() && !errors.is_empty() {
            return Err(ProviderError::Provider(format!(
                "adapter model discovery failed: {}",
                errors.join("; ")
            )));
        }

        Ok(visible)
    }

    async fn execute_image_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
        mut request: ProviderImageRequest,
    ) -> Result<ProviderImageResponse, ProviderError> {
        let visible_model = model.clone();
        let (
            _adapter_id,
            target_model,
            _agena_tool_mode,
            provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if provider_native_tools
            .routes
            .route_for(ProviderNativeToolKind::ImageGeneration)
            != Some(ProviderNativeToolRoute::ProviderHosted)
        {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{visible_model}` does not enable the provider-hosted image_generation route",
                self.id
            )));
        }
        let capabilities = adapter
            .image_capabilities_for_adapter(None, &target_model)
            .ok_or_else(|| {
                ProviderError::Config(format!(
                    "provider `{}` model `{visible_model}` enables image_generation, but adapter `{}` has no direct image runtime port",
                    self.id,
                    self.selected_adapter(adapter_id, &visible_model)
                ))
            })?;
        if !capabilities.supports(request.operation) {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{visible_model}` does not support the requested direct image operation",
                self.id
            )));
        }
        request.options = merge_image_options(
            provider_native_tools.hosted.image_generation,
            request.options,
        );
        let mut response = adapter.execute_image(&target_model, request).await?;
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.complete_for_adapter(None, request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let visible_model = request.model.clone();
        let (
            _adapter_id,
            target_model,
            agena_tool_mode,
            _provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        request.model = target_model;
        let mut response = adapter.complete(request).await?;
        if agena_tool_mode.is_disabled() {
            agena_provider::validate_disabled_tool_response(
                self.id.as_str(),
                &visible_model,
                &response,
            )?;
        }
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
        self.compact_conversation_for_adapter(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
        let visible_model = request.model.clone();
        let (
            _adapter_id,
            target_model,
            _agena_tool_mode,
            _provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if !self.native_compaction_enabled_for_adapter(adapter_id, &visible_model) {
            return Ok(None);
        }
        request.model = target_model;
        adapter.compact_conversation(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        self.complete_stream_for_adapter(None, request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let visible_model = request.model.clone();
        let (
            _adapter_id,
            target_model,
            agena_tool_mode,
            _provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        request.model = target_model;
        let provider_id = self.id.clone();
        let stream = adapter.complete_stream(request.clone()).await?;
        let stream = if agena_tool_mode.is_disabled() {
            tool_mode::guard_disabled_stream(stream, provider_id.clone(), visible_model.clone())
        } else {
            stream
        };
        let stream: BoxStream<'static, Result<CompletionStreamEvent, ProviderError>> =
            Box::pin(stream.map(move |item| {
                let provider_id = ProviderId::new(provider_id.clone());
                let visible_model = visible_model.clone();
                item.map(|event| {
                    remap_stream_event_provider_and_model(&provider_id, &visible_model, event)
                })
            }));
        Ok(Box::pin(stream))
    }
}

fn merge_image_options(
    mut configured: ProviderHostedImageGenerationConfig,
    requested: ProviderHostedImageGenerationConfig,
) -> ProviderHostedImageGenerationConfig {
    if requested.background.is_some() {
        configured.background = requested.background;
    }
    if requested.size.is_some() {
        configured.size = requested.size;
    }
    if requested.quality.is_some() {
        configured.quality = requested.quality;
    }
    if requested.moderation.is_some() {
        configured.moderation = requested.moderation;
    }
    if requested.provider_options.is_some() {
        configured.provider_options = requested.provider_options;
    }
    configured
}
