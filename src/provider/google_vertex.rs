use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    error::AppError,
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelProvider,
        OpenAiCompatibleProvider, ProviderModel, StreamResumePolicy,
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
    default_model: String,
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
            default_model: default_model.into(),
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
            default_model: default_model.into(),
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

    fn default_model(&self) -> &str {
        self.default_model.as_str()
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
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
