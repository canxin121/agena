use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{StreamExt, stream::BoxStream};

use crate::{
    error::AppError,
    model::{
        AdapterId, Model, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode, ProviderId,
    },
};

use super::core::{
    impl_model_runtime_base_via_adapter_methods, remap_stream_event_provider_and_model,
};
use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, ConfiguredModelDefinition,
    ModelCapabilities, ModelRuntime, PromptCacheShape, StreamResumePolicy,
    configured_models::apply_configured_modes, prompt_tool_transport,
};
use crate::config::{AgenaToolTransport, ProviderToolsConfig};

#[derive(Debug, Clone)]
pub struct ProviderModelRoute {
    pub enabled: bool,
    pub agena_tool_transport: AgenaToolTransport,
    pub provider_tools: ProviderToolsConfig,
    pub definition: ConfiguredModelDefinition,
}

pub type ProviderModelRouteKey = (String, String);

#[derive(Clone)]
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

    fn adapter(&self, adapter_id: &str) -> Result<Arc<dyn ModelRuntime>, AppError> {
        self.adapters.get(adapter_id).cloned().ok_or_else(|| {
            AppError::Config(format!(
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
            AgenaToolTransport,
            ProviderToolsConfig,
            ConfiguredModelDefinition,
        ),
        AppError,
    > {
        let adapter_id = self.selected_adapter(adapter_id);
        let target_model = model.clone();
        let key = (adapter_id.to_string(), target_model.to_string());
        if let Some(route) = self.routes.get(&key) {
            if !route.enabled {
                return Err(AppError::Config(format!(
                    "provider `{}` adapter `{adapter_id}` model `{}` is disabled",
                    self.id, model
                )));
            }
            return Ok((
                adapter_id,
                target_model,
                route.agena_tool_transport,
                route.provider_tools.clone(),
                route.definition.clone(),
            ));
        }

        self.adapter(adapter_id.as_ref())?;
        Ok((
            adapter_id,
            target_model,
            AgenaToolTransport::default(),
            ProviderToolsConfig::default(),
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
            AgenaToolTransport,
            ProviderToolsConfig,
            ConfiguredModelDefinition,
            Arc<dyn ModelRuntime>,
        ),
        AppError,
    > {
        let (adapter_id, target_model, agena_tool_transport, provider_tools, definition) =
            self.resolve_route(adapter_id, model)?;
        let adapter = self.adapter(adapter_id.as_ref())?;
        Ok((
            adapter_id,
            target_model,
            agena_tool_transport,
            provider_tools,
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
            AgenaToolTransport,
            &ProviderToolsConfig,
            &ConfiguredModelDefinition,
            &dyn ModelRuntime,
        ) -> T,
    ) -> Option<T> {
        let (adapter_id, target_model, agena_tool_transport, provider_tools, definition, adapter) =
            self.resolve_route_and_adapter(adapter_id, model).ok()?;
        Some(map(
            &adapter_id,
            &target_model,
            agena_tool_transport,
            &provider_tools,
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
            capabilities: ModelCapabilities::default(),
            metadata: ModelMetadata::default(),
            thinking_modes: BTreeMap::new(),
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
        fn model_thinking_modes / model_thinking_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode>;
        fn model_speed_modes / model_speed_modes_for_adapter (&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode>;
        fn supports_prompt_continuation / supports_prompt_continuation_for_adapter (&self, model: &ModelId) -> bool;
        fn prompt_cache_shape / prompt_cache_shape_for_adapter (&self, model: &ModelId) -> Option<PromptCacheShape>;
        fn provider_tools_config / provider_tools_config_for_adapter (&self, model: &ModelId) -> ProviderToolsConfig;
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
             _agena_tool_transport,
             _provider_tools,
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
             _agena_tool_transport,
             _provider_tools,
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
    ) -> BTreeMap<String, ModelThinkingMode> {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             target_model,
             _agena_tool_transport,
             _provider_tools,
             definition,
             adapter| {
                apply_configured_modes(
                    adapter.model_thinking_modes(target_model),
                    definition.thinking_modes.iter(),
                    |configured, existing| configured.apply_to_mode(existing),
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
             _agena_tool_transport,
             _provider_tools,
             definition,
             adapter| {
                apply_configured_modes(
                    adapter.model_speed_modes(target_model),
                    definition.speed_modes.iter(),
                    |configured, existing| configured.apply_to_mode(existing),
                )
            },
        )
        .unwrap_or_default()
    }

    fn provider_tools_config_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ProviderToolsConfig {
        self.map_route_and_adapter(
            adapter_id,
            model,
            |_adapter_id,
             _target_model,
             _agena_tool_transport,
             provider_tools,
             _definition,
             _adapter| { provider_tools.clone() },
        )
        .unwrap_or_default()
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
             _agena_tool_transport,
             _provider_tools,
             _definition,
             adapter| { adapter.supports_prompt_continuation(target_model) },
        )
        .unwrap_or(false)
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
             agena_tool_transport,
             _provider_tools,
             _definition,
             adapter| {
                let base = adapter.prompt_cache_shape(target_model);
                if agena_tool_transport.is_provider_protocol() {
                    return base;
                }
                let mut shape = base.unwrap_or_else(|| PromptCacheShape::new(self.id.as_str()));
                shape.insert_string(
                    "agena.tools.transport",
                    prompt_tool_transport::PROTOCOL_VERSION,
                );
                Some(shape)
            },
        )
        .flatten()
    }

    fn validate_provider_tools_request(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        if request.provider_tools.bindings().is_empty() {
            return Ok(());
        }
        let (
            _adapter_id,
            target_model,
            agena_tool_transport,
            _provider_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &request.model)?;
        if agena_tool_transport.is_prompt_envelope() {
            return prompt_tool_transport::validate_request(request);
        }
        let mut delegated = request.clone();
        delegated.model = target_model;
        adapter.validate_provider_tools_request(None, &delegated)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
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
            return Err(AppError::Provider(format!(
                "adapter model discovery failed: {}",
                errors.join("; ")
            )));
        }

        Ok(visible)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.complete_for_adapter(None, request).await
    }

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let visible_model = request.model.clone();
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (
            _adapter_id,
            target_model,
            agena_tool_transport,
            _provider_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::prepare_request(&mut request)?;
        }
        request.model = target_model;
        let mut response = adapter.complete(request).await?;
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::rewrite_response(&mut response);
        }
        response.provider_id = ProviderId::new(self.id.clone());
        response.model = visible_model;
        Ok(response)
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        self.compact_conversation_for_adapter(None, request).await
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let visible_model = request.model.clone();
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (
            _adapter_id,
            target_model,
            agena_tool_transport,
            _provider_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::prepare_request(&mut request)?;
        }
        request.model = target_model;
        adapter.compact_conversation(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.complete_stream_for_adapter(None, request).await
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let visible_model = request.model.clone();
        self.backfill_assistant_reasoning_field(adapter_id, &mut request);
        let (
            _adapter_id,
            target_model,
            agena_tool_transport,
            _provider_tools,
            _definition,
            adapter,
        ) = self.resolve_route_and_adapter(adapter_id, &visible_model)?;
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::prepare_request(&mut request)?;
        }
        request.model = target_model;
        let provider_id = self.id.clone();
        let stream = adapter.complete_stream(request).await?;
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
            Box::pin(stream.map(move |item| {
                let provider_id = ProviderId::new(provider_id.clone());
                let visible_model = visible_model.clone();
                item.map(|event| {
                    remap_stream_event_provider_and_model(&provider_id, &visible_model, event)
                })
            }));
        if agena_tool_transport.is_prompt_envelope() {
            Ok(prompt_tool_transport::rewrite_stream(Box::pin(stream)))
        } else {
            Ok(Box::pin(stream))
        }
    }
}
