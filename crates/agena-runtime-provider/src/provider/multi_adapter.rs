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
    pub agena_tool_mode: AgenaToolMode,
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

    fn selected_adapter(&self, adapter_id: Option<&AdapterId>) -> AdapterId {
        adapter_id
            .cloned()
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
        let adapter_id = self.selected_adapter(adapter_id);
        let target_model = model.clone();
        let key = (adapter_id.to_string(), target_model.to_string());
        if let Some(route) = self.routes.get(&key) {
            if !route.enabled {
                return Err(ProviderError::Config(format!(
                    "provider `{}` adapter `{adapter_id}` model `{}` is disabled",
                    self.id, model
                )));
            }
            return Ok((
                adapter_id,
                target_model,
                route.agena_tool_mode,
                route.provider_native_tools.clone(),
                route.definition.clone(),
            ));
        }

        self.adapter(adapter_id.as_ref())?;
        Ok((
            adapter_id,
            target_model,
            AgenaToolMode::default(),
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
            self.resolve_route_and_adapter(adapter_id, model).ok()?;
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
            .unwrap_or(AgenaToolMode::Disabled)
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
        let adapter_id = self.selected_adapter(adapter_id);
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
                    self.selected_adapter(adapter_id)
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
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (
            _adapter_id,
            target_model,
            agena_tool_mode,
            provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        agena_provider::apply_configured_tool_request(
            agena_tool_mode,
            &provider_native_tools,
            &mut request,
        );
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
            agena_tool_mode,
            provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if !self.native_compaction_enabled_for_adapter(adapter_id, &visible_model) {
            return Ok(None);
        }
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        agena_provider::apply_configured_tool_request(
            agena_tool_mode,
            &provider_native_tools,
            &mut request,
        );
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
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (
            _adapter_id,
            target_model,
            agena_tool_mode,
            provider_native_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        agena_provider::apply_configured_tool_request(
            agena_tool_mode,
            &provider_native_tools,
            &mut request,
        );
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use agena_plugin_host::{PluginKey, registry::RegisteredTool, sdk::ToolDefinition};
    use agena_provider::ProviderNativeToolRoute;
    use agena_provider::{CompletionFinishReason, CompletionToolCall};
    use agena_runtime_tools::tool::ToolApiBinding;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

    struct ProviderNativePromptAdapter {
        model: ModelId,
        calls: AtomicUsize,
    }

    struct RecordingAdapter {
        model: ModelId,
        request: Mutex<Option<CompletionRequest>>,
        compact_calls: AtomicUsize,
    }

    struct ImageAdapter {
        model: ModelId,
        request: Mutex<Option<ProviderImageRequest>>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for ImageAdapter {
        fn id(&self) -> &str {
            "image_adapter"
        }

        fn default_model(&self) -> &ModelId {
            &self.model
        }

        fn image_capabilities(&self, _model: &ModelId) -> Option<ProviderImageCapabilities> {
            Some(ProviderImageCapabilities {
                generate: true,
                edit: true,
                accepted_input_mime_types: vec!["image/png".to_owned()],
                max_input_bytes: Some(1024),
                max_input_images: Some(1),
            })
        }

        async fn execute_image(
            &self,
            model: &ModelId,
            request: ProviderImageRequest,
        ) -> Result<ProviderImageResponse, ProviderError> {
            *self.request.lock().expect("record image request") = Some(request);
            Ok(ProviderImageResponse {
                provider_id: ProviderId::new(self.id()),
                model: model.clone(),
                revised_prompt: None,
                artifacts: vec![agena_provider::ProviderNativeToolArtifact {
                    uri: "data:image/png;base64,iVBORw0KGgo=".to_owned(),
                    mime: "image/png".to_owned(),
                    name: Some("image.png".to_owned()),
                    size_bytes: None,
                    sha256: None,
                }],
                usage: None,
            })
        }

        async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                provider_id: ProviderId::new(self.id()),
                model: request.model,
                text: String::new(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl ModelRuntime for RecordingAdapter {
        fn id(&self) -> &str {
            "recording_adapter"
        }

        fn default_model(&self) -> &ModelId {
            &self.model
        }

        async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            *self.request.lock().expect("record request") = Some(request.clone());
            Ok(CompletionResponse {
                provider_id: ProviderId::new(self.id()),
                model: request.model,
                text: "ok".to_owned(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn compact_conversation(
            &self,
            _request: CompletionRequest,
        ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
            self.compact_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ProviderCompactionOutput::OpenAiResponses {
                items: vec![serde_json::json!({ "type": "compaction" })],
            }))
        }
    }

    #[async_trait::async_trait]
    impl ModelRuntime for ProviderNativePromptAdapter {
        fn id(&self) -> &str {
            "provider_native_prompt_adapter"
        }

        fn default_model(&self) -> &ModelId {
            &self.model
        }

        async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                provider_id: ProviderId::new(self.id()),
                model: request.model,
                text: String::new(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::ToolCalls),
                tool_calls: vec![CompletionToolCall::Function {
                    id: "native-call-0".to_owned(),
                    name: "tools_list".to_owned(),
                    arguments_json: "{}".to_owned(),
                }],
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>,
            >,
            ProviderError,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let provider_id = ProviderId::new(self.id());
            let model = request.model;
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(CompletionStreamEvent::ProviderNativeToolCallStarted {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "provider:web_search:0".to_owned(),
                    id: Some("web-search-0".to_owned()),
                    invocation: agena_domain::ToolInvocation::new(
                        "web_search",
                        agena_domain::StructuredObject::default(),
                    ),
                    title: "web search".to_owned(),
                    raw: None,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                    end_turn: None,
                }),
            ])))
        }
    }

    fn tool_api_list_binding() -> ToolApiBinding {
        let plugin = PluginKey::new("agena", "tools").expect("Tool API plugin key");
        let definition = ToolDefinition {
            name: "list".to_owned(),
            contract: agena_plugin_host::sdk::manifest::ToolContract {
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
                output_schema: serde_json::Value::Null,
                strict: true,
            },
            model: Default::default(),
            docs: Default::default(),
            runtime: Default::default(),
            permissions: Default::default(),
            display: Default::default(),
            tags: Vec::new(),
        };
        ToolApiBinding::from_registered_tool(
            RegisteredTool::new(plugin, definition).expect("registered Tool API handler"),
        )
        .expect("Tool API binding")
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: ModelId::new("model"),
            system: Some("base system".to_owned()),
            messages: vec![crate::provider::project_completion_input(&[Part {
                part_id: 1,
                kind: "text".to_owned(),
                role: PartRole::User,
                state: PartState::Completed,
                content: serde_json::json!({ "text": "use a tool" }),
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                run_id: Some(1),
                origin_session_id: 1,
                revision: 1,
                started_at_ms: 0,
                finished_at_ms: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                provider_state: None,
            }])],
            tool_api_functions: vec![tool_api_list_binding().definition()],
            provider_native_tools: Default::default(),
            disable_tools: false,
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,
            provider_compaction: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: None,
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: Default::default(),
        }
    }

    fn provider_for_adapter_with_mode(
        adapter: Arc<dyn ModelRuntime>,
        mode: AgenaToolMode,
    ) -> MultiAdapterProvider {
        provider_for_adapter_with_tool_policy(adapter, mode, ProviderNativeToolsConfig::default())
    }

    fn provider_for_adapter_with_tool_policy(
        adapter: Arc<dyn ModelRuntime>,
        mode: AgenaToolMode,
        provider_native_tools: ProviderNativeToolsConfig,
    ) -> MultiAdapterProvider {
        let adapters = BTreeMap::from([("adapter".to_owned(), adapter)]);
        let routes = BTreeMap::from([(
            ("adapter".to_owned(), "model".to_owned()),
            ProviderModelRoute {
                enabled: true,
                native_compaction: true,
                agena_tool_mode: mode,
                provider_native_tools,
                definition: Default::default(),
            },
        )]);
        MultiAdapterProvider::new(
            "provider",
            "adapter",
            "model",
            adapters,
            routes,
            BTreeSet::new(),
        )
    }

    fn provider_with_native_compaction_policy(
        adapter: Arc<dyn ModelRuntime>,
        native_compaction: bool,
    ) -> MultiAdapterProvider {
        MultiAdapterProvider::new(
            "provider",
            "adapter",
            "model",
            BTreeMap::from([("adapter".to_owned(), adapter)]),
            BTreeMap::from([(
                ("adapter".to_owned(), "model".to_owned()),
                ProviderModelRoute {
                    enabled: true,
                    native_compaction,
                    agena_tool_mode: AgenaToolMode::Disabled,
                    provider_native_tools: Default::default(),
                    definition: Default::default(),
                },
            )]),
            BTreeSet::new(),
        )
    }

    #[tokio::test]
    async fn model_route_native_compaction_policy_is_authoritative() {
        for (enabled, expected_calls) in [(true, 1), (false, 0)] {
            let adapter = Arc::new(RecordingAdapter {
                model: ModelId::new("model"),
                request: Mutex::new(None),
                compact_calls: AtomicUsize::new(0),
            });
            let provider = provider_with_native_compaction_policy(
                adapter.clone() as Arc<dyn ModelRuntime>,
                enabled,
            );

            assert_eq!(
                provider.native_compaction_enabled_for_adapter(
                    Some(&AdapterId::new("adapter")),
                    &ModelId::new("model"),
                ),
                enabled,
            );
            let output = provider
                .compact_conversation_for_adapter(Some(&AdapterId::new("adapter")), request())
                .await
                .expect("model route compaction policy should be applied");
            assert_eq!(output.is_some(), enabled);
            assert_eq!(adapter.compact_calls.load(Ordering::SeqCst), expected_calls);

            let listed = provider
                .list_models()
                .await
                .expect("configured route should be synthesized");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].native_compaction, enabled);
        }
    }

    #[tokio::test]
    async fn disabled_mode_strips_all_tool_configuration_from_provider_request() {
        let adapter = Arc::new(RecordingAdapter {
            model: ModelId::new("model"),
            request: Mutex::new(None),
            compact_calls: AtomicUsize::new(0),
        });
        let provider = provider_for_adapter_with_mode(
            adapter.clone() as Arc<dyn ModelRuntime>,
            AgenaToolMode::Disabled,
        );
        let mut input = request();
        input.provider_native_tools.routes.web_search =
            Some(ProviderNativeToolRoute::ProviderHosted);

        provider
            .complete(input)
            .await
            .expect("disabled tool mode should still complete normally");

        let recorded = adapter
            .request
            .lock()
            .expect("recorded request")
            .clone()
            .expect("adapter should receive a request");
        assert!(recorded.tool_api_functions.is_empty());
        assert!(recorded.provider_native_tools.is_empty());
        assert_eq!(recorded.system.as_deref(), Some("base system"));
    }

    #[tokio::test]
    async fn provider_protocol_strips_removed_route_native_tool_configuration() {
        let adapter = Arc::new(RecordingAdapter {
            model: ModelId::new("model"),
            request: Mutex::new(None),
            compact_calls: AtomicUsize::new(0),
        });
        let mut configured_native = ProviderNativeToolsConfig::default();
        configured_native.routes.web_search = Some(ProviderNativeToolRoute::ProviderHosted);
        let provider = provider_for_adapter_with_tool_policy(
            adapter.clone() as Arc<dyn ModelRuntime>,
            AgenaToolMode::ProviderProtocol,
            configured_native,
        );
        let mut input = request();
        input.provider_native_tools.routes.file_search =
            Some(ProviderNativeToolRoute::ProviderHosted);
        input.previous_response_id = Some("provider-response".to_owned());

        provider
            .complete(input)
            .await
            .expect("provider protocol should complete normally");

        let recorded = adapter
            .request
            .lock()
            .expect("recorded request")
            .clone()
            .expect("adapter should receive a request");
        assert_eq!(recorded.tool_api_functions.len(), 1);
        assert!(recorded.provider_native_tools.is_empty());
        assert_eq!(recorded.provider_native_tools.routes.file_search, None);
        assert_eq!(
            recorded.previous_response_id.as_deref(),
            Some("provider-response")
        );
        assert_eq!(recorded.system.as_deref(), Some("base system"));
    }

    #[tokio::test]
    async fn direct_image_port_requires_route_opt_in_and_merges_route_options() {
        let adapter = Arc::new(ImageAdapter {
            model: ModelId::new("model"),
            request: Mutex::new(None),
        });
        let disabled = provider_for_adapter_with_tool_policy(
            adapter.clone() as Arc<dyn ModelRuntime>,
            AgenaToolMode::ProviderProtocol,
            ProviderNativeToolsConfig::default(),
        );
        assert!(
            disabled
                .image_capabilities_for_adapter(
                    Some(&AdapterId::new("adapter")),
                    &ModelId::new("model")
                )
                .is_none()
        );
        let error = disabled
            .execute_image_for_adapter(
                Some(&AdapterId::new("adapter")),
                &ModelId::new("model"),
                ProviderImageRequest {
                    operation: agena_provider::ProviderImageOperation::Generate,
                    prompt: "fixture".to_owned(),
                    inputs: Vec::new(),
                    options: Default::default(),
                },
            )
            .await
            .expect_err("route without image_generation must reject execution");
        assert!(error.to_string().contains("does not enable"));

        let mut native = ProviderNativeToolsConfig::default();
        native.routes.image_generation = Some(ProviderNativeToolRoute::ProviderHosted);
        native.hosted.image_generation.size = Some("1024x1024".to_owned());
        let enabled = provider_for_adapter_with_tool_policy(
            adapter.clone() as Arc<dyn ModelRuntime>,
            AgenaToolMode::ProviderProtocol,
            native,
        );
        assert!(
            enabled
                .image_capabilities_for_adapter(
                    Some(&AdapterId::new("adapter")),
                    &ModelId::new("model")
                )
                .is_some()
        );
        enabled
            .execute_image_for_adapter(
                Some(&AdapterId::new("adapter")),
                &ModelId::new("model"),
                ProviderImageRequest {
                    operation: agena_provider::ProviderImageOperation::Generate,
                    prompt: "fixture".to_owned(),
                    inputs: Vec::new(),
                    options: ProviderHostedImageGenerationConfig {
                        quality: Some("high".to_owned()),
                        ..Default::default()
                    },
                },
            )
            .await
            .expect("enabled direct image route");
        let recorded = adapter
            .request
            .lock()
            .expect("recorded image request")
            .clone()
            .expect("adapter image request");
        assert_eq!(recorded.options.size.as_deref(), Some("1024x1024"));
        assert_eq!(recorded.options.quality.as_deref(), Some("high"));
    }

    #[test]
    fn each_tool_mode_has_a_distinct_prompt_cache_shape() {
        let adapter = Arc::new(RecordingAdapter {
            model: ModelId::new("model"),
            request: Mutex::new(None),
            compact_calls: AtomicUsize::new(0),
        });
        let shapes = [AgenaToolMode::ProviderProtocol, AgenaToolMode::Disabled].map(|mode| {
            provider_for_adapter_with_mode(adapter.clone() as Arc<dyn ModelRuntime>, mode)
                .prompt_cache_shape(&ModelId::new("model"))
                .expect("tool mode must be part of the cache shape")
        });

        assert_eq!(
            shapes[0].fields.get("agena.tools.mode").map(String::as_str),
            Some("provider_protocol")
        );
        assert_eq!(
            shapes[1].fields.get("agena.tools.mode").map(String::as_str),
            Some("disabled")
        );
        assert_ne!(shapes[0].fingerprint(), shapes[1].fingerprint());
    }

    #[tokio::test]
    async fn disabled_mode_rejects_backend_native_tool_calls() {
        let adapter = Arc::new(ProviderNativePromptAdapter {
            model: ModelId::new("model"),
            calls: AtomicUsize::new(0),
        });
        let provider = provider_for_adapter_with_mode(
            adapter as Arc<dyn ModelRuntime>,
            AgenaToolMode::Disabled,
        );

        let error = provider
            .complete(request())
            .await
            .expect_err("disabled mode must reject native tool calls");

        assert!(error.to_string().contains("disabled Agena tools mode"));
    }

    #[tokio::test]
    async fn disabled_mode_rejects_backend_provider_native_tool_stream_events() {
        let adapter = Arc::new(ProviderNativePromptAdapter {
            model: ModelId::new("model"),
            calls: AtomicUsize::new(0),
        });
        let provider = provider_for_adapter_with_mode(
            adapter as Arc<dyn ModelRuntime>,
            AgenaToolMode::Disabled,
        );

        let events = provider
            .complete_stream(request())
            .await
            .expect("construct disabled guard stream")
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 1);
        assert!(
            events[0]
                .as_ref()
                .expect_err("disabled mode must reject provider-native tool activity")
                .to_string()
                .contains("disabled Agena tools mode")
        );
    }
}
