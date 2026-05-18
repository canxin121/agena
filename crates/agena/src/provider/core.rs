use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;
use std::collections::BTreeMap;

use crate::error::AppError;
use crate::model::{
    AdapterId, Model, ModelCapabilities, ModelId, ModelMetadata, ModelSpeedMode, ModelThinkingMode,
};

use super::{
    CapabilityFamily, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    PromptCacheShape,
};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &ModelId;
    fn default_adapter(&self) -> Option<&AdapterId> {
        None
    }

    /// Return the capability family used to look up model capabilities and
    /// metadata from the global registries.  Providers that use the standard
    /// registries only need to override this one method; `model_capabilities`
    /// and `model_metadata` are derived from it automatically.
    ///
    /// Providers that need custom logic can still override
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
