use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::core::remap_stream_event_provider_id;
use crate::{
    error::AppError,
    model::{ModelId, ProviderId},
    model_catalog::canonical_model_catalog_id,
    provider::{
        AnthropicAdapter, AnthropicAdapterOptions, CapabilityFamily, CompletionRequest,
        CompletionResponse, CompletionStreamEvent, ManagedCredential, ModelRuntime, OpenAiAdapter,
        OpenAiAdapterOptions, OpenAiApiMode, ProviderModel, auth::AuthData,
        should_retry_credential, utils,
    },
};

const PROVIDER_ID: &str = "gitlab";
const ADAPTER_KIND: &str = "gitlab";
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
            ai_gateway_headers: default_ai_gateway_headers(),
            feature_flags: default_feature_flags(),
        }
    }
}

pub(crate) fn default_ai_gateway_headers() -> HashMap<String, String> {
    HashMap::from([
        (
            "User-Agent".to_owned(),
            crate::provider::claude_code_api_user_agent(),
        ),
        (
            "anthropic-beta".to_owned(),
            "context-1m-2025-08-07".to_owned(),
        ),
    ])
}

pub(crate) fn default_feature_flags() -> HashMap<String, bool> {
    HashMap::from([
        ("duo_agent_platform_agentic_chat".to_owned(), true),
        ("duo_agent_platform".to_owned(), true),
    ])
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
        let config = GitlabProviderConfig {
            instance_url: instance_url.unwrap_or_else(|| DEFAULT_INSTANCE_URL.to_owned()),
            ai_gateway_url: DEFAULT_AI_GATEWAY_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            ai_gateway_headers: default_ai_gateway_headers(),
            feature_flags: default_feature_flags(),
        };
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
        let config = GitlabProviderConfig {
            instance_url: instance_url.unwrap_or_else(|| DEFAULT_INSTANCE_URL.to_owned()),
            ai_gateway_url: DEFAULT_AI_GATEWAY_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            ai_gateway_headers: default_ai_gateway_headers(),
            feature_flags: default_feature_flags(),
        };
        Self::from_token_with_config(client, token, config)
    }

    pub fn from_token_with_urls(
        client: reqwest::Client,
        token: impl Into<String>,
        instance_url: Option<String>,
        ai_gateway_url: Option<String>,
    ) -> Result<Self, AppError> {
        let config = GitlabProviderConfig {
            instance_url: instance_url.unwrap_or_else(|| DEFAULT_INSTANCE_URL.to_owned()),
            ai_gateway_url: ai_gateway_url.unwrap_or_else(|| DEFAULT_AI_GATEWAY_URL.to_owned()),
            default_model: DEFAULT_MODEL.to_owned(),
            ai_gateway_headers: default_ai_gateway_headers(),
            feature_flags: default_feature_flags(),
        };
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

    pub(crate) fn mapped_model(model: &str) -> String {
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

    pub(crate) fn use_openai_backend(model: &str) -> bool {
        let model = model.to_ascii_lowercase();
        !model.contains("claude")
    }

    fn use_responses_api(model: &str) -> bool {
        model.to_ascii_lowercase().contains("codex")
    }

    fn listed_model_id(model_id: &str) -> String {
        canonical_model_catalog_id(model_id.trim().trim_start_matches("gitlab/"))
    }

    fn upsert_listed_model(
        &self,
        models: &mut BTreeMap<String, ProviderModel>,
        raw_model_id: &str,
        display_name: Option<String>,
    ) {
        let model_id = Self::listed_model_id(raw_model_id);
        if model_id.trim().is_empty() {
            return;
        }
        let model_id = ModelId::new(model_id);
        let capabilities = self.model_capabilities(&model_id);
        let metadata = self.model_metadata(&model_id);
        let display_name = display_name.filter(|value| !value.trim().is_empty());
        let model = ProviderModel {
            provider_id: ProviderId::new(PROVIDER_ID),
            adapter_id: None,
            id: model_id.clone(),
            catalog_model_id: None,
            display_name,
            capabilities,
            metadata,
            thinking_modes: std::collections::BTreeMap::new(),
            speed_modes: std::collections::BTreeMap::new(),
        };
        models.entry(model_id.to_string()).or_insert(model);
    }

    fn apply_direct_access_headers(
        &self,
        request: reqwest::RequestBuilder,
        token: &DirectAccessToken,
    ) -> reqwest::RequestBuilder {
        let mut request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token.token),
        );
        for (key, value) in &token.headers {
            request = request.header(key.as_str(), value.as_str());
        }
        request
    }

    async fn fetch_openai_proxy_models(
        &self,
        token: &DirectAccessToken,
    ) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = format!("{}/models", self.openai_proxy_base_url());
        let mut headers = BTreeMap::from([(
            reqwest::header::AUTHORIZATION.as_str().to_owned(),
            format!("Bearer {}", token.token),
        )]);
        headers.extend(token.headers.clone());
        utils::adapter_log_http_request_json(
            PROVIDER_ID,
            ADAPTER_KIND,
            "list_models.openai_proxy",
            "GET",
            endpoint.as_str(),
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            None,
        );
        let response = self
            .apply_direct_access_headers(self.client.get(endpoint.as_str()), token)
            .send()
            .await?;
        let payload: OpenAiModelListResponse = utils::parse_json_response_logged(
            PROVIDER_ID,
            ADAPTER_KIND,
            "list_models.openai_proxy",
            response,
        )
        .await?;
        let mut models = BTreeMap::new();
        for item in payload.into_items() {
            self.upsert_listed_model(&mut models, item.id.as_str(), item.name);
        }
        Ok(models.into_values().collect())
    }

    async fn fetch_anthropic_proxy_models(
        &self,
        token: &DirectAccessToken,
    ) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = format!("{}/models", self.anthropic_proxy_base_url());
        let mut headers = BTreeMap::from([(
            reqwest::header::AUTHORIZATION.as_str().to_owned(),
            format!("Bearer {}", token.token),
        )]);
        headers.extend(token.headers.clone());
        utils::adapter_log_http_request_json(
            PROVIDER_ID,
            ADAPTER_KIND,
            "list_models.anthropic_proxy",
            "GET",
            endpoint.as_str(),
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            None,
        );
        let response = self
            .apply_direct_access_headers(self.client.get(endpoint.as_str()), token)
            .send()
            .await?;
        let payload: AnthropicModelListResponse = utils::parse_json_response_logged(
            PROVIDER_ID,
            ADAPTER_KIND,
            "list_models.anthropic_proxy",
            response,
        )
        .await?;
        let mut models = BTreeMap::new();
        for item in payload.into_items() {
            let display_name = item.display_name.or(item.name);
            self.upsert_listed_model(&mut models, item.id.as_str(), display_name);
        }
        Ok(models.into_values().collect())
    }

    async fn fetch_proxy_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let token = self.get_direct_access_token(false).await?;
        let mut models = BTreeMap::new();
        let mut errors = Vec::new();

        match self.fetch_openai_proxy_models(&token).await {
            Ok(openai_models) => {
                for model in openai_models {
                    models.entry(model.id.to_string()).or_insert(model);
                }
            }
            Err(error) => errors.push(format!("openai proxy: {error}")),
        }

        match self.fetch_anthropic_proxy_models(&token).await {
            Ok(anthropic_models) => {
                for model in anthropic_models {
                    models.entry(model.id.to_string()).or_insert(model);
                }
            }
            Err(error) => errors.push(format!("anthropic proxy: {error}")),
        }

        if models.is_empty() {
            let detail = if errors.is_empty() {
                "GitLab AI Gateway returned no model lists".to_owned()
            } else {
                errors.join("; ")
            };
            return Err(AppError::Provider(format!(
                "gitlab model discovery failed: {detail}"
            )));
        }

        Ok(models.into_values().collect())
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
            if let Some(cached) = cached
                && cached.expires_at_ms > now_ms()
            {
                return Ok(cached);
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

            let endpoint = self.direct_access_endpoint();
            let headers = [
                (
                    reqwest::header::AUTHORIZATION.as_str(),
                    format!("Bearer {api_key}"),
                ),
                (
                    reqwest::header::CONTENT_TYPE.as_str(),
                    "application/json".to_owned(),
                ),
            ];
            utils::adapter_log_http_request_json(
                PROVIDER_ID,
                ADAPTER_KIND,
                "direct_access.token",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (*k, v.as_str())),
                Some(&body),
            );
            let response = self
                .client
                .post(endpoint)
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

        let mut parsed: DirectAccessResponse = utils::parse_json_response_logged(
            PROVIDER_ID,
            ADAPTER_KIND,
            "direct_access.token",
            response,
        )
        .await?;
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
        let model = Self::mapped_model(request.model.as_ref());
        let token = self.get_direct_access_token(force_refresh).await?;

        if Self::use_openai_backend(model.as_str()) {
            if Self::use_responses_api(model.as_str()) {
                let provider = OpenAiAdapter::new_managed_with_options(
                    "openai",
                    self.client.clone(),
                    ManagedCredential::static_value("openai api key", token.token),
                    self.openai_proxy_base_url(),
                    model,
                    OpenAiAdapterOptions {
                        extra_headers: token.headers,
                        ..OpenAiAdapterOptions::default()
                    },
                );
                let mut result = provider.complete(request).await?;
                result.provider_id = ProviderId::new(PROVIDER_ID);
                return Ok(result);
            }

            let provider = OpenAiAdapter::new_managed_with_options(
                PROVIDER_ID,
                self.client.clone(),
                ManagedCredential::static_value("openai api key", token.token),
                self.openai_proxy_base_url(),
                model,
                OpenAiAdapterOptions {
                    api_mode: OpenAiApiMode::Chat,
                    capability_family: CapabilityFamily::OpenAiCompatible,
                    auth_header: "authorization".to_owned(),
                    auth_scheme: Some("Bearer".to_owned()),
                    extra_headers: token.headers,
                    ..OpenAiAdapterOptions::default()
                },
            );
            return provider.complete(request).await;
        }

        let provider = AnthropicAdapter::new_managed_with_options(
            "anthropic",
            self.client.clone(),
            ManagedCredential::static_value("anthropic api key", token.token),
            self.anthropic_proxy_base_url(),
            model,
            AnthropicAdapterOptions {
                auth_header: "authorization".to_owned(),
                auth_scheme: Some("Bearer".to_owned()),
                extra_headers: token.headers,
                ..AnthropicAdapterOptions::default()
            },
        );

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
        let model = Self::mapped_model(request.model.as_ref());
        let token = self.get_direct_access_token(force_refresh).await?;

        if Self::use_openai_backend(model.as_str()) {
            if Self::use_responses_api(model.as_str()) {
                let provider = OpenAiAdapter::new_managed_with_options(
                    "openai",
                    self.client.clone(),
                    ManagedCredential::static_value("openai api key", token.token),
                    self.openai_proxy_base_url(),
                    model,
                    OpenAiAdapterOptions {
                        extra_headers: token.headers,
                        ..OpenAiAdapterOptions::default()
                    },
                );
                let stream = provider.complete_stream(request).await?;
                let provider_id = ProviderId::new(PROVIDER_ID);
                let mapped = stream.map(move |item| {
                    let provider_id = provider_id.clone();
                    item.map(|event| remap_stream_event_provider_id(&provider_id, event))
                });
                return Ok(Box::pin(mapped));
            }

            let provider = OpenAiAdapter::new_managed_with_options(
                PROVIDER_ID,
                self.client.clone(),
                ManagedCredential::static_value("openai api key", token.token),
                self.openai_proxy_base_url(),
                model,
                OpenAiAdapterOptions {
                    api_mode: OpenAiApiMode::Chat,
                    capability_family: CapabilityFamily::OpenAiCompatible,
                    auth_header: "authorization".to_owned(),
                    auth_scheme: Some("Bearer".to_owned()),
                    extra_headers: token.headers,
                    ..OpenAiAdapterOptions::default()
                },
            );
            return provider.complete_stream(request).await;
        }

        let provider = AnthropicAdapter::new_managed_with_options(
            "anthropic",
            self.client.clone(),
            ManagedCredential::static_value("anthropic api key", token.token),
            self.anthropic_proxy_base_url(),
            model,
            AnthropicAdapterOptions {
                auth_header: "authorization".to_owned(),
                auth_scheme: Some("Bearer".to_owned()),
                extra_headers: token.headers,
                ..AnthropicAdapterOptions::default()
            },
        );

        let stream = provider.complete_stream(request).await?;
        let provider_id = ProviderId::new(PROVIDER_ID);
        let mapped = stream.map(move |item| {
            let provider_id = provider_id.clone();
            item.map(|event| remap_stream_event_provider_id(&provider_id, event))
        });
        Ok(Box::pin(mapped))
    }

    fn invalidate_direct_access_cache(&self) -> Result<(), AppError> {
        *self.direct_access_cache.lock().map_err(|_| {
            AppError::Internal("gitlab direct access cache lock poisoned".to_owned())
        })? = None;
        Ok(())
    }

    fn prompt_cache_direct_access_shape(&self) -> Option<crate::provider::PromptCacheShape> {
        let cached = self.direct_access_cache.lock().ok()?.clone()?;
        if cached.expires_at_ms <= now_ms() {
            return None;
        }

        let route_headers = utils::prompt_cache_header_entries(&cached.headers);
        let fields = (!route_headers.is_empty()).then(|| {
            (
                "direct_access_route_headers",
                crate::provider::PromptCacheShape::json_field_value(&route_headers),
            )
        });
        Some(crate::provider::PromptCacheShape::from_fields(
            PROVIDER_ID,
            fields,
        ))
    }
}

#[async_trait]
impl ModelRuntime for GitlabProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(crate::provider::CapabilityFamily::Gitlab)
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        let _ = model;
        false
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let mapped_model = Self::mapped_model(model.as_ref());
        let mut feature_flags = self
            .feature_flags
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect::<Vec<_>>();
        feature_flags.sort_unstable_by(|left, right| left.0.cmp(&right.0));

        let runtime_shape = self.prompt_cache_direct_access_shape();
        let mut shape = crate::provider::PromptCacheShape::from_fields(
            PROVIDER_ID,
            [
                ("auth_scope", self.api_key.prompt_cache_scope()),
                ("instance_url", self.instance_url.clone()),
                ("ai_gateway_url", self.ai_gateway_url.clone()),
                (
                    "ai_gateway_headers",
                    crate::provider::PromptCacheShape::json_field_value(
                        &utils::prompt_cache_header_entries(&self.ai_gateway_headers),
                    ),
                ),
                (
                    "feature_flags",
                    crate::provider::PromptCacheShape::json_field_value(&feature_flags),
                ),
                ("mapped_model", mapped_model.as_str().to_owned()),
                (
                    "openai_backend",
                    Self::use_openai_backend(mapped_model.as_str()).to_string(),
                ),
                (
                    "responses_api",
                    Self::use_responses_api(mapped_model.as_str()).to_string(),
                ),
                ("direct_access_cached", runtime_shape.is_some().to_string()),
            ],
        );
        if let Some(runtime_shape) = runtime_shape {
            shape.extend_prefixed("runtime", &runtime_shape);
        }

        Some(shape)
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        self.fetch_proxy_models().await
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
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    expires_at: Option<serde_json::Value>,
    #[serde(default)]
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiModelListResponse {
    Wrapped { data: Vec<OpenAiModel> },
    Bare(Vec<OpenAiModel>),
}

impl OpenAiModelListResponse {
    fn into_items(self) -> Vec<OpenAiModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnthropicModelListResponse {
    Wrapped { data: Vec<AnthropicModel> },
    Bare(Vec<AnthropicModel>),
}

impl AnthropicModelListResponse {
    fn into_items(self) -> Vec<AnthropicModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
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
