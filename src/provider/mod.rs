mod amazon_bedrock;
mod anthropic;
mod catalog;
mod cloudflare_ai_gateway;
mod codex;
mod copilot;
mod gemini;
mod gitlab;
mod google_vertex;
mod openai;
mod openai_compatible;
mod sse;
mod types;
mod utils;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;

use crate::{
    auth::{AuthData, AuthStore, FileAuthStore},
    error::AppError,
};

pub use amazon_bedrock::AmazonBedrockProvider;
pub use anthropic::AnthropicProvider;
pub use cloudflare_ai_gateway::CloudflareAiGatewayProvider;
pub use codex::CodexProvider;
pub use copilot::CopilotProvider;
pub use gemini::GeminiProvider;
pub use gitlab::GitlabProvider;
pub use google_vertex::GoogleVertexProvider;
pub use openai::OpenAiProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use types::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage, ProviderContent, ProviderContentPart, ProviderMessage,
    ProviderModel,
};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &str;
    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError>;
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

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn with_defaults_from_env() -> Result<Self, AppError> {
        let store: Arc<dyn AuthStore> = Arc::new(FileAuthStore::new(FileAuthStore::default_path()));
        Self::with_defaults_from_env_and_auth_store(store)
    }

    pub fn with_defaults_from_env_and_auth_store(
        store: Arc<dyn AuthStore>,
    ) -> Result<Self, AppError> {
        let client = reqwest::Client::builder().build()?;
        let mut registry = Self::new();
        let auth_all = store.all()?;

        if env_has_non_empty("OPENAI_API_KEY") {
            registry.register(OpenAiProvider::from_env(client.clone())?);
        } else if let Some(auth) = auth_all.get("openai") {
            match auth {
                AuthData::Api { key } | AuthData::WellKnown { key, .. } => {
                    registry.register(OpenAiProvider::new(
                        client.clone(),
                        key,
                        std::env::var("OPENAI_BASE_URL")
                            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned()),
                        std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4.1-mini".to_owned()),
                    ));
                }
                AuthData::OAuth { .. } => {
                    registry.register(CodexProvider::from_auth(
                        client.clone(),
                        Arc::clone(&store),
                        auth,
                    )?);
                }
            }
        }

        if env_has_non_empty("ANTHROPIC_API_KEY") {
            registry.register(AnthropicProvider::from_env(client.clone())?);
        } else if let Some(auth) = auth_all.get("anthropic") {
            if let Some(key) = auth.api_key() {
                registry.register(AnthropicProvider::new(
                    client.clone(),
                    key,
                    std::env::var("ANTHROPIC_BASE_URL")
                        .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_owned()),
                    std::env::var("ANTHROPIC_MODEL")
                        .unwrap_or_else(|_| "claude-3-7-sonnet-latest".to_owned()),
                ));
            }
        }

        if env_has_non_empty("GEMINI_API_KEY") || env_has_non_empty("GOOGLE_API_KEY") {
            registry.register(GeminiProvider::from_env(client.clone())?);
        }

        if let Some(provider) = CloudflareAiGatewayProvider::from_env_and_auth(
            client.clone(),
            auth_all.get("cloudflare-ai-gateway"),
        )? {
            registry.register(provider);
        }

        if let Some(provider) = GoogleVertexProvider::from_env_and_auth(
            "google-vertex",
            client.clone(),
            auth_all.get("google-vertex"),
        )? {
            registry.register(provider);
        }

        if let Some(provider) = GoogleVertexProvider::from_env_and_auth(
            "google-vertex-anthropic",
            client.clone(),
            auth_all.get("google-vertex-anthropic"),
        )? {
            registry.register(provider);
        }

        if let Some(provider) = AmazonBedrockProvider::from_env_and_auth(
            client.clone(),
            auth_all.get("amazon-bedrock"),
        )? {
            registry.register(provider);
        }

        for provider_id in catalog::OPENCODE_PROVIDER_IDS {
            if matches!(
                *provider_id,
                "cloudflare-ai-gateway"
                    | "google-vertex"
                    | "google-vertex-anthropic"
                    | "amazon-bedrock"
            ) {
                continue;
            }

            if let Some(provider) = build_opencode_compatible_provider(
                *provider_id,
                client.clone(),
                auth_all.get(*provider_id),
            )? {
                registry.register(provider);
            }
        }

        if let Some(auth) = auth_all.get("github-copilot") {
            registry.register(CopilotProvider::from_auth(
                "github-copilot",
                client.clone(),
                auth,
            )?);
        }

        if let Some(auth) = auth_all.get("github-copilot-enterprise") {
            registry.register(CopilotProvider::from_auth(
                "github-copilot-enterprise",
                client.clone(),
                auth,
            )?);
        }

        let gitlab_instance = auth_all
            .get("gitlab-instance")
            .and_then(AuthData::api_key)
            .map(ToOwned::to_owned)
            .or_else(|| env_non_empty("GITLAB_INSTANCE_URL"));

        if let Some(auth) = auth_all.get("gitlab") {
            registry.register(GitlabProvider::from_auth_with_instance(
                client.clone(),
                auth,
                gitlab_instance,
            )?);
        } else if let Some(token) = env_non_empty("GITLAB_TOKEN") {
            registry.register(GitlabProvider::from_token(
                client.clone(),
                token,
                gitlab_instance,
            )?);
        }

        Ok(registry)
    }

    pub fn register<P>(&mut self, provider: P)
    where
        P: ModelProvider + 'static,
    {
        self.providers
            .insert(provider.id().to_owned(), Arc::new(provider));
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ModelProvider>> {
        self.providers.get(provider_id).cloned()
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<ProviderModel>, AppError> {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        provider.list_models().await
    }

    pub async fn complete(
        &self,
        provider_id: &str,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        provider.complete(request).await
    }

    pub async fn complete_stream(
        &self,
        provider_id: &str,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let provider = self
            .get(provider_id)
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        provider.complete_stream(request).await
    }
}

fn env_has_non_empty(key: &str) -> bool {
    std::env::var(key)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn build_opencode_compatible_provider(
    provider_id: &str,
    client: reqwest::Client,
    auth: Option<&AuthData>,
) -> Result<Option<OpenAiCompatibleProvider>, AppError> {
    let key = provider_env_value(provider_id, "API_KEY")
        .or_else(|| auth.and_then(AuthData::api_key).map(ToOwned::to_owned))
        .or_else(|| provider_default_api_key(provider_id));

    let Some(key) = key else {
        return Ok(None);
    };

    let base_url = provider_env_value(provider_id, "BASE_URL")
        .or_else(|| provider_default_base_url(provider_id))
        .or_else(|| catalog::default_base_url(provider_id).map(ToOwned::to_owned));

    let Some(base_url) = base_url else {
        return Ok(None);
    };

    let default_model = provider_env_value(provider_id, "MODEL")
        .or_else(|| provider_default_model(provider_id))
        .unwrap_or_else(|| "gpt-4.1-mini".to_owned());

    let auth_header = provider_env_value(provider_id, "AUTH_HEADER");
    let auth_scheme = provider_env_value(provider_id, "AUTH_SCHEME");

    let mut provider =
        OpenAiCompatibleProvider::new(provider_id, client, key, base_url, default_model);
    if auth_header.is_some() || auth_scheme.is_some() {
        let resolved_scheme = match auth_scheme {
            Some(scheme) => Some(scheme),
            None => Some("Bearer".to_owned()),
        };
        provider = provider.with_auth_header(
            auth_header.unwrap_or_else(|| "authorization".to_owned()),
            resolved_scheme,
        );
    }

    let mut headers = provider_default_headers(provider_id);
    if let Some(headers_json) = provider_env_value(provider_id, "EXTRA_HEADERS_JSON") {
        headers.extend(utils::parse_headers_json(headers_json.as_str())?);
    }
    if !headers.is_empty() {
        provider = provider.with_extra_headers(headers);
    }

    Ok(Some(provider))
}

fn provider_env_value(provider_id: &str, suffix: &str) -> Option<String> {
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

    // For common providers, also read conventional env keys.
    if suffix == "API_KEY" {
        keys.push(format!("{normalized}_{suffix}"));
    } else if suffix == "BASE_URL" {
        keys.push(format!("{normalized}_{suffix}"));
    } else if suffix == "MODEL" {
        keys.push(format!("{normalized}_{suffix}"));
    }

    keys.into_iter().find_map(|k| {
        std::env::var(&k)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

fn provider_default_api_key(provider_id: &str) -> Option<String> {
    match provider_id {
        "opencode" => Some("public".to_owned()),
        "cloudflare-workers-ai" => env_non_empty("CLOUDFLARE_API_KEY"),
        _ => None,
    }
}

fn provider_default_base_url(provider_id: &str) -> Option<String> {
    match provider_id {
        "azure-cognitive-services" => env_non_empty("AZURE_COGNITIVE_SERVICES_RESOURCE_NAME")
            .map(|resource| format!("https://{resource}.cognitiveservices.azure.com/openai")),
        "cloudflare-workers-ai" => env_non_empty("CLOUDFLARE_ACCOUNT_ID").map(|account_id| {
            format!("https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1")
        }),
        _ => None,
    }
}

fn provider_default_model(provider_id: &str) -> Option<String> {
    match provider_id {
        "deepseek" => Some("deepseek-chat".to_owned()),
        "mistral" => Some("mistral-small-latest".to_owned()),
        "groq" => Some("llama-3.3-70b-versatile".to_owned()),
        "xai" => Some("grok-3-mini".to_owned()),
        "moonshotai" | "moonshotai-cn" | "kimi-for-coding" => Some("kimi-k2-instruct".to_owned()),
        "fireworks-ai" => Some("accounts/fireworks/models/llama-v3p1-8b-instruct".to_owned()),
        "togetherai" => Some("meta-llama/Llama-3.3-70B-Instruct-Turbo".to_owned()),
        "perplexity" => Some("sonar-pro".to_owned()),
        _ => None,
    }
}

fn provider_default_headers(provider_id: &str) -> HashMap<String, String> {
    match provider_id {
        "openrouter" | "zenmux" | "kilo" => HashMap::from([
            ("HTTP-Referer".to_owned(), "https://opencode.ai/".to_owned()),
            ("X-Title".to_owned(), "opencode".to_owned()),
        ]),
        "vercel" => HashMap::from([
            ("http-referer".to_owned(), "https://opencode.ai/".to_owned()),
            ("x-title".to_owned(), "opencode".to_owned()),
        ]),
        "cerebras" => HashMap::from([(
            "X-Cerebras-3rd-Party-Integration".to_owned(),
            "opencode".to_owned(),
        )]),
        _ => HashMap::new(),
    }
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
    use crate::provider::{CompletionRequest, ProviderMessage};

    struct FailingStore;

    impl AuthStore for FailingStore {
        fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
            Err(AppError::Internal("store read failed".to_owned()))
        }

        fn get(&self, _provider_id: &str) -> Result<Option<AuthData>, AppError> {
            Err(AppError::Internal("store read failed".to_owned()))
        }

        fn set(&self, _provider_id: &str, _auth: AuthData) -> Result<(), AppError> {
            Err(AppError::Internal("store write failed".to_owned()))
        }

        fn remove(&self, _provider_id: &str) -> Result<(), AppError> {
            Err(AppError::Internal("store write failed".to_owned()))
        }
    }

    #[test]
    fn with_defaults_from_env_and_auth_store_surfaces_store_errors() {
        let err =
            match ProviderRegistry::with_defaults_from_env_and_auth_store(Arc::new(FailingStore)) {
                Ok(_) => panic!("registry init should fail when auth store fails"),
                Err(err) => err,
            };

        assert!(matches!(
            err,
            AppError::Internal(message) if message == "store read failed"
        ));
    }

    #[test]
    fn provider_default_headers_match_opencode_defaults() {
        let openrouter = provider_default_headers("openrouter");
        assert_eq!(
            openrouter.get("HTTP-Referer").map(String::as_str),
            Some("https://opencode.ai/")
        );
        assert_eq!(
            openrouter.get("X-Title").map(String::as_str),
            Some("opencode")
        );

        let vercel = provider_default_headers("vercel");
        assert_eq!(
            vercel.get("http-referer").map(String::as_str),
            Some("https://opencode.ai/")
        );
        assert_eq!(vercel.get("x-title").map(String::as_str), Some("opencode"));

        let cerebras = provider_default_headers("cerebras");
        assert_eq!(
            cerebras
                .get("X-Cerebras-3rd-Party-Integration")
                .map(String::as_str),
            Some("opencode")
        );
    }

    #[test]
    fn opencode_uses_public_api_key_default() {
        assert_eq!(
            provider_default_api_key("opencode").as_deref(),
            Some("public")
        );
    }

    #[tokio::test]
    async fn compatible_provider_keeps_default_bearer_auth_scheme() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .match_header("authorization", "Bearer sk-test")
            .match_body(mockito::Matcher::Regex(
                "\"model\":\"deepseek-chat\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "deepseek-chat",
                    "choices": [{
                        "message": {"content": "ok"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1}
                })
                .to_string(),
            )
            .create_async()
            .await;

        unsafe {
            std::env::set_var("AGENA_PROVIDER_DEEPSEEK_BASE_URL", server.url());
        }

        let provider = build_opencode_compatible_provider(
            "deepseek",
            reqwest::Client::new(),
            Some(&crate::auth::AuthData::Api {
                key: "sk-test".to_owned(),
            }),
        )
        .expect("provider build should succeed")
        .expect("provider should be created");

        let response = provider
            .complete(CompletionRequest {
                model: String::new(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(16),
            })
            .await
            .expect("completion should succeed");

        unsafe {
            std::env::remove_var("AGENA_PROVIDER_DEEPSEEK_BASE_URL");
        }

        assert_eq!(response.text, "ok");
    }
}
