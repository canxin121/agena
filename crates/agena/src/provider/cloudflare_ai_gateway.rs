use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    model::{Model, ModelId},
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelProvider,
        OpenAiCompatibleProvider, StreamResumePolicy,
    },
};

const PROVIDER_ID: &str = "cloudflare-ai-gateway";

#[derive(Clone)]
pub struct CloudflareAiGatewayProvider {
    inner: OpenAiCompatibleProvider,
}

impl CloudflareAiGatewayProvider {
    pub fn new(inner: OpenAiCompatibleProvider) -> Self {
        Self { inner }
    }

    pub fn with_gateway(
        client: reqwest::Client,
        api_token: impl Into<String>,
        account_id: impl AsRef<str>,
        gateway_id: impl AsRef<str>,
        default_model: impl Into<String>,
    ) -> Self {
        let base_url = format!(
            "https://gateway.ai.cloudflare.com/v1/{}/{}/compat",
            account_id.as_ref(),
            gateway_id.as_ref()
        );
        let inner =
            OpenAiCompatibleProvider::new(PROVIDER_ID, client, api_token, base_url, default_model);
        Self { inner }
    }

    fn normalize_model(&self, model: &ModelId) -> Result<ModelId, AppError> {
        let normalized = model.as_str().trim();

        if normalized.contains('/') && !normalized.starts_with('/') && !normalized.ends_with('/') {
            return Ok(ModelId::new(normalized));
        }

        Err(AppError::Config(format!(
            "{PROVIDER_ID} requires unified model id in `provider/model` format"
        )))
    }
}

#[async_trait]
impl ModelProvider for CloudflareAiGatewayProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        self.inner.default_model()
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        self.inner.model_capabilities(model)
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        self.inner.model_metadata(model)
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.inner.stream_resume_policy()
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        self.inner.prompt_cache_shape(model)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        self.inner.list_models().await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = self.normalize_model(&request.model)?;
        self.inner
            .complete(CompletionRequest { model, ..request })
            .await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = self.normalize_model(&request.model)?;
        self.inner
            .complete_stream(CompletionRequest { model, ..request })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_model_requires_unified_format() {
        let provider = CloudflareAiGatewayProvider {
            inner: OpenAiCompatibleProvider::new(
                PROVIDER_ID,
                reqwest::Client::new(),
                "token",
                "https://example.com",
                "workers-ai/@cf/meta/llama-3.1-8b-instruct",
            ),
        };

        assert!(
            provider
                .normalize_model(&crate::model::ModelId::new(
                    "workers-ai/@cf/meta/llama-3.1-8b-instruct",
                ))
                .is_ok()
        );
        assert!(
            provider
                .normalize_model(&crate::model::ModelId::new("gpt-4.1-mini"))
                .is_err()
        );
    }
}
