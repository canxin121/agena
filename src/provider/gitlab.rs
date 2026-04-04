use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    error::AppError,
    model::{ModelId, ProviderId},
    provider::{
        AnthropicProvider, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        ManagedCredential, ModelProvider, OpenAiCompatibleProvider, OpenAiProvider, ProviderModel,
        auth::AuthData, should_retry_credential, utils,
    },
};

const PROVIDER_ID: &str = "gitlab";
const DEFAULT_INSTANCE_URL: &str = "https://gitlab.com";
const DEFAULT_AI_GATEWAY_URL: &str = "https://cloud.gitlab.com";
const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
const DIRECT_ACCESS_CACHE_TTL: Duration = Duration::from_secs(25 * 60);

#[derive(Debug, Clone)]
pub struct GitlabProviderConfig {
    pub instance_url: String,
    pub ai_gateway_url: String,
    pub default_model: String,
    pub ai_gateway_headers: HashMap<String, String>,
    pub feature_flags: HashMap<String, bool>,
}

impl Default for GitlabProviderConfig {
    fn default() -> Self {
        Self {
            instance_url: DEFAULT_INSTANCE_URL.to_owned(),
            ai_gateway_url: DEFAULT_AI_GATEWAY_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            ai_gateway_headers: HashMap::from([
                ("User-Agent".to_owned(), "agena/0.1.0".to_owned()),
                (
                    "anthropic-beta".to_owned(),
                    "context-1m-2025-08-07".to_owned(),
                ),
            ]),
            feature_flags: HashMap::from([
                ("duo_agent_platform_agentic_chat".to_owned(), true),
                ("duo_agent_platform".to_owned(), true),
            ]),
        }
    }
}

pub struct GitlabProvider {
    client: reqwest::Client,
    api_key: ManagedCredential,
    instance_url: String,
    ai_gateway_url: String,
    default_model: ModelId,
    ai_gateway_headers: HashMap<String, String>,
    feature_flags: HashMap<String, bool>,
    direct_access_cache: Mutex<Option<DirectAccessToken>>,
}

impl GitlabProvider {
    pub fn from_auth(client: reqwest::Client, auth: &AuthData) -> Result<Self, AppError> {
        Self::from_auth_with_config(client, auth, GitlabProviderConfig::default())
    }

    pub fn from_auth_with_instance(
        client: reqwest::Client,
        auth: &AuthData,
        instance_url: Option<String>,
    ) -> Result<Self, AppError> {
        let mut config = GitlabProviderConfig::default();
        if let Some(instance_url) = instance_url {
            config.instance_url = instance_url;
        }
        Self::from_auth_with_config(client, auth, config)
    }

    pub fn from_auth_with_config(
        client: reqwest::Client,
        auth: &AuthData,
        config: GitlabProviderConfig,
    ) -> Result<Self, AppError> {
        let token = match auth {
            AuthData::OAuth { access, .. } => access.clone(),
            AuthData::Api { key } => key.clone(),
            AuthData::WellKnown { key, .. } => key.clone(),
        };

        Self::from_token_with_config(client, token, config)
    }

    pub fn from_token(
        client: reqwest::Client,
        token: impl Into<String>,
        instance_url: Option<String>,
    ) -> Result<Self, AppError> {
        let mut config = GitlabProviderConfig::default();
        if let Some(instance_url) = instance_url {
            config.instance_url = instance_url;
        }
        Self::from_token_with_config(client, token, config)
    }

    pub fn from_token_with_urls(
        client: reqwest::Client,
        token: impl Into<String>,
        instance_url: Option<String>,
        ai_gateway_url: Option<String>,
    ) -> Result<Self, AppError> {
        let mut config = GitlabProviderConfig::default();
        if let Some(instance_url) = instance_url {
            config.instance_url = instance_url;
        }
        if let Some(ai_gateway_url) = ai_gateway_url {
            config.ai_gateway_url = ai_gateway_url;
        }
        Self::from_token_with_config(client, token, config)
    }

    pub fn from_token_with_config(
        client: reqwest::Client,
        token: impl Into<String>,
        config: GitlabProviderConfig,
    ) -> Result<Self, AppError> {
        Self::from_managed_token_with_config(
            client,
            ManagedCredential::static_value("gitlab token", token.into()),
            config,
        )
    }

    pub fn from_managed_token_with_config(
        client: reqwest::Client,
        token: ManagedCredential,
        config: GitlabProviderConfig,
    ) -> Result<Self, AppError> {
        Ok(Self {
            client,
            api_key: token,
            instance_url: normalize_url(config.instance_url),
            ai_gateway_url: normalize_url(config.ai_gateway_url),
            default_model: ModelId::new(config.default_model),
            ai_gateway_headers: config.ai_gateway_headers,
            feature_flags: config.feature_flags,
            direct_access_cache: Mutex::new(None),
        })
    }

    fn direct_access_endpoint(&self) -> String {
        format!(
            "{}/api/v4/ai/third_party_agents/direct_access",
            self.instance_url
        )
    }

    fn openai_proxy_base_url(&self) -> String {
        format!("{}/ai/v1/proxy/openai/v1", self.ai_gateway_url)
    }

    fn anthropic_proxy_base_url(&self) -> String {
        format!("{}/ai/v1/proxy/anthropic/v1", self.ai_gateway_url)
    }

    fn mapped_model(model: &str) -> String {
        match model {
            "duo-chat-opus-4-6" => "claude-opus-4-6".to_owned(),
            "duo-chat-sonnet-4-6" => "claude-sonnet-4-6".to_owned(),
            "duo-chat-opus-4-5" => "claude-opus-4-5-20251101".to_owned(),
            "duo-chat-sonnet-4-5" | "claude-sonnet-4-5" => "claude-sonnet-4-5-20250929".to_owned(),
            "duo-chat-haiku-4-5" => "claude-haiku-4-5-20251001".to_owned(),
            "duo-chat-gpt-5-1" => "gpt-5.1-2025-11-13".to_owned(),
            "duo-chat-gpt-5-2" => "gpt-5.2-2025-12-11".to_owned(),
            "duo-chat-gpt-5-mini" => "gpt-5-mini-2025-08-07".to_owned(),
            "duo-chat-gpt-5-codex" => "gpt-5-codex".to_owned(),
            "duo-chat-gpt-5-2-codex" => "gpt-5.2-codex".to_owned(),
            _ => model.to_owned(),
        }
    }

    fn use_openai_backend(model: &str) -> bool {
        let model = model.to_ascii_lowercase();
        !model.contains("claude")
    }

    fn use_responses_api(model: &str) -> bool {
        model.to_ascii_lowercase().contains("codex")
    }

    async fn get_direct_access_token(
        &self,
        force_refresh: bool,
    ) -> Result<DirectAccessToken, AppError> {
        if !force_refresh {
            let cached = self
                .direct_access_cache
                .lock()
                .map_err(|_| {
                    AppError::Internal("gitlab direct access cache lock poisoned".to_owned())
                })?
                .clone();
            if let Some(cached) = cached {
                if cached.expires_at_ms > now_ms() {
                    return Ok(cached);
                }
            }
        }

        let body = if self.feature_flags.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "feature_flags": self.feature_flags })
        };

        let mut force_credential_refresh = force_refresh;
        let response = loop {
            let api_key = if force_credential_refresh {
                self.api_key.force_refresh().await?
            } else {
                self.api_key.resolve().await?
            };

            let response = self
                .client
                .post(self.direct_access_endpoint())
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await?;

            if !force_credential_refresh && should_retry_credential(response.status()) {
                force_credential_refresh = true;
                continue;
            }

            break response;
        };

        let mut parsed: DirectAccessResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        parsed
            .headers
            .retain(|key, _| !key.eq_ignore_ascii_case("x-api-key"));
        parsed.headers.extend(self.ai_gateway_headers.clone());

        let expires_at_ms = direct_access_expires_at_ms(&parsed)
            .unwrap_or_else(|| now_ms() + DIRECT_ACCESS_CACHE_TTL.as_millis() as i64);

        let cached = DirectAccessToken {
            token: parsed.token,
            headers: parsed.headers,
            expires_at_ms,
        };

        *self.direct_access_cache.lock().map_err(|_| {
            AppError::Internal("gitlab direct access cache lock poisoned".to_owned())
        })? = Some(cached.clone());

        Ok(cached)
    }

    async fn complete_via_backend(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        match self.complete_via_backend_once(request.clone(), false).await {
            Ok(result) => Ok(result),
            Err(err) if should_retry_backend_auth(&err) => {
                self.invalidate_direct_access_cache()?;
                self.complete_via_backend_once(request, true).await
            }
            Err(err) => Err(err),
        }
    }

    async fn complete_via_backend_once(
        &self,
        request: CompletionRequest,
        force_refresh: bool,
    ) -> Result<CompletionResponse, AppError> {
        let model = Self::mapped_model(request.model.as_str());
        let token = self.get_direct_access_token(force_refresh).await?;

        if Self::use_openai_backend(model.as_str()) {
            if Self::use_responses_api(model.as_str()) {
                let provider = OpenAiProvider::new(
                    self.client.clone(),
                    token.token,
                    self.openai_proxy_base_url(),
                    model,
                )
                .with_extra_headers(token.headers);
                let mut result = provider.complete(request).await?;
                result.provider_id = ProviderId::new(PROVIDER_ID);
                return Ok(result);
            }

            let provider = OpenAiCompatibleProvider::new(
                PROVIDER_ID,
                self.client.clone(),
                token.token,
                self.openai_proxy_base_url(),
                model,
            )
            .with_auth_header("authorization", Some("Bearer"))
            .with_extra_headers(token.headers);
            return provider.complete(request).await;
        }

        let provider = AnthropicProvider::new(
            self.client.clone(),
            token.token,
            self.anthropic_proxy_base_url(),
            model,
        )
        .with_auth_header("authorization", Some("Bearer"))
        .with_extra_headers(token.headers);

        let mut result = provider.complete(request).await?;
        result.provider_id = ProviderId::new(PROVIDER_ID);
        Ok(result)
    }

    async fn complete_stream_via_backend(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        match self
            .complete_stream_via_backend_once(request.clone(), false)
            .await
        {
            Ok(stream) => Ok(stream),
            Err(err) if should_retry_backend_auth(&err) => {
                self.invalidate_direct_access_cache()?;
                self.complete_stream_via_backend_once(request, true).await
            }
            Err(err) => Err(err),
        }
    }

    async fn complete_stream_via_backend_once(
        &self,
        request: CompletionRequest,
        force_refresh: bool,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = Self::mapped_model(request.model.as_str());
        let token = self.get_direct_access_token(force_refresh).await?;

        if Self::use_openai_backend(model.as_str()) {
            if Self::use_responses_api(model.as_str()) {
                let provider = OpenAiProvider::new(
                    self.client.clone(),
                    token.token,
                    self.openai_proxy_base_url(),
                    model,
                )
                .with_extra_headers(token.headers);
                let stream = provider.complete_stream(request).await?;
                let mapped = stream.map(|item| item.map(remap_stream_provider_id));
                return Ok(Box::pin(mapped));
            }

            let provider = OpenAiCompatibleProvider::new(
                PROVIDER_ID,
                self.client.clone(),
                token.token,
                self.openai_proxy_base_url(),
                model,
            )
            .with_auth_header("authorization", Some("Bearer"))
            .with_extra_headers(token.headers);
            return provider.complete_stream(request).await;
        }

        let provider = AnthropicProvider::new(
            self.client.clone(),
            token.token,
            self.anthropic_proxy_base_url(),
            model,
        )
        .with_auth_header("authorization", Some("Bearer"))
        .with_extra_headers(token.headers);

        let stream = provider.complete_stream(request).await?;
        let mapped = stream.map(|item| item.map(remap_stream_provider_id));
        Ok(Box::pin(mapped))
    }

    fn invalidate_direct_access_cache(&self) -> Result<(), AppError> {
        *self.direct_access_cache.lock().map_err(|_| {
            AppError::Internal("gitlab direct access cache lock poisoned".to_owned())
        })? = None;
        Ok(())
    }
}

#[async_trait]
impl ModelProvider for GitlabProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        crate::provider::default_capability_registry()
            .capabilities_for_family(crate::provider::CapabilityFamily::Gitlab, model.as_str())
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        crate::provider::default_model_metadata_registry()
            .metadata_for_family(crate::provider::CapabilityFamily::Gitlab, model.as_str())
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new(PROVIDER_ID, self.default_model.clone())
                .with_display_name("GitLab Duo model")
                .with_capabilities(self.model_capabilities(&self.default_model)),
        ])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.complete_via_backend(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        self.complete_stream_via_backend(request).await
    }
}

#[derive(Debug, Clone)]
struct DirectAccessToken {
    token: String,
    headers: HashMap<String, String>,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct DirectAccessResponse {
    token: String,
    headers: HashMap<String, String>,
    #[serde(default, alias = "expiresIn")]
    expires_in: Option<u64>,
    #[serde(default, alias = "expiresAt")]
    expires_at: Option<serde_json::Value>,
    #[serde(default, alias = "expiresAtMs")]
    expires_at_ms: Option<i64>,
}

fn remap_stream_provider_id(event: CompletionStreamEvent) -> CompletionStreamEvent {
    let provider_id = ProviderId::new(PROVIDER_ID);
    match event {
        CompletionStreamEvent::TextDelta { model, delta, .. } => CompletionStreamEvent::TextDelta {
            provider_id: provider_id.clone(),
            model,
            delta,
        },
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
            provider_id,
            model,
            finish_reason,
            usage,
            provider_metadata,
        },
    }
}

fn should_retry_backend_auth(err: &AppError) -> bool {
    match err {
        AppError::HttpStatus { status, .. } => should_retry_credential(*status),
        _ => false,
    }
}

fn normalize_url(value: String) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn direct_access_expires_at_ms(value: &DirectAccessResponse) -> Option<i64> {
    if let Some(ms) = value.expires_at_ms.and_then(normalize_unix_timestamp_ms) {
        return Some(ms);
    }

    if let Some(seconds) = value.expires_in.filter(|seconds| *seconds > 0) {
        return Some(now_ms() + seconds as i64 * 1_000);
    }

    value
        .expires_at
        .as_ref()
        .and_then(direct_access_expires_at_from_value)
}

fn direct_access_expires_at_from_value(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => number.as_i64().and_then(normalize_unix_timestamp_ms),
        serde_json::Value::String(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }

            if let Ok(parsed) = raw.parse::<i64>() {
                return normalize_unix_timestamp_ms(parsed);
            }

            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|datetime| datetime.timestamp_millis())
        }
        _ => None,
    }
}

fn normalize_unix_timestamp_ms(value: i64) -> Option<i64> {
    if value <= 0 {
        return None;
    }

    if value > 10_000_000_000 {
        Some(value)
    } else {
        Some(value * 1_000)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    #[tokio::test]
    async fn complete_stream_uses_direct_access_and_openai_proxy() {
        let mut server = mockito::Server::new_async().await;
        let direct_access = server
            .mock("POST", "/api/v4/ai/third_party_agents/direct_access")
            .match_header("authorization", "Bearer gl-token")
            .match_body(mockito::Matcher::Regex(
                "duo_agent_platform_agentic_chat.*duo_agent_platform|duo_agent_platform.*duo_agent_platform_agentic_chat"
                    .to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "token": "direct-token",
                    "headers": {
                        "x-request-id": "req-1",
                        "x-api-key": "remove-me"
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let stream_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );

        let completions = server
            .mock("POST", "/ai/v1/proxy/openai/v1/chat/completions")
            .match_header("authorization", "Bearer direct-token")
            .match_header("user-agent", "agena/0.1.0")
            .match_header("anthropic-beta", "context-1m-2025-08-07")
            .match_header("x-request-id", "req-1")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(stream_body)
            .create_async()
            .await;

        let provider = GitlabProvider::from_token_with_urls(
            reqwest::Client::new(),
            "gl-token",
            Some(server.url()),
            Some(server.url()),
        )
        .expect("gitlab provider should be created from token");

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: crate::model::ModelId::new("gpt-4o-mini"),
                system: None,
                messages: vec![crate::message::Message::prompt_text(
                    crate::role::Role::User,
                    "hello",
                )],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
            })
            .await
            .expect("stream should start");

        let mut saw_delta = false;
        let mut saw_done = false;
        while let Some(item) = stream.next().await {
            match item.expect("stream item should be ok") {
                CompletionStreamEvent::TextDelta {
                    provider_id, delta, ..
                } => {
                    assert_eq!(provider_id.as_str(), "gitlab");
                    if delta == "Hello" {
                        saw_delta = true;
                    }
                }
                CompletionStreamEvent::Completed { provider_id, .. } => {
                    assert_eq!(provider_id.as_str(), "gitlab");
                    saw_done = true;
                }
                _ => {}
            }
        }

        direct_access.assert();
        completions.assert();
        assert!(saw_delta);
        assert!(saw_done);
    }

    #[test]
    fn direct_access_expiry_prefers_expires_in() {
        let now = now_ms();
        let parsed = DirectAccessResponse {
            token: "direct".to_owned(),
            headers: HashMap::new(),
            expires_in: Some(60),
            expires_at: None,
            expires_at_ms: None,
        };

        let expires_at =
            direct_access_expires_at_ms(&parsed).expect("expires_in should produce expiry");
        assert!(expires_at >= now + 59_000);
    }

    #[test]
    fn direct_access_expiry_supports_rfc3339_expires_at() {
        let parsed = DirectAccessResponse {
            token: "direct".to_owned(),
            headers: HashMap::new(),
            expires_in: None,
            expires_at: Some(serde_json::Value::String("2030-01-01T00:00:00Z".to_owned())),
            expires_at_ms: None,
        };

        let expires_at =
            direct_access_expires_at_ms(&parsed).expect("rfc3339 expires_at should parse");
        assert!(expires_at >= 1_893_456_000_000);
    }
}
