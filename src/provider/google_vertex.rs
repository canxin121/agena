use async_trait::async_trait;
use futures_core::Stream;

use crate::{
    auth::AuthData,
    error::AppError,
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelProvider,
        OpenAiCompatibleProvider, ProviderModel,
    },
};

const PROVIDER_ID: &str = "google-vertex";
const PROVIDER_ID_ANTHROPIC: &str = "google-vertex-anthropic";
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
    pub fn from_env_and_auth(
        provider_id: &str,
        client: reqwest::Client,
        auth: Option<&AuthData>,
    ) -> Result<Option<Self>, AppError> {
        if provider_id != PROVIDER_ID && provider_id != PROVIDER_ID_ANTHROPIC {
            return Err(AppError::Config(format!(
                "unsupported google vertex provider id: {provider_id}"
            )));
        }

        let project = scoped_env_non_empty(provider_id, "PROJECT").or_else(|| {
            first_non_empty_env(&["GOOGLE_CLOUD_PROJECT", "GCP_PROJECT", "GCLOUD_PROJECT"])
        });

        let Some(project) = project else {
            return Ok(None);
        };

        let location = scoped_env_non_empty(provider_id, "LOCATION")
            .or_else(|| first_non_empty_env(&["GOOGLE_CLOUD_LOCATION", "VERTEX_LOCATION"]))
            .unwrap_or_else(|| {
                if provider_id == PROVIDER_ID_ANTHROPIC {
                    "global".to_owned()
                } else {
                    "us-central1".to_owned()
                }
            });

        let explicit_token = scoped_env_non_empty(provider_id, "ACCESS_TOKEN")
            .or_else(|| scoped_env_non_empty(provider_id, "API_KEY"))
            .or_else(|| env_non_empty("GOOGLE_VERTEX_ACCESS_TOKEN"))
            .or_else(|| auth.and_then(AuthData::api_key).map(ToOwned::to_owned));

        let endpoint = if location == "global" {
            "aiplatform.googleapis.com".to_owned()
        } else {
            format!("{location}-aiplatform.googleapis.com")
        };

        let base_url = scoped_env_non_empty(provider_id, "BASE_URL").unwrap_or_else(|| {
            format!(
                "https://{endpoint}/v1/projects/{project}/locations/{location}/endpoints/openapi"
            )
        });

        let default_model = scoped_env_non_empty(provider_id, "MODEL").unwrap_or_else(|| {
            if provider_id == PROVIDER_ID_ANTHROPIC {
                "anthropic/claude-sonnet-4@20250514".to_owned()
            } else {
                "google/gemini-2.5-flash".to_owned()
            }
        });

        Ok(Some(Self {
            id: provider_id.to_owned(),
            client,
            base_url,
            default_model,
            auth: resolve_auth(explicit_token),
        }))
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

fn resolve_auth(explicit_token: Option<String>) -> GoogleVertexAuth {
    if let Some(token) = explicit_token {
        return GoogleVertexAuth::Static(token);
    }
    GoogleVertexAuth::Adc
}

fn scoped_env_non_empty(provider_id: &str, suffix: &str) -> Option<String> {
    let normalized = provider_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    let mut keys = vec![format!("AGENA_PROVIDER_{normalized}_{suffix}")];
    keys.push(format!("{normalized}_{suffix}"));

    keys.into_iter().find_map(|k| env_non_empty(k.as_str()))
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| env_non_empty(key))
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_auth_prefers_static_token() {
        assert_eq!(
            resolve_auth(Some("token".to_owned())),
            GoogleVertexAuth::Static("token".to_owned())
        );
    }

    #[test]
    fn resolve_auth_falls_back_to_adc() {
        assert_eq!(resolve_auth(None), GoogleVertexAuth::Adc);
    }
}
