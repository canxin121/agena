use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};

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
    CompletionRequest, CompletionResponse, CompletionStreamEvent, CompletionUsage,
    ConfiguredModelDefinition, ModelCapabilities, ModelRuntime, PromptCacheShape,
    StreamResumePolicy, configured_models::apply_configured_modes, prompt_tool_transport,
};
use crate::config::{AgenaToolTransport, ProviderToolsConfig};

const MAX_PROMPT_TOOL_REPAIRS: usize = 2;

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
        let repair_context = if agena_tool_transport.is_prompt_envelope() {
            Some(prompt_tool_transport::repair_context(&request)?)
        } else {
            None
        };
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::prepare_request(&mut request)?;
        }
        request.model = target_model;
        let mut discarded_usage = None;
        let mut repair_count = 0_usize;
        let mut response = loop {
            let mut response = adapter.complete(request.clone()).await?;
            let repair_reason = repair_context.as_ref().and_then(|context| {
                prompt_tool_transport::repair_reason(
                    response.text.as_str(),
                    !response.tool_calls.is_empty(),
                    context,
                )
            });
            if let Some(reason) = repair_reason {
                if repair_count < MAX_PROMPT_TOOL_REPAIRS {
                    merge_completion_usage(&mut discarded_usage, response.usage.take());
                    prompt_tool_transport::append_repair_turn(
                        &mut request,
                        response.text.as_str(),
                        reason.as_str(),
                        repair_context.as_ref().expect("repair context is present"),
                    );
                    repair_count += 1;
                    tracing::warn!(
                        provider_id = self.id.as_str(),
                        model_id = %visible_model,
                        repair_count,
                        reason = reason.as_str(),
                        "retrying rejected prompt-envelope tool response"
                    );
                    continue;
                }
                return Err(prompt_tool_protocol_error(
                    self.id.as_str(),
                    &visible_model,
                    repair_count,
                    reason.as_str(),
                ));
            }
            merge_completion_usage(&mut response.usage, discarded_usage.take());
            break response;
        };
        if repair_context.is_some() {
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
            prompt_tool_transport::prepare_compaction_request(&mut request)?;
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
        let repair_context = if agena_tool_transport.is_prompt_envelope() {
            Some(prompt_tool_transport::repair_context(&request)?)
        } else {
            None
        };
        if agena_tool_transport.is_prompt_envelope() {
            prompt_tool_transport::prepare_request(&mut request)?;
        }
        request.model = target_model;
        let provider_id = self.id.clone();
        let stream = adapter.complete_stream(request.clone()).await?;
        if let Some(repair_context) = repair_context {
            let repaired = async_stream::stream! {
                let mut next_stream = Some(stream);
                let mut repair_count = 0_usize;
                let mut discarded_usage = None;

                loop {
                    let mut inner_stream = match next_stream.take() {
                        Some(stream) => stream,
                        None => match adapter.complete_stream(request.clone()).await {
                            Ok(stream) => stream,
                            Err(error) => {
                                yield Err(error);
                                break;
                            }
                        },
                    };
                    let mut buffered = Vec::new();
                    let mut response_text = String::new();
                    let mut has_client_tool_call = false;
                    let mut has_provider_tool_activity = false;
                    let mut stream_failed = false;

                    while let Some(item) = inner_stream.next().await {
                        match &item {
                            Ok(CompletionStreamEvent::TextDelta { delta, .. }) => {
                                response_text.push_str(delta);
                            }
                            Ok(
                                CompletionStreamEvent::ToolCallDelta { .. }
                                | CompletionStreamEvent::ToolCallSnapshot { .. },
                            ) => {
                                has_client_tool_call = true;
                            }
                            Ok(
                                CompletionStreamEvent::ProviderToolCallStarted { .. }
                                | CompletionStreamEvent::ProviderToolCallCompleted { .. },
                            ) => {
                                has_provider_tool_activity = true;
                            }
                            Err(_) => stream_failed = true,
                            _ => {}
                        }
                        buffered.push(item);
                    }

                    let repair_reason = (!stream_failed)
                        .then(|| {
                            if has_provider_tool_activity {
                                return Some(
                                    "the backend used a provider-native tool, but prompt-envelope mode permits only the declared Agena client-function envelope"
                                        .to_owned(),
                                );
                            }
                            prompt_tool_transport::repair_reason(
                                response_text.as_str(),
                                has_client_tool_call,
                                &repair_context,
                            )
                        })
                        .flatten();
                    if let Some(reason) = repair_reason {
                        if repair_count < MAX_PROMPT_TOOL_REPAIRS {
                            take_stream_usage(buffered.as_mut_slice(), &mut discarded_usage);
                            prompt_tool_transport::append_repair_turn(
                                &mut request,
                                response_text.as_str(),
                                reason.as_str(),
                                &repair_context,
                            );
                            repair_count += 1;
                            tracing::warn!(
                                provider_id = provider_id.as_str(),
                                model_id = %visible_model,
                                repair_count,
                                reason = reason.as_str(),
                                "retrying rejected prompt-envelope tool stream"
                            );
                            continue;
                        }
                        yield Err(prompt_tool_protocol_error(
                            provider_id.as_str(),
                            &visible_model,
                            repair_count,
                            reason.as_str(),
                        ));
                        break;
                    }

                    add_stream_usage(buffered.as_mut_slice(), discarded_usage.take());
                    let source = Box::pin(stream::iter(buffered));
                    let mut rewritten = prompt_tool_transport::rewrite_stream(source);
                    while let Some(item) = rewritten.next().await {
                        let provider_id = ProviderId::new(provider_id.clone());
                        let visible_model = visible_model.clone();
                        yield item.map(|event| {
                            remap_stream_event_provider_and_model(
                                &provider_id,
                                &visible_model,
                                event,
                            )
                        });
                    }
                    break;
                }
            };
            return Ok(Box::pin(repaired));
        }
        let stream: BoxStream<'static, Result<CompletionStreamEvent, AppError>> =
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

fn prompt_tool_protocol_error(
    provider_id: &str,
    model: &ModelId,
    repair_count: usize,
    reason: &str,
) -> AppError {
    AppError::Provider(format!(
        "provider `{provider_id}` model `{model}` violated the Agena prompt-envelope tool protocol after {repair_count} repair attempt(s): {reason}"
    ))
}

fn merge_completion_usage(
    target: &mut Option<CompletionUsage>,
    additional: Option<CompletionUsage>,
) {
    let Some(additional) = additional else {
        return;
    };
    let Some(target) = target.as_mut() else {
        *target = Some(additional);
        return;
    };
    target.input_tokens = target.input_tokens.saturating_add(additional.input_tokens);
    target.output_tokens = target
        .output_tokens
        .saturating_add(additional.output_tokens);
    target.reasoning_tokens = target
        .reasoning_tokens
        .saturating_add(additional.reasoning_tokens);
    target.cache_write_tokens = target
        .cache_write_tokens
        .saturating_add(additional.cache_write_tokens);
    target.cache_read_tokens = target
        .cache_read_tokens
        .saturating_add(additional.cache_read_tokens);
    target.total_cost += additional.total_cost;
}

fn take_stream_usage(
    events: &mut [Result<CompletionStreamEvent, AppError>],
    usage: &mut Option<CompletionUsage>,
) {
    for event in events {
        if let Ok(CompletionStreamEvent::Completed {
            usage: event_usage, ..
        }) = event
        {
            merge_completion_usage(usage, event_usage.take());
        }
    }
}

fn add_stream_usage(
    events: &mut [Result<CompletionStreamEvent, AppError>],
    additional: Option<CompletionUsage>,
) {
    let Some(additional) = additional else {
        return;
    };
    if let Some(usage) = events.iter_mut().find_map(|event| match event {
        Ok(CompletionStreamEvent::Completed { usage, .. }) => Some(usage),
        _ => None,
    }) {
        merge_completion_usage(usage, Some(additional));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{
        config::AgenaToolTransport,
        message::Message,
        plugin::{PluginKey, registry::RegisteredTool, sdk::ToolDefinition},
        provider::CompletionFinishReason,
        role::Role,
        tool::ToolApiBinding,
    };

    struct InvalidPromptEnvelopeAdapter {
        model: ModelId,
        calls: AtomicUsize,
    }

    struct ProviderNativePromptAdapter {
        model: ModelId,
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for InvalidPromptEnvelopeAdapter {
        fn id(&self) -> &str {
            "invalid_prompt_adapter"
        }

        fn default_model(&self) -> &ModelId {
            &self.model
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                provider_id: ProviderId::new(self.id()),
                model: request.model,
                text: "<agena_tool_calls>not-json</agena_tool_calls>".to_owned(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
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

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
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

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let provider_id = ProviderId::new(self.id());
            let model = request.model;
            Ok(Box::pin(stream::iter(vec![
                Ok(CompletionStreamEvent::ProviderToolCallStarted {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "provider:web_search:0".to_owned(),
                    id: Some("web-search-0".to_owned()),
                    invocation: crate::message::ToolInvocation::new(
                        "web_search",
                        crate::message::StructuredObject::default(),
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
                }),
            ])))
        }
    }

    fn tool_api_list_binding() -> ToolApiBinding {
        let plugin = PluginKey::new("agena", "tools").expect("Tool API plugin key");
        let definition = ToolDefinition {
            name: "list".to_owned(),
            contract: crate::plugin::sdk::manifest::ToolContract {
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
            capabilities: Vec::new(),
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
            messages: vec![Message::prompt_text(Role::User, "use a tool")],
            tool_api_functions: vec![tool_api_list_binding()],
            provider_tools: Default::default(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,
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

    fn provider_for_adapter(adapter: Arc<dyn ModelRuntime>) -> MultiAdapterProvider {
        let adapters = BTreeMap::from([("adapter".to_owned(), adapter)]);
        let routes = BTreeMap::from([(
            ("adapter".to_owned(), "model".to_owned()),
            ProviderModelRoute {
                enabled: true,
                agena_tool_transport: AgenaToolTransport::PromptEnvelope,
                provider_tools: Default::default(),
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

    fn provider() -> (MultiAdapterProvider, Arc<InvalidPromptEnvelopeAdapter>) {
        let adapter = Arc::new(InvalidPromptEnvelopeAdapter {
            model: ModelId::new("model"),
            calls: AtomicUsize::new(0),
        });
        (
            provider_for_adapter(adapter.clone() as Arc<dyn ModelRuntime>),
            adapter,
        )
    }

    #[tokio::test]
    async fn non_streaming_prompt_protocol_exhaustion_fails_closed() {
        let (provider, adapter) = provider();

        let error = provider
            .complete(request())
            .await
            .expect_err("invalid prompt protocol must fail");

        assert!(
            error
                .to_string()
                .contains("violated the Agena prompt-envelope")
        );
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn streaming_prompt_protocol_exhaustion_does_not_leak_rejected_text() {
        let (provider, adapter) = provider();
        let events = provider
            .complete_stream(request())
            .await
            .expect("construct repaired stream")
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 1);
        let error = events[0]
            .as_ref()
            .expect_err("invalid prompt protocol must fail");
        assert!(
            error
                .to_string()
                .contains("violated the Agena prompt-envelope")
        );
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn streaming_prompt_transport_rejects_backend_native_tools() {
        let adapter = Arc::new(ProviderNativePromptAdapter {
            model: ModelId::new("model"),
            calls: AtomicUsize::new(0),
        });
        let provider = provider_for_adapter(adapter.clone() as Arc<dyn ModelRuntime>);

        let events = provider
            .complete_stream(request())
            .await
            .expect("construct repaired stream")
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 1);
        let error = events[0]
            .as_ref()
            .expect_err("provider-native activity must fail closed");
        assert!(error.to_string().contains("prompt-envelope"));
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 3);
    }
}
