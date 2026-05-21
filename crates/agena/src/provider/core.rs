use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;
use std::collections::BTreeMap;

use crate::error::AppError;
use crate::model::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
    ProviderId,
};

use super::{
    CapabilityFamily, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    PromptCacheShape, chat_wire,
};

#[async_trait]
pub trait ModelRuntime: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &ModelId;
    fn default_adapter(&self) -> Option<&AdapterId> {
        None
    }

    /// Return the capability family used to look up model capabilities and
    /// metadata from the global registries. Implementations that use the
    /// standard registries only need to override this one method;
    /// `model_capabilities` and `model_metadata` are derived from it
    /// automatically.
    ///
    /// Implementations that need custom logic can still override
    /// `model_capabilities`/`model_metadata` directly.
    fn capability_family(&self) -> Option<CapabilityFamily> {
        None
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        match self.capability_family() {
            Some(family) => {
                super::default_capability_registry().capabilities_for_family(family, model.as_str())
            }
            None => ModelCapabilities::default(),
        }
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let _ = adapter_id;
        self.model_capabilities(model)
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        match self.capability_family() {
            Some(family) => {
                super::default_model_metadata_registry().metadata_for_family(family, model.as_str())
            }
            None => ModelMetadata::default(),
        }
    }

    fn model_metadata_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> ModelMetadata {
        let _ = adapter_id;
        self.model_metadata(model)
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        match self.capability_family() {
            Some(family) => super::default_model_mode_registry().thinking_modes_for_family(
                family,
                None,
                model.as_str(),
                &self.model_metadata(model),
            ),
            None => BTreeMap::new(),
        }
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        match self.capability_family() {
            Some(family) => super::default_model_mode_registry().thinking_modes_for_family(
                family,
                adapter_id,
                model.as_str(),
                &self.model_metadata_for_adapter(adapter_id, model),
            ),
            None => self.model_thinking_modes(model),
        }
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        let _ = model;
        BTreeMap::new()
    }

    fn model_speed_modes_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelSpeedMode> {
        let _ = adapter_id;
        self.model_speed_modes(model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::Disabled
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        let _ = model;
        false
    }

    fn supports_prompt_continuation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> bool {
        let _ = adapter_id;
        self.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        let _ = model;
        None
    }

    fn prompt_cache_shape_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        model: &ModelId,
    ) -> Option<PromptCacheShape> {
        let _ = adapter_id;
        self.prompt_cache_shape(model)
    }

    fn prompt_cache_shape_fingerprint(&self, model: &ModelId) -> Option<String> {
        self.prompt_cache_shape(model)
            .map(|shape| shape.fingerprint())
    }

    fn backfill_assistant_reasoning_field(
        &self,
        adapter_id: Option<&AdapterId>,
        request: &mut CompletionRequest,
    ) {
        let metadata = self.model_metadata_for_adapter(adapter_id, &request.model);
        chat_wire::backfill_assistant_reasoning_field_on_request(
            request,
            metadata.assistant_reasoning_field.as_deref(),
            metadata.assistant_reasoning_interleaved.unwrap_or(false),
        );
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError>;

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError>;

    async fn complete_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let _ = adapter_id;
        self.complete(request).await
    }

    /// Provider-native conversation compaction. Providers that do not expose a
    /// dedicated compaction API return `Ok(None)` so callers can fall back to
    /// an ordinary local summarization turn.
    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let _ = request;
        Ok(None)
    }

    async fn compact_conversation_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let _ = adapter_id;
        self.compact_conversation(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let response = self.complete(request).await?;
        let mut events = Vec::new();
        if let Some(reasoning) = response.reasoning_text
            && !reasoning.is_empty()
        {
            events.push(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: response.provider_id.clone(),
                model: response.model.clone(),
                delta: reasoning,
            }));
        }
        events.push(Ok(CompletionStreamEvent::TextDelta {
            provider_id: response.provider_id.clone(),
            model: response.model.clone(),
            delta: response.text,
        }));
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id: response.provider_id,
            model: response.model,
            finish_reason: response.finish_reason,
            usage: response.usage,
            provider_metadata: response.provider_metadata,
        }));
        Ok(Box::pin(stream::iter(events)))
    }

    async fn complete_stream_for_adapter(
        &self,
        adapter_id: Option<&AdapterId>,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let _ = adapter_id;
        self.complete_stream(request).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamResumePolicy {
    Disabled,
    ReplaySafePrefix,
}

pub(crate) fn remap_stream_event_provider_id(
    provider_id: &ProviderId,
    event: CompletionStreamEvent,
) -> CompletionStreamEvent {
    match event {
        CompletionStreamEvent::TextDelta { model, delta, .. } => CompletionStreamEvent::TextDelta {
            provider_id: provider_id.clone(),
            model,
            delta,
        },
        CompletionStreamEvent::ThinkingDelta { model, delta, .. } => {
            CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model,
                delta,
            }
        }
        CompletionStreamEvent::ToolCallDelta {
            model,
            stream_key,
            id,
            name,
            arguments_delta,
            ..
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model,
            stream_key,
            id,
            name,
            arguments_delta,
        },
        CompletionStreamEvent::Completed {
            model,
            finish_reason,
            usage,
            provider_metadata,
            ..
        } => CompletionStreamEvent::Completed {
            provider_id: provider_id.clone(),
            model,
            finish_reason,
            usage,
            provider_metadata,
        },
    }
}

pub(crate) fn remap_stream_event_provider_and_model(
    provider_id: &ProviderId,
    model: &ModelId,
    event: CompletionStreamEvent,
) -> CompletionStreamEvent {
    match event {
        CompletionStreamEvent::TextDelta { delta, .. } => CompletionStreamEvent::TextDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            delta,
        },
        CompletionStreamEvent::ThinkingDelta { delta, .. } => {
            CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta,
            }
        }
        CompletionStreamEvent::ToolCallDelta {
            stream_key,
            id,
            name,
            arguments_delta,
            ..
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_delta,
        },
        CompletionStreamEvent::Completed {
            finish_reason,
            usage,
            provider_metadata,
            ..
        } => CompletionStreamEvent::Completed {
            provider_id: provider_id.clone(),
            model: model.clone(),
            finish_reason,
            usage,
            provider_metadata,
        },
    }
}
