use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;
use std::collections::BTreeMap;

use crate::error::AppError;
use crate::model::{Model, ModelCapabilities, ModelId, ModelMetadata, ModelVariant};

use super::{
    CapabilityFamily, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    PromptCacheShape,
};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &ModelId;

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

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        match self.capability_family() {
            Some(family) => {
                super::default_model_metadata_registry().metadata_for_family(family, model.as_str())
            }
            None => ModelMetadata::default(),
        }
    }

    fn model_variants(&self, model: &ModelId) -> BTreeMap<String, ModelVariant> {
        let _ = model;
        BTreeMap::new()
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::Disabled
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        let _ = model;
        false
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        let _ = model;
        None
    }

    fn prompt_cache_shape_fingerprint(&self, model: &ModelId) -> Option<String> {
        self.prompt_cache_shape(model)
            .map(|shape| shape.fingerprint())
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError>;

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError>;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamResumePolicy {
    Disabled,
    ReplaySafePrefix,
}
