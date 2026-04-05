use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;

use crate::error::AppError;
use crate::model::{Model, ModelCapabilities, ModelId, ModelMetadata};

use super::{CompletionRequest, CompletionResponse, CompletionStreamEvent};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &ModelId;

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        let _ = model;
        ModelCapabilities::default()
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        let _ = model;
        ModelMetadata::default()
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::Disabled
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        let _ = model;
        false
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
        let events = vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: response.provider_id.clone(),
                model: response.model.clone(),
                delta: response.text,
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: response.provider_id,
                model: response.model,
                finish_reason: response.finish_reason,
                usage: response.usage,
                provider_metadata: response.provider_metadata,
            }),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamResumePolicy {
    Disabled,
    ReplaySafePrefix,
}
