use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    model::{Model, ModelId},
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, ManagedCredential,
        ModelProvider, OpenAiCompatibleProvider, StreamResumePolicy,
    },
};

#[derive(Clone)]
pub struct GoogleVertexProvider {
    id: String,
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
    auth: ManagedCredential,
}

impl GoogleVertexProvider {
    pub fn new_static_token(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        access_token: impl Into<String>,
    ) -> Self {
        Self::new_managed_token(
            provider_id,
            client,
            base_url,
            default_model,
            ManagedCredential::static_value("google-vertex access token", access_token.into()),
        )
    }

    pub fn new_managed_token(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        access_token: ManagedCredential,
    ) -> Self {
        let provider_id = provider_id.into();
        Self {
            id: provider_id,
            client,
            base_url: crate::provider::utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth: access_token,
        }
    }

    pub fn new_adc(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        let provider_id = provider_id.into();
        Self {
            id: provider_id.clone(),
            client,
            base_url: crate::provider::utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth: ManagedCredential::google_adc(format!("{provider_id} google adc"), provider_id),
        }
    }

    async fn auth_token(&self) -> Result<String, AppError> {
        self.auth.resolve().await
    }

    fn provider_with_token(&self, token: String) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(
            self.id.clone(),
            self.client.clone(),
            token,
            self.base_url.clone(),
            self.default_model.clone(),
        )
    }
}

#[async_trait]
impl ModelProvider for GoogleVertexProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry()
            .capabilities_for_family(crate::provider::CapabilityFamily::Gemini, model.as_str())
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        crate::provider::default_model_metadata_registry()
            .metadata_for_family(crate::provider::CapabilityFamily::Gemini, model.as_str())
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(self.id.as_str())
                .with_string("auth_scope", self.auth.prompt_cache_scope())
                .with_string("base_url", self.base_url.as_str())
                .with_string("default_model", self.default_model.as_str()),
        )
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        let token = self.auth_token().await?;
        self.provider_with_token(token).list_models().await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let token = self.auth_token().await?;
        self.provider_with_token(token).complete(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let token = self.auth_token().await?;
        self.provider_with_token(token)
            .complete_stream(request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_static_token_uses_static_auth_credential() {
        let provider = GoogleVertexProvider::new_static_token(
            "google-vertex",
            reqwest::Client::new(),
            "https://example.com/openapi",
            "google/gemini-2.5-flash",
            "token",
        );
        assert_eq!(provider.auth_token().await.expect("token"), "token");
    }

    #[test]
    fn prompt_cache_shape_changes_when_auth_scope_changes() {
        let provider_a = GoogleVertexProvider::new_managed_token(
            "google-vertex",
            reqwest::Client::new(),
            "https://example.com/openapi",
            "google/gemini-2.5-flash",
            ManagedCredential::environment(
                "vertex env",
                "google-vertex",
                "access_token",
                "VERTEX_TOKEN_A",
            ),
        );
        let provider_b = GoogleVertexProvider::new_managed_token(
            "google-vertex",
            reqwest::Client::new(),
            "https://example.com/openapi",
            "google/gemini-2.5-flash",
            ManagedCredential::environment(
                "vertex env",
                "google-vertex",
                "access_token",
                "VERTEX_TOKEN_B",
            ),
        );

        let shape_a = provider_a
            .prompt_cache_shape(&ModelId::new("google/gemini-2.5-flash"))
            .expect("shape should exist");
        let shape_b = provider_b
            .prompt_cache_shape(&ModelId::new("google/gemini-2.5-flash"))
            .expect("shape should exist");

        assert_ne!(shape_a.fingerprint(), shape_b.fingerprint());
    }
}
