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

const GOOGLE_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, Clone, PartialEq, Eq)]
enum GoogleVertexAuth {
    Static(String),
    Adc,
}

#[derive(Clone)]
pub struct GoogleVertexProvider {
    id: String,
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
    auth: GoogleVertexAuth,
}

impl GoogleVertexProvider {
    pub fn new_static_token(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        access_token: impl Into<String>,
    ) -> Self {
        Self {
            id: provider_id.into(),
            client,
            base_url: crate::provider::utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth: GoogleVertexAuth::Static(access_token.into()),
        }
    }

    pub fn new_adc(
        provider_id: impl Into<String>,
        client: reqwest::Client,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            id: provider_id.into(),
            client,
            base_url: crate::provider::utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            auth: GoogleVertexAuth::Adc,
        }
    }

    async fn auth_token(&self) -> Result<String, AppError> {
        match &self.auth {
            GoogleVertexAuth::Static(token) => Ok(token.clone()),
            GoogleVertexAuth::Adc => {
                let provider = gcp_auth::provider().await.map_err(|err| {
                    AppError::Config(format!(
                        "{} requires Google ADC credentials: {err}",
                        self.id
                    ))
                })?;

                let token = provider
                    .token(&[GOOGLE_CLOUD_PLATFORM_SCOPE])
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "{} failed to obtain Google ADC access token: {err}",
                            self.id
                        ))
                    })?;
                Ok(token.as_str().to_owned())
            }
        }
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

    #[test]
    fn new_static_token_configures_static_auth() {
        let provider = GoogleVertexProvider::new_static_token(
            "google-vertex",
            reqwest::Client::new(),
            "https://example.com/openapi",
            "google/gemini-2.5-flash",
            "token",
        );
        assert_eq!(provider.auth, GoogleVertexAuth::Static("token".to_owned()));
    }

    #[test]
    fn new_adc_configures_adc_auth() {
        let provider = GoogleVertexProvider::new_adc(
            "google-vertex",
            reqwest::Client::new(),
            "https://example.com/openapi",
            "google/gemini-2.5-flash",
        );
        assert_eq!(provider.auth, GoogleVertexAuth::Adc);
    }
}
