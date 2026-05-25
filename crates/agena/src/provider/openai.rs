use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::{
    config::{NativeToolFreshness, ProviderNativeToolKind, ProviderNativeToolRoute},
    error::{AppError, ProviderErrorKind},
    message::{AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage},
    model::{
        CapabilitySupport, ModelCapabilities, ModelId, ModelMetadata, ModelThinkingMode, ProviderId,
    },
    provider::{
        CapabilityFamily, CompletionFinishReason, CompletionRequest, CompletionResponse,
        CompletionStreamEvent, CompletionToolCall, CompletionUsage, ManagedCredential,
        ModelRuntime, ProviderModel, StreamResumePolicy,
        auth::AuthData,
        chat_wire::{
            self, ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions,
            tools_to_chat_definitions,
        },
        prompt_cache, should_retry_credential, sse, utils, wire_message,
    },
    role::Role,
};

const CHATGPT_CODEX_ORIGINATOR: &str = crate::provider::CODEX_ORIGINATOR;
const CHATGPT_CODEX_USER_AGENT: &str = crate::provider::CODEX_USER_AGENT;
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const ATOMGIT_CODING_PLAN_MODELS_URL: &str = "https://api.gitcode.com/api/v5/coding-plan/models-v2";
const ADAPTER_KIND: &str = "openai";

#[derive(Clone)]
pub struct OpenAiAdapter {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    backend: OpenAiBackend,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    api_mode: OpenAiApiMode,
    api_mode_explicit: bool,
    profile: OpenAiProfile,
    models_url: Option<String>,
    auth_header: String,
    auth_scheme: Option<String>,
    capability_family: CapabilityFamily,
    extra_headers: HashMap<String, String>,
    stream_mode: OpenAiStreamMode,
    realtime_ws_url: Option<String>,
    top_level_prompt_cache_override: Option<bool>,
    atomgit_coding_plan_models: bool,
}

#[derive(Debug, Default)]
struct OpenAiResponsesToolPlan {
    tools: Vec<serde_json::Value>,
    include: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiMode {
    Responses,
    Chat,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiBackend {
    Api,
    ChatgptCodex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiProfile {
    Standard,
    GithubCopilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiStreamMode {
    Sse,
    RealtimeWebSocket,
}

impl OpenAiAdapter {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed(
            client,
            ManagedCredential::static_value("openai api key", api_key.into()),
            base_url,
            default_model,
        )
    }

    pub fn new_with_id(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed_with_id(
            id,
            client,
            ManagedCredential::static_value("openai api key", api_key.into()),
            base_url,
            default_model,
        )
    }

    pub fn new_managed(
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed_with_id("openai", client, api_key, base_url, default_model)
    }

    pub fn new_managed_with_id(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        let id = id.into();
        Self {
            id,
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            backend: OpenAiBackend::Api,
            auth_data: None,
            api_mode: OpenAiApiMode::Responses,
            api_mode_explicit: false,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::from([(
                reqwest::header::USER_AGENT.as_str().to_owned(),
                crate::provider::CODEX_USER_AGENT.to_owned(),
            )]),
            stream_mode: OpenAiStreamMode::Sse,
            realtime_ws_url: None,
            top_level_prompt_cache_override: None,
            atomgit_coding_plan_models: false,
        }
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_backend(mut self, backend: OpenAiBackend) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_auth_data(mut self, auth_data: Arc<Mutex<AuthData>>) -> Self {
        self.auth_data = Some(auth_data);
        self
    }

    pub fn with_api_mode(mut self, mode: OpenAiApiMode) -> Self {
        self.api_mode = mode;
        self
    }

    pub fn with_api_mode_explicit(mut self, explicit: bool) -> Self {
        self.api_mode_explicit = explicit;
        self
    }

    pub fn with_profile(mut self, profile: OpenAiProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_models_url(mut self, models_url: Option<String>) -> Self {
        self.models_url = models_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    pub fn with_atomgit_coding_plan_models(mut self, enabled: bool) -> Self {
        self.atomgit_coding_plan_models = enabled;
        self
    }

    pub fn with_auth_header(
        mut self,
        header: impl Into<String>,
        scheme: Option<impl Into<String>>,
    ) -> Self {
        self.auth_header = header.into();
        self.auth_scheme = scheme.map(|value| value.into());
        self
    }

    pub fn with_capability_family(mut self, family: CapabilityFamily) -> Self {
        self.capability_family = family;
        self
    }

    pub fn with_stream_mode(mut self, mode: OpenAiStreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    pub fn with_realtime_ws_url(mut self, ws_url: Option<String>) -> Self {
        self.realtime_ws_url = ws_url.and_then(|value| utils::normalize_optional_text(Some(value)));
        self
    }

    pub fn with_top_level_prompt_cache(mut self, enabled: bool) -> Self {
        self.top_level_prompt_cache_override = Some(enabled);
        self
    }

    fn configured_public_copilot_base_url(&self) -> bool {
        self.base_url.trim_end_matches('/') == DEFAULT_COPILOT_BASE_URL
    }

    fn resolved_base_url(&self) -> Result<String, AppError> {
        if self.profile != OpenAiProfile::GithubCopilot
            || !self.configured_public_copilot_base_url()
        {
            return Ok(self.base_url.clone());
        }

        let Some(auth_data) = self.auth_data.as_ref() else {
            return Ok(self.base_url.clone());
        };

        let domain = auth_data
            .try_lock()
            .ok()
            .as_deref()
            .and_then(AuthData::enterprise_url)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                AppError::Config("enterprise_url missing for enterprise copilot auth".to_owned())
            })?;

        Ok(format!("https://copilot-api.{}", normalize_domain(&domain)))
    }

    fn prompt_cache_base_url(&self) -> String {
        self.resolved_base_url()
            .unwrap_or_else(|_| self.base_url.clone())
    }

    fn model_endpoint(&self) -> Result<String, AppError> {
        Ok(self.models_url.clone().unwrap_or_else(|| {
            format!(
                "{}/models",
                self.prompt_cache_base_url().trim_end_matches('/')
            )
        }))
    }

    fn atomgit_coding_plan_models_endpoint(
        &self,
        plan_type: AtomGitCodingPlanType,
    ) -> Result<String, AppError> {
        let base = self
            .models_url
            .clone()
            .unwrap_or_else(|| ATOMGIT_CODING_PLAN_MODELS_URL.to_owned());
        let mut endpoint = url::Url::parse(base.as_str()).map_err(|err| {
            AppError::Config(format!("atomgit coding plan models url is invalid: {err}"))
        })?;
        let existing = endpoint
            .query_pairs()
            .filter(|(key, _)| key != "plan_type")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        {
            let mut query = endpoint.query_pairs_mut();
            query.clear();
            for (key, value) in existing {
                query.append_pair(key.as_str(), value.as_str());
            }
            query.append_pair("plan_type", plan_type.as_str());
        }
        Ok(endpoint.to_string())
    }

    fn atomgit_coding_plan_claim_endpoint(&self) -> Result<String, AppError> {
        let base = self
            .models_url
            .clone()
            .unwrap_or_else(|| ATOMGIT_CODING_PLAN_MODELS_URL.to_owned());
        let mut endpoint = url::Url::parse(base.as_str()).map_err(|err| {
            AppError::Config(format!("atomgit coding plan models url is invalid: {err}"))
        })?;
        let path = endpoint.path().trim_end_matches('/');
        let claim_path = path
            .strip_suffix("/models-v2")
            .map(|prefix| format!("{prefix}/claim-v2"))
            .unwrap_or_else(|| "/api/v5/coding-plan/claim-v2".to_owned());
        endpoint.set_path(claim_path.as_str());
        endpoint.set_query(None);
        Ok(endpoint.to_string())
    }

    async fn atomgit_coding_plan_model_response(&self) -> Result<reqwest::Response, AppError> {
        let mut force_refresh = false;
        loop {
            let api_key = if force_refresh {
                self.api_key.force_refresh().await?
            } else {
                self.api_key.resolve().await?
            };
            let headers = self.auth_headers(RequestHeaderContext::none(), api_key.as_str());
            let plan_type = self.atomgit_claim_coding_plan_type(headers.clone()).await?;
            let endpoint = self.atomgit_coding_plan_models_endpoint(plan_type)?;
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "list_models",
                "GET",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                None,
            );
            let response =
                utils::apply_resolved_request_headers(self.client.get(endpoint.as_str()), &headers)
                    .send()
                    .await?;
            if !force_refresh && should_retry_credential(response.status()) {
                force_refresh = true;
                continue;
            }
            return Ok(response);
        }
    }

    async fn atomgit_claim_coding_plan_type(
        &self,
        headers: BTreeMap<String, String>,
    ) -> Result<AtomGitCodingPlanType, AppError> {
        let endpoint = self.atomgit_coding_plan_claim_endpoint()?;
        let mut last_message = String::new();
        for &plan_type in AtomGitCodingPlanType::CASCADE_ORDER {
            let body = serde_json::json!({ "plan_type": plan_type.as_str() });
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "coding_plan_claim",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(&body),
            );
            let response = utils::apply_resolved_request_headers(
                self.client.post(endpoint.as_str()),
                &headers,
            )
            .json(&body)
            .send()
            .await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(AppError::HttpStatus {
                    provider: self.id.to_string(),
                    status,
                    body: format!("coding-plan/claim-v2 {}: {}", plan_type.as_str(), text),
                    kind: ProviderErrorKind::ApiError,
                    retryable: false,
                });
            }
            let claim: AtomGitCodingPlanClaimResponse =
                serde_json::from_str(text.as_str()).map_err(AppError::from)?;
            if claim.duplicate {
                return Ok(AtomGitCodingPlanType::Max);
            }
            if claim.success {
                return Ok(plan_type);
            }
            last_message = if claim.message.trim().is_empty() {
                format!("{} claim refused", plan_type.as_str())
            } else {
                format!("{}: {}", plan_type.as_str(), claim.message.trim())
            };
        }
        Err(AppError::Provider(if last_message.is_empty() {
            "atomgit coding plan claim failed at every tier".to_owned()
        } else {
            format!("atomgit coding plan claim failed at every tier: {last_message}")
        }))
    }

    fn responses_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/responses",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    fn responses_compact_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/responses/compact",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    fn chat_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/chat/completions",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    fn is_openai_compatible_family(&self) -> bool {
        matches!(self.capability_family, CapabilityFamily::OpenAiCompatible)
    }

    fn uses_chat_compatible_request_fields(&self) -> bool {
        self.is_openai_compatible_family()
    }

    fn supports_top_level_prompt_cache(&self) -> bool {
        if let Some(enabled) = self.top_level_prompt_cache_override {
            return enabled;
        }
        self.uses_chat_compatible_request_fields()
            && matches!(self.id.as_str(), "openrouter" | "zenmux" | "kilo")
    }

    fn is_dashscope_compatible(&self) -> bool {
        self.id.eq_ignore_ascii_case("alibaba-cn")
            || self.id.to_ascii_lowercase().contains("dashscope")
            || url::Url::parse(self.base_url.as_str())
                .ok()
                .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                .is_some_and(|host| host.contains("dashscope") && host.contains("aliyuncs.com"))
    }

    fn dashscope_reasoning_profile(model: &str) -> Option<DashscopeReasoningProfile> {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }
        if normalized.contains("kimi-k2-thinking")
            || normalized.contains(":thinking")
            || normalized.contains("-thinking")
        {
            return Some(DashscopeReasoningProfile::AlwaysOn);
        }
        if normalized.contains("qwen-plus")
            || normalized.contains("qwen3")
            || normalized.contains("qwq")
            || normalized.contains("deepseek-r1")
            || normalized.contains("kimi-k2")
            || normalized.contains("k2p")
            || normalized.contains("k2-5")
            || normalized.contains("qvq")
        {
            return Some(DashscopeReasoningProfile::Toggleable);
        }
        None
    }

    fn is_dashscope_reasoning_model(&self, model: &ModelId) -> bool {
        self.is_openai_compatible_family()
            && self.is_dashscope_compatible()
            && Self::dashscope_reasoning_profile(model.as_str()).is_some()
    }

    fn assistant_reasoning_field_for_model(&self, model: &ModelId) -> Option<&'static str> {
        self.is_dashscope_reasoning_model(model)
            .then_some("reasoning_content")
    }

    fn apply_dashscope_reasoning_overrides(
        &self,
        model: &ModelId,
        thinking: Option<&crate::provider::ThinkingRequest>,
        request_override: &mut crate::model::ModelSpeedModeRequestOverride,
    ) {
        if !self.is_dashscope_compatible() {
            return;
        }

        let Some(profile) = Self::dashscope_reasoning_profile(model.as_str()) else {
            return;
        };

        match thinking {
            Some(crate::provider::ThinkingRequest::Disabled) => {
                if matches!(profile, DashscopeReasoningProfile::Toggleable) {
                    request_override
                        .body_patch
                        .insert("enable_thinking".to_owned(), serde_json::Value::Bool(false));
                }
                request_override.body_patch.remove("thinking_budget");
            }
            Some(crate::provider::ThinkingRequest::Budget { budget_tokens }) => {
                request_override
                    .body_patch
                    .entry("enable_thinking".to_owned())
                    .or_insert_with(|| serde_json::Value::Bool(true));
                request_override
                    .body_patch
                    .entry("thinking_budget".to_owned())
                    .or_insert_with(|| serde_json::Value::from(*budget_tokens));
            }
            _ => {
                if !request_override.body_patch.contains_key("enable_thinking")
                    && !matches!(profile, DashscopeReasoningProfile::AlwaysOn)
                {
                    request_override
                        .body_patch
                        .insert("enable_thinking".to_owned(), serde_json::Value::Bool(true));
                }
            }
        }
    }

    fn dashscope_thinking_modes(model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        let mut modes = BTreeMap::new();
        match Self::dashscope_reasoning_profile(model.as_str()) {
            Some(DashscopeReasoningProfile::Toggleable) => {
                let mut disabled_override = crate::model::ModelSpeedModeRequestOverride::default();
                disabled_override
                    .body_patch
                    .insert("enable_thinking".to_owned(), serde_json::Value::Bool(false));
                modes.insert(
                    "no-thinking".to_owned(),
                    ModelThinkingMode::new()
                        .with_display_name("No Thinking")
                        .with_thinking(crate::provider::ThinkingRequest::Disabled)
                        .with_request_override(disabled_override),
                );

                let mut enabled_override = crate::model::ModelSpeedModeRequestOverride::default();
                enabled_override
                    .body_patch
                    .insert("enable_thinking".to_owned(), serde_json::Value::Bool(true));
                modes.insert(
                    "thinking-enabled".to_owned(),
                    ModelThinkingMode::new()
                        .with_display_name("Thinking")
                        .with_description("Enable DashScope reasoning output")
                        .with_request_override(enabled_override),
                );
            }
            Some(DashscopeReasoningProfile::AlwaysOn) => {
                let mut enabled_override = crate::model::ModelSpeedModeRequestOverride::default();
                enabled_override
                    .body_patch
                    .insert("enable_thinking".to_owned(), serde_json::Value::Bool(true));
                modes.insert(
                    "thinking-enabled".to_owned(),
                    ModelThinkingMode::new()
                        .with_display_name("Thinking")
                        .with_description("Use the model's built-in reasoning output")
                        .with_request_override(enabled_override),
                );
            }
            None => {}
        }
        modes
    }

    fn backend_key(&self) -> &'static str {
        match self.backend {
            OpenAiBackend::Api => "api",
            OpenAiBackend::ChatgptCodex => "chatgpt_codex",
        }
    }

    fn can_fallback_to_chat(&self) -> bool {
        matches!(self.backend, OpenAiBackend::Api)
    }

    fn chatgpt_account_id(&self) -> Option<String> {
        self.auth_data
            .as_ref()
            .and_then(|auth| auth.try_lock().ok())
            .as_deref()
            .and_then(AuthData::account_id)
            .map(ToOwned::to_owned)
            .and_then(|value| utils::normalize_optional_text(Some(value)))
    }

    fn realtime_ws_endpoint(&self, model: &str) -> Result<url::Url, AppError> {
        let mut endpoint = if let Some(ws_url) = self.realtime_ws_url.as_ref() {
            url::Url::parse(ws_url).map_err(|err| {
                AppError::Config(format!("openai realtime websocket url is invalid: {err}"))
            })?
        } else {
            let mut url = url::Url::parse(self.base_url.as_str())
                .map_err(|err| AppError::Config(format!("openai base url is invalid: {err}")))?;
            let realtime_path = format!("{}/realtime", url.path().trim_end_matches('/'));
            url.set_path(realtime_path.as_str());
            url
        };

        match endpoint.scheme() {
            "http" => endpoint.set_scheme("ws").map_err(|_| {
                AppError::Config("openai realtime websocket url is invalid".to_owned())
            })?,
            "https" => endpoint.set_scheme("wss").map_err(|_| {
                AppError::Config("openai realtime websocket url is invalid".to_owned())
            })?,
            "ws" | "wss" => {}
            other => {
                return Err(AppError::Config(format!(
                    "openai realtime websocket url has unsupported scheme `{other}`"
                )));
            }
        }

        let existing = endpoint
            .query_pairs()
            .into_owned()
            .filter(|(key, _)| key != "model")
            .collect::<Vec<_>>();

        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in existing {
            serializer.append_pair(key.as_str(), value.as_str());
        }
        serializer.append_pair("model", model);
        let query = serializer.finish();
        endpoint.set_query(Some(query.as_str()));

        Ok(endpoint)
    }

    fn realtime_handshake_request(
        &self,
        endpoint: &url::Url,
        api_key: &str,
        session_affinity: Option<&str>,
    ) -> Result<http::Request<()>, AppError> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let mut request = endpoint.as_str().into_client_request().map_err(|err| {
            AppError::Config(format!(
                "openai realtime websocket handshake invalid: {err}"
            ))
        })?;

        let auth_header_name = http::header::HeaderName::from_bytes(self.auth_header.as_bytes())
            .map_err(|err| {
                AppError::Config(format!("openai auth header name is invalid: {err}"))
            })?;
        let auth_header_value = http::header::HeaderValue::from_str(
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key).as_str(),
        )
        .map_err(|err| AppError::Config(format!("openai auth header value is invalid: {err}")))?;
        request
            .headers_mut()
            .insert(auth_header_name, auth_header_value);

        if let Some(session_affinity) = session_affinity.filter(|value| !value.trim().is_empty()) {
            request.headers_mut().insert(
                http::header::HeaderName::from_static("x-session-affinity"),
                http::header::HeaderValue::from_str(session_affinity).map_err(|err| {
                    AppError::Config(format!(
                        "openai session affinity header value is invalid: {err}"
                    ))
                })?,
            );
        }

        if endpoint
            .host_str()
            .map(|host| {
                host.eq_ignore_ascii_case("api.openai.com") || host.ends_with(".openai.com")
            })
            .unwrap_or(false)
        {
            request.headers_mut().insert(
                http::header::HeaderName::from_static("openai-beta"),
                http::header::HeaderValue::from_static("realtime=v1"),
            );
        }

        for (key, value) in &self.extra_headers {
            let header_name =
                http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|err| {
                    AppError::Config(format!(
                        "openai extra header name `{key}` is invalid: {err}"
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_str(value.as_str()).map_err(|err| {
                    AppError::Config(format!(
                        "openai extra header `{key}` value is invalid: {err}"
                    ))
                })?;
            request.headers_mut().insert(header_name, header_value);
        }

        Ok(request)
    }

    fn should_use_responses(&self, model: &str) -> bool {
        if matches!(self.profile, OpenAiProfile::GithubCopilot)
            && !self.api_mode_explicit
            && matches!(self.api_mode, OpenAiApiMode::Responses)
        {
            return Self::copilot_should_use_responses(model);
        }

        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            return true;
        }

        match self.api_mode {
            OpenAiApiMode::Responses => true,
            OpenAiApiMode::Chat => false,
            OpenAiApiMode::Auto => {
                model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4")
            }
        }
    }

    fn copilot_should_use_responses(model: &str) -> bool {
        let is_gpt5 = model
            .strip_prefix("gpt-")
            .and_then(|x| x.split('-').next())
            .and_then(|major| major.parse::<u32>().ok())
            .map(|major| major >= 5)
            .unwrap_or(false);
        is_gpt5 && !model.starts_with("gpt-5-mini")
    }

    fn api_mode_key(&self) -> &'static str {
        match self.api_mode {
            OpenAiApiMode::Responses => "responses",
            OpenAiApiMode::Chat => "chat",
            OpenAiApiMode::Auto => "auto",
        }
    }

    fn stream_mode_key(&self) -> &'static str {
        match self.stream_mode {
            OpenAiStreamMode::Sse => "sse",
            OpenAiStreamMode::RealtimeWebSocket => "realtime_websocket",
        }
    }

    fn responses_endpoint_unsupported(status: reqwest::StatusCode) -> bool {
        matches!(
            status,
            reqwest::StatusCode::NOT_FOUND
                | reqwest::StatusCode::METHOD_NOT_ALLOWED
                | reqwest::StatusCode::NOT_IMPLEMENTED
        )
    }

    async fn complete_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<CompletionResponse, AppError> {
        if !request.native_tools.bindings().is_empty() {
            return Err(AppError::Config(format!(
                "provider `{}` model `{}` configures native hosted tools, but the OpenAI chat API path does not support them; use Responses mode instead",
                self.id, model
            )));
        }
        let model_id = ModelId::new(model.clone());
        let prompt_cache_key = self
            .uses_chat_compatible_request_fields()
            .then(|| request.prompt_cache_key.clone())
            .flatten();
        let mut request_override = request.request_override.clone();
        let assistant_reasoning_field = self.assistant_reasoning_field_for_model(&model_id);
        self.apply_dashscope_reasoning_overrides(
            &model_id,
            request.thinking.as_ref(),
            &mut request_override,
        );

        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request, assistant_reasoning_field),
            tools: (!request.tools.is_empty())
                .then(|| tools_to_chat_definitions(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
            stream: false,
            stream_options: None,
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            verbosity: request.verbosity.clone(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self.chat_endpoint().expect("chat endpoint should resolve");
            let mut headers = self.auth_headers(
                RequestHeaderContext::from_chat_request(request, prompt_cache_key.as_deref()),
                api_key,
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete.chat",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(&body_json),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                .json(&body_json)
        })
        .await?;

        let payload: ChatCompletionResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete.chat",
            response,
        )
        .await?;
        let response_reasoning_field = payload
            .choices
            .first()
            .and_then(|choice| choice.message.as_ref())
            .and_then(chat_wire::assistant_reasoning_field_from_delta_or_message)
            .or_else(|| {
                payload
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.as_ref())
                    .and_then(chat_wire::assistant_reasoning_field_from_delta_or_message)
            })
            .or(assistant_reasoning_field);
        let mut parsed =
            chat_wire::parse_completion_response(self.id.as_str(), model.as_str(), payload)?;
        parsed.provider_metadata = utils::provider_metadata_with_assistant_reasoning_field(
            parsed.provider_metadata.take(),
            response_reasoning_field,
        );
        Ok(parsed)
    }

    async fn complete_stream_with_chat_api(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        if !request.native_tools.bindings().is_empty() {
            return Err(AppError::Config(format!(
                "provider `{}` model `{}` configures native hosted tools, but the OpenAI chat API path does not support them; use Responses mode instead",
                self.id, model
            )));
        }
        let model_id = ModelId::new(model.clone());
        let prompt_cache_key = self
            .uses_chat_compatible_request_fields()
            .then(|| request.prompt_cache_key.clone())
            .flatten();
        let mut request_override = request.request_override.clone();
        let assistant_reasoning_field = self.assistant_reasoning_field_for_model(&model_id);
        self.apply_dashscope_reasoning_overrides(
            &model_id,
            request.thinking.as_ref(),
            &mut request_override,
        );

        let body = ChatCompletionRequest {
            model: model.clone(),
            messages: self.chat_messages_for_request(request, assistant_reasoning_field),
            tools: (!request.tools.is_empty())
                .then(|| tools_to_chat_definitions(request.tools.as_slice())),
            temperature: request.temperature,
            max_tokens: request.max_output_tokens,
            cache_control: self
                .supports_top_level_prompt_cache()
                .then(prompt_cache::PromptCacheControl::ephemeral),
            prompt_cache_key: prompt_cache_key.clone(),
            prompt_cache_key_camel_case: prompt_cache_key.clone(),
            stream: true,
            stream_options: Some(ChatStreamOptions {
                include_usage: true,
            }),
            stop: request.stop_sequences.clone(),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            verbosity: request.verbosity.clone(),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self.chat_endpoint().expect("chat endpoint should resolve");
            let mut headers = self.auth_headers(
                RequestHeaderContext::from_chat_request(request, prompt_cache_key.as_deref()),
                api_key,
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.chat",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(&body_json),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                .json(&body_json)
        })
        .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.chat",
                response,
            )
            .await);
        }

        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.chat",
            response.status(),
            response.headers(),
        );
        let provider_name = self.id.clone();
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, chat_wire::ChatToolCallStreamState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut response_id: Option<String> = None;
            let mut assistant_reasoning_field_seen: Option<&'static str> = None;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.chat",
                    &event,
                );

                let chunk: utils::ChatStreamChunk =
                    utils::parse_json_value(provider_name.as_str(), "chat stream chunk", event)?;
                if let Some(next_response_id) = chunk.id.clone() {
                    response_id = Some(next_response_id);
                }
                let choice = chunk.choices.first();

                let delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.content.as_ref())
                    .map(chat_wire::extract_text_from_content)
                    .or_else(|| choice.and_then(|item| item.text.clone()))
                    .unwrap_or_default();

                if !delta.is_empty() {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                let reasoning_delta = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| {
                        if assistant_reasoning_field_seen.is_none() {
                            assistant_reasoning_field_seen =
                                chat_wire::assistant_reasoning_field_from_fields(
                                    delta.reasoning_content.as_ref(),
                                    delta.reasoning_details.as_ref(),
                                );
                        }
                        chat_wire::extract_reasoning_text_from_fields(
                            delta.reasoning_content.as_ref(),
                            delta.reasoning_details.as_ref(),
                        )
                    })
                    .unwrap_or_default();

                if !reasoning_delta.is_empty() {
                    stream_has_content = true;
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta: reasoning_delta,
                    };
                }

                let tool_deltas = choice
                    .and_then(|item| item.delta.as_ref())
                    .and_then(|delta| delta.tool_calls.clone())
                    .unwrap_or_default();

                for raw_tool in tool_deltas {
                    let tool = utils::parse_json_value::<chat_wire::ChatToolCallWire>(
                        provider_name.as_str(),
                        "chat stream tool_call delta",
                        raw_tool,
                    )?;
                    let id = utils::normalize_optional_text(tool.id.clone());
                    let key = tool
                        .index
                        .map(|idx| format!("idx:{idx}"))
                        .or_else(|| id.as_ref().map(|value| format!("id:{value}")))
                        .ok_or_else(|| {
                            AppError::Provider(
                                "openai chat stream tool_call delta missing index/id".to_owned(),
                            )
                        })?;

                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = id {
                        state.id = Some(id);
                    }
                    let mut emitted_any = false;
                    if let Some(function) = tool.function {
                        if let Some(name) = utils::normalize_optional_text(function.name) {
                            state.name = Some(name);
                        }
                        if let Some(args) = function.arguments
                            && !args.is_empty() {
                                state.arguments.push_str(args.as_str());
                                stream_has_content = true;
                                emitted_any = true;
                                state.announced = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta: args,
                                };
                            }
                    }
                    // Register the call with the aggregator the first
                    // time we have its name available, even if no
                    // arguments arrived this chunk — a parameterless
                    // call may never carry args.
                    if !state.announced && !emitted_any && state.name.is_some() {
                        state.announced = true;
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
                    }
                }

                if let Some(raw_usage) = chunk.usage {
                    let usage = utils::parse_json_value::<chat_wire::ChatUsage>(
                        provider_name.as_str(),
                        "chat stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Some(chat_wire::chat_usage_to_completion(usage));
                }

                let finish_reason = choice
                    .and_then(|item| item.finish_reason.as_deref())
                    .filter(|value| !value.is_empty() && *value != "null")
                    .map(ToOwned::to_owned);

                if stream_finish_reason.is_none() {
                    stream_finish_reason = finish_reason;
                }
            }

            if stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some() {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: utils::provider_metadata_with_assistant_reasoning_field(
                        response_id_metadata(response_id),
                        assistant_reasoning_field_seen.or(assistant_reasoning_field),
                    ),
                };
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete_stream_with_realtime_ws(
        &self,
        request: &CompletionRequest,
        model: String,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let ws_endpoint = self.realtime_ws_endpoint(model.as_str())?;
        let api_key = self.api_key.resolve().await?;
        let handshake = self.realtime_handshake_request(
            &ws_endpoint,
            api_key.as_str(),
            self.uses_chat_compatible_request_fields()
                .then_some(request.prompt_cache_key.as_deref())
                .flatten(),
        )?;
        let handshake_headers = handshake
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (key.as_str().to_owned(), text.to_owned()))
            })
            .collect::<Vec<_>>();
        utils::adapter_log_http_request_json(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.realtime_ws.handshake",
            "GET",
            ws_endpoint.as_str(),
            handshake_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
            None,
        );
        let (ws_stream, handshake_response) = tokio_tungstenite::connect_async(handshake)
            .await
            .map_err(|err| {
                AppError::Provider(format!("openai realtime websocket connect failed: {err}"))
            })?;
        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.realtime_ws.handshake",
            handshake_response.status(),
            handshake_response.headers(),
        );

        let provider_name = self.id.clone();
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = ModelId::new(model);
        let conversation_items =
            Self::realtime_conversation_items_for_messages(request.messages.as_slice());
        let tool_plan = Self::responses_tool_plan(request)?;
        let response_tools =
            (!tool_plan.tools.is_empty()).then(|| serde_json::Value::Array(tool_plan.tools));
        let system = request.system.clone();
        let temperature = request.temperature;
        let max_output_tokens = request.max_output_tokens;

        let stream = async_stream::try_stream! {
            let (mut ws_writer, mut ws_reader) = ws_stream.split();

            if let Some(instructions) = system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let event = serde_json::json!({
                    "type": "session.update",
                    "session": {
                        "instructions": instructions,
                    }
                });
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.realtime_ws.outbound",
                    &event,
                );
                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "openai realtime websocket send session.update failed: {err}"
                        ))
                    })?;
            }

            for item in &conversation_items {
                let event = serde_json::json!({
                    "type": "conversation.item.create",
                    "item": item,
                });
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.realtime_ws.outbound",
                    &event,
                );

                ws_writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(event.to_string().into()))
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "openai realtime websocket send conversation.item.create failed: {err}"
                        ))
                    })?;
            }

            let mut response = serde_json::json!({
                "output_modalities": ["text"],
            });
            if let Some(temperature) = temperature {
                response["temperature"] = serde_json::json!(temperature);
            }
            if let Some(max_tokens) = max_output_tokens {
                response["max_output_tokens"] = serde_json::json!(max_tokens);
            }
            if let Some(tools) = response_tools.as_ref() {
                response["tools"] = tools.clone();
            }

            let create_event = serde_json::json!({
                "type": "response.create",
                "response": response,
            });
            utils::adapter_log_stream_event(
                provider_name.as_str(),
                ADAPTER_KIND,
                "complete_stream.realtime_ws.outbound",
                &create_event,
            );

            ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Text(create_event.to_string().into()))
                .await
                .map_err(|err| {
                    AppError::Provider(format!(
                        "openai realtime websocket send response.create failed: {err}"
                    ))
                })?;

            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(message) = ws_reader.next().await {
                let message = message.map_err(|err| {
                    AppError::Provider(format!("openai realtime websocket receive failed: {err}"))
                })?;

                let payload = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        String::from_utf8(bytes.to_vec()).map_err(|err| {
                            AppError::Provider(format!(
                                "openai realtime websocket binary frame is not utf-8: {err}"
                            ))
                        })?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                    tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
                };

                let event: serde_json::Value = serde_json::from_str(payload.as_str()).map_err(|err| {
                    AppError::Provider(format!("openai realtime websocket event decode failed: {err}"))
                })?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.realtime_ws.inbound",
                    &event,
                );

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
                    Err(err)?;
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    let key = tool_event.stream_key(provider_name.as_str())?;

                    let is_added = matches!(tool_event.kind, utils::ResponsesToolEventKind::Added);
                    let was_new = !pending_tool_calls.contains_key(&key);
                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    if is_added && was_new {
                        // Register the call with the aggregator so a
                        // parameterless tool call (no Delta events) is
                        // not silently dropped.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
                    }

                    match tool_event.kind {
                        utils::ResponsesToolEventKind::Delta => {
                            if let Some(arguments_delta) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                state.arguments.push_str(arguments_delta.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta,
                                };
                            }
                        }
                        utils::ResponsesToolEventKind::Added => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }
                        }
                        utils::ResponsesToolEventKind::Done => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }

                            pending_tool_calls.remove(key.as_str());
                        }
                    }
                }

                if let Some(raw_usage) = utils::responses_usage_value(&event) {
                    let usage = utils::parse_json_value::<OpenAiUsage>(
                        provider_name.as_str(),
                        "realtime stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Self::map_usage(Some(usage));
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if let Some(next_response_id) = utils::responses_response_id(&event) {
                    response_id = Some(next_response_id);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: response_id_metadata(response_id.clone()),
                    };
                    completed_emitted = true;
                    break;
                }
            }

            let _ = ws_writer
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await;

            if !completed_emitted
                && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some())
            {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: None,
                };
            }
        };

        Ok(Box::pin(stream))
    }

    fn extract_text(response: &OpenAiResponsesResponse) -> String {
        if let Some(text) = response.output_text.as_ref() {
            return text.clone();
        }

        response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() != Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("")
    }

    fn extract_reasoning_text(response: &OpenAiResponsesResponse) -> Option<String> {
        let summaries: String = response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("reasoning"))
            .flat_map(|item| item.summary.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        if !summaries.is_empty() {
            return Some(summaries);
        }

        let text: String = response
            .output
            .iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("reasoning"))
            .flat_map(|item| item.content.iter().flatten())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        (!text.is_empty()).then_some(text)
    }

    async fn complete_by_aggregating_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        let fallback_model = request.model.clone();
        let stream = ModelRuntime::complete_stream(self, request).await?;
        utils::aggregate_stream(self.id.as_str(), fallback_model, stream).await
    }

    fn map_usage(usage: Option<OpenAiUsage>) -> Option<CompletionUsage> {
        usage.map(|u| {
            let input_tokens_raw = u.input_tokens.unwrap_or_default();
            let cache_read_tokens = u
                .input_tokens_details
                .and_then(|d| d.cached_tokens)
                .unwrap_or_default();
            let reasoning_tokens = u
                .output_tokens_details
                .and_then(|d| d.reasoning_tokens)
                .unwrap_or_default();
            // Match Anthropic's convention: `input_tokens` is the uncached
            // portion only. OpenAI's `input_tokens` is inclusive of cache.
            let input_tokens = input_tokens_raw.saturating_sub(cache_read_tokens);
            let output_tokens = u
                .output_tokens
                .unwrap_or_default()
                .saturating_sub(reasoning_tokens);
            MessageUsage {
                input_tokens,
                output_tokens,
                reasoning_tokens,
                cache_write_tokens: 0,
                cache_read_tokens,
                total_cost: 0.0,
            }
            .into()
        })
    }

    fn merge_tool_provider_options(
        map: &mut serde_json::Map<String, serde_json::Value>,
        extra: Option<&serde_json::Value>,
        tool_label: &str,
    ) -> Result<(), AppError> {
        let Some(extra) = extra else {
            return Ok(());
        };
        let extra = extra.as_object().ok_or_else(|| {
            AppError::Config(format!(
                "openai native tool `{tool_label}` provider_options must be a JSON object"
            ))
        })?;
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
        Ok(())
    }

    fn responses_tool_plan(
        request: &CompletionRequest,
    ) -> Result<OpenAiResponsesToolPlan, AppError> {
        let mut plan = OpenAiResponsesToolPlan::default();
        for tool in &request.tools {
            let mut map = serde_json::Map::new();
            map.insert(
                "type".to_owned(),
                serde_json::Value::String("function".to_owned()),
            );
            map.insert(
                "name".to_owned(),
                serde_json::Value::String(tool.exposed_name.clone()),
            );
            map.insert(
                "description".to_owned(),
                serde_json::Value::String(tool.description_text().to_string()),
            );
            map.insert("parameters".to_owned(), tool.sanitized_input_schema());
            if tool.decl.strict {
                map.insert("strict".to_owned(), serde_json::Value::Bool(true));
            }
            plan.tools.push(serde_json::Value::Object(map));
        }

        for binding in request.native_tools.bindings() {
            if binding.route != ProviderNativeToolRoute::ProviderHosted {
                return Err(AppError::Config(format!(
                    "openai native tool `{}` only supports `provider_hosted` routes in the current runtime",
                    binding.tool.config_key()
                )));
            }
            match binding.tool {
                ProviderNativeToolKind::WebSearch => {
                    let config = &request.native_tools.hosted.web_search;
                    if config.max_results.is_some() {
                        return Err(AppError::Config(
                            "openai native tool `web_search` does not support `hosted.web_search.max_results`; use `provider_options` for provider-specific overrides instead".to_owned(),
                        ));
                    }
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("web_search".to_owned()),
                    );
                    if let Some(freshness) = config.freshness {
                        match freshness {
                            NativeToolFreshness::Auto => {}
                            NativeToolFreshness::Cached => {
                                map.insert(
                                    "external_web_access".to_owned(),
                                    serde_json::Value::Bool(false),
                                );
                            }
                            NativeToolFreshness::Live => {
                                map.insert(
                                    "external_web_access".to_owned(),
                                    serde_json::Value::Bool(true),
                                );
                            }
                        }
                    }
                    if let Some(search_context_size) = config.search_context_size.as_ref() {
                        map.insert(
                            "search_context_size".to_owned(),
                            serde_json::Value::String(search_context_size.clone()),
                        );
                    }
                    if !config.user_location.is_empty() {
                        let mut location = serde_json::Map::new();
                        location.insert(
                            "type".to_owned(),
                            serde_json::Value::String("approximate".to_owned()),
                        );
                        if let Some(country) = config.user_location.country.as_ref() {
                            location.insert(
                                "country".to_owned(),
                                serde_json::Value::String(country.clone()),
                            );
                        }
                        if let Some(region) = config.user_location.region.as_ref() {
                            location.insert(
                                "region".to_owned(),
                                serde_json::Value::String(region.clone()),
                            );
                        }
                        if let Some(city) = config.user_location.city.as_ref() {
                            location
                                .insert("city".to_owned(), serde_json::Value::String(city.clone()));
                        }
                        if let Some(timezone) = config.user_location.timezone.as_ref() {
                            location.insert(
                                "timezone".to_owned(),
                                serde_json::Value::String(timezone.clone()),
                            );
                        }
                        map.insert(
                            "user_location".to_owned(),
                            serde_json::Value::Object(location),
                        );
                    }
                    if !config.allowed_domains.is_empty() || !config.blocked_domains.is_empty() {
                        let mut filters = serde_json::Map::new();
                        if !config.allowed_domains.is_empty() {
                            filters.insert(
                                "allowed_domains".to_owned(),
                                serde_json::Value::Array(
                                    config
                                        .allowed_domains
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        if !config.blocked_domains.is_empty() {
                            filters.insert(
                                "blocked_domains".to_owned(),
                                serde_json::Value::Array(
                                    config
                                        .blocked_domains
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        map.insert("filters".to_owned(), serde_json::Value::Object(filters));
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "web_search",
                    )?;
                    plan.tools.push(serde_json::Value::Object(map));
                    plan.include
                        .push("web_search_call.action.sources".to_owned());
                }
                ProviderNativeToolKind::FileSearch => {
                    let config = &request.native_tools.hosted.file_search;
                    if config.vector_store_ids.is_empty() {
                        return Err(AppError::Config(
                            "openai native tool `file_search` requires at least one `vector_store_ids` entry".to_owned(),
                        ));
                    }
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("file_search".to_owned()),
                    );
                    map.insert(
                        "vector_store_ids".to_owned(),
                        serde_json::Value::Array(
                            config
                                .vector_store_ids
                                .iter()
                                .cloned()
                                .map(serde_json::Value::String)
                                .collect(),
                        ),
                    );
                    if let Some(max_results) = config.max_results {
                        map.insert(
                            "max_num_results".to_owned(),
                            serde_json::Value::Number(max_results.into()),
                        );
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "file_search",
                    )?;
                    plan.tools.push(serde_json::Value::Object(map));
                    if config.include_results.unwrap_or(false) {
                        plan.include.push("file_search_call.results".to_owned());
                    }
                }
                ProviderNativeToolKind::CodeExecution => {
                    let config = &request.native_tools.hosted.code_execution;
                    let container = &config.container;
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("code_interpreter".to_owned()),
                    );
                    if let Some(container_id) = container.id.as_ref() {
                        if container.kind.is_some()
                            || container.memory_limit.is_some()
                            || !container.file_ids.is_empty()
                        {
                            return Err(AppError::Config(
                                "openai native tool `code_execution` cannot combine `container.id` with `container.type`, `memory_limit`, or `file_ids`".to_owned(),
                            ));
                        }
                        map.insert(
                            "container".to_owned(),
                            serde_json::Value::String(container_id.clone()),
                        );
                    } else if !container.is_empty() {
                        let kind = container.kind.as_deref().unwrap_or("auto");
                        if kind != "auto" {
                            return Err(AppError::Config(format!(
                                "openai native tool `code_execution` only supports container type `auto`, found `{kind}`"
                            )));
                        }
                        let mut container_map = serde_json::Map::new();
                        container_map.insert(
                            "type".to_owned(),
                            serde_json::Value::String("auto".to_owned()),
                        );
                        if let Some(memory_limit) = container.memory_limit.as_ref() {
                            container_map.insert(
                                "memory_limit".to_owned(),
                                serde_json::Value::String(memory_limit.clone()),
                            );
                        }
                        if !container.file_ids.is_empty() {
                            container_map.insert(
                                "file_ids".to_owned(),
                                serde_json::Value::Array(
                                    container
                                        .file_ids
                                        .iter()
                                        .cloned()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                ),
                            );
                        }
                        map.insert(
                            "container".to_owned(),
                            serde_json::Value::Object(container_map),
                        );
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "code_execution",
                    )?;
                    plan.tools.push(serde_json::Value::Object(map));
                }
                ProviderNativeToolKind::ImageGeneration => {
                    return Err(AppError::Config(
                        "openai native tool `image_generation` is not enabled in Agena runtime yet because generated image outputs are not projected into assistant message parts".to_owned(),
                    ));
                }
                other => {
                    return Err(AppError::Config(format!(
                        "openai native tool `{}` is not supported by the current runtime",
                        other.config_key()
                    )));
                }
            }
        }

        Ok(plan)
    }

    fn native_tools_request_requires_responses(request: &CompletionRequest) -> bool {
        !request.native_tools.bindings().is_empty()
    }

    fn is_vision_request(request: &CompletionRequest) -> bool {
        request.messages.iter().any(|message| {
            wire_message::project(message).iter().any(|part| {
                matches!(
                    part,
                    wire_message::WirePart::Attachment { item }
                        if item.kind == AttachmentKind::Image
                )
            })
        })
    }

    fn initiator(request: &CompletionRequest) -> &'static str {
        match request.messages.last().map(|m| m.role) {
            Some(Role::User) => "user",
            _ => "agent",
        }
    }

    fn chat_messages_for_request(
        &self,
        request: &CompletionRequest,
        assistant_reasoning_field: Option<&str>,
    ) -> Vec<chat_wire::ChatMessage> {
        let mut messages = chat_wire::request_to_chat_messages_with_assistant_reasoning_field(
            request,
            assistant_reasoning_field,
        );
        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            apply_chat_prompt_cache_hints(messages.as_mut_slice());
        }
        messages
    }

    fn responses_input_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Self::to_responses_input(request);
        if !matches!(self.profile, OpenAiProfile::GithubCopilot) {
            clear_responses_prompt_cache_hints(input.as_mut_slice());
        }
        input
    }

    fn to_responses_input(request: &CompletionRequest) -> Vec<OpenAiResponsesInputItem> {
        let mut input = Vec::new();

        if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for message in &request.messages {
            Self::append_responses_items_for_message(&mut input, message);
        }

        apply_responses_prompt_cache_hints(input.as_mut_slice());
        input
    }

    fn realtime_conversation_items_for_messages(
        messages: &[Message],
    ) -> Vec<OpenAiRealtimeConversationItem> {
        let mut input = Vec::new();
        for message in messages {
            Self::append_responses_items_for_message(&mut input, message);
        }
        clear_responses_prompt_cache_hints(input.as_mut_slice());
        input
            .into_iter()
            .map(OpenAiRealtimeConversationItem::from_responses_input)
            .collect()
    }

    fn attachment_upload_name(item: &AttachmentItem) -> String {
        wire_message::filename(item)
            .map(str::to_owned)
            .unwrap_or_else(|| item.summary_label())
    }

    fn responses_file_content(item: &AttachmentItem) -> Option<OpenAiInputContent> {
        let filename = Some(Self::attachment_upload_name(item));
        match &item.source {
            AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
                wire_message::data_url(item).map(|file_data| OpenAiInputContent::File {
                    file_data: Some(file_data),
                    file_id: None,
                    file_url: None,
                    filename,
                })
            }
            AttachmentSource::FileId { file_id } => {
                let file_id = file_id.trim();
                (!file_id.is_empty()).then(|| OpenAiInputContent::File {
                    file_data: None,
                    file_id: Some(file_id.to_owned()),
                    file_url: None,
                    filename,
                })
            }
            AttachmentSource::Url { url } => {
                let file_url = url.trim();
                (!file_url.is_empty()).then(|| OpenAiInputContent::File {
                    file_data: None,
                    file_id: None,
                    file_url: Some(file_url.to_owned()),
                    filename,
                })
            }
            AttachmentSource::LocalPath { .. } => None,
        }
    }

    fn responses_content_from_attachment(item: &AttachmentItem) -> OpenAiInputContent {
        match item.kind {
            AttachmentKind::Image => wire_message::media_url(item)
                .map(|image_url| OpenAiInputContent::Image { image_url })
                .unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
                }),
            AttachmentKind::Audio
            | AttachmentKind::Video
            | AttachmentKind::Pdf
            | AttachmentKind::File => {
                Self::responses_file_content(item).unwrap_or_else(|| OpenAiInputContent::Text {
                    text: wire_message::hint_text(item),
                })
            }
        }
    }

    fn push_responses_text_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        text: String,
    ) {
        if text.trim().is_empty() {
            return;
        }

        input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
            role: role.to_owned(),
            content: vec![OpenAiInputContent::Text { text }],
            copilot_cache_control: None,
        }));
    }

    fn flush_assistant_responses_text(
        input: &mut Vec<OpenAiResponsesInputItem>,
        text_chunks: &mut Vec<String>,
    ) {
        if text_chunks.is_empty() {
            return;
        }

        let text = text_chunks.join("");
        text_chunks.clear();
        Self::push_responses_text_message(input, "assistant", text);
    }

    fn push_responses_message_from_parts(
        input: &mut Vec<OpenAiResponsesInputItem>,
        role: &str,
        parts: &[wire_message::WirePart],
    ) {
        let content = Self::responses_input_contents_from_parts(parts);
        if content.is_empty() {
            return;
        }

        input.push(OpenAiResponsesInputItem::Message(OpenAiInputMessage {
            role: role.to_owned(),
            content,
            copilot_cache_control: None,
        }));
    }

    fn append_responses_items_for_message(
        input: &mut Vec<OpenAiResponsesInputItem>,
        message: &Message,
    ) {
        let projected_parts = wire_message::project(message);
        match message.role {
            Role::System => Self::push_responses_text_message(
                input,
                "system",
                session_text_lossy(message, projected_parts.as_slice()),
            ),
            Role::User => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "user", message.as_text_lossy());
                } else {
                    Self::push_responses_message_from_parts(
                        input,
                        "user",
                        projected_parts.as_slice(),
                    );
                }
            }
            Role::Assistant => {
                if projected_parts.is_empty() {
                    Self::push_responses_text_message(input, "assistant", message.as_text_lossy());
                } else {
                    let mut text_chunks = Vec::new();
                    let mut pending_output: Option<(String, String, Vec<wire_message::WirePart>)> =
                        None;
                    for part in projected_parts {
                        match part {
                            wire_message::WirePart::Text { text } => {
                                Self::flush_responses_function_output(input, &mut pending_output);
                                text_chunks.push(text);
                            }
                            wire_message::WirePart::Attachment { item } => {
                                if let Some((_, _, extra_parts)) = pending_output.as_mut() {
                                    extra_parts.push(wire_message::WirePart::Attachment { item });
                                } else {
                                    text_chunks.push(wire_message::hint_text(&item));
                                }
                            }
                            wire_message::WirePart::ToolCall {
                                id,
                                name,
                                arguments_json,
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
                                Self::flush_responses_function_output(input, &mut pending_output);
                                if !id.trim().is_empty() && !name.trim().is_empty() {
                                    input.push(OpenAiResponsesInputItem::FunctionCall(
                                        OpenAiFunctionCallItem {
                                            kind: "function_call",
                                            call_id: id,
                                            name,
                                            arguments: arguments_json,
                                            copilot_cache_control: None,
                                        },
                                    ));
                                }
                            }
                            wire_message::WirePart::ToolResult {
                                tool_call_id,
                                output_json,
                                ..
                            } => {
                                Self::flush_assistant_responses_text(input, &mut text_chunks);
                                Self::flush_responses_function_output(input, &mut pending_output);
                                if !tool_call_id.trim().is_empty() {
                                    pending_output = Some((tool_call_id, output_json, Vec::new()));
                                }
                            }
                        }
                    }
                    Self::flush_responses_function_output(input, &mut pending_output);
                    Self::flush_assistant_responses_text(input, &mut text_chunks);
                }
            }
        }
    }

    fn responses_input_contents_from_parts(
        parts: &[wire_message::WirePart],
    ) -> Vec<OpenAiInputContent> {
        parts
            .iter()
            .map(|part| match part {
                wire_message::WirePart::Text { text } => {
                    OpenAiInputContent::Text { text: text.clone() }
                }
                wire_message::WirePart::Attachment { item } => {
                    Self::responses_content_from_attachment(item)
                }
                wire_message::WirePart::ToolCall { name, .. } => OpenAiInputContent::Text {
                    text: format!("[tool_call:{name}]"),
                },
                wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                    OpenAiInputContent::Text {
                        text: format!("[tool_result:{tool_call_id}]"),
                    }
                }
            })
            .collect()
    }

    fn multimodal_function_output_value(
        output_json: &str,
        extra_parts: &[wire_message::WirePart],
    ) -> serde_json::Value {
        if extra_parts.is_empty() {
            return serde_json::Value::String(output_json.to_owned());
        }

        let mut content = Vec::new();
        if !output_json.trim().is_empty() {
            content.push(OpenAiInputContent::Text {
                text: output_json.to_owned(),
            });
        }
        content.extend(Self::responses_input_contents_from_parts(extra_parts));
        serde_json::to_value(content).expect("openai function_call_output content should serialize")
    }

    fn flush_responses_function_output(
        input: &mut Vec<OpenAiResponsesInputItem>,
        pending_output: &mut Option<(String, String, Vec<wire_message::WirePart>)>,
    ) {
        let Some((call_id, output_json, extra_parts)) = pending_output.take() else {
            return;
        };
        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
            OpenAiFunctionCallOutputItem {
                kind: "function_call_output",
                call_id,
                output: Self::multimodal_function_output_value(
                    output_json.as_str(),
                    extra_parts.as_slice(),
                ),
                copilot_cache_control: None,
            },
        ));
    }

    fn parse_responses_tool_calls(
        items: Option<&Vec<OpenAiOutputItem>>,
    ) -> Result<Vec<CompletionToolCall>, AppError> {
        items
            .into_iter()
            .flatten()
            .filter(|item| item.kind.as_deref() == Some("function_call"))
            .map(|item| {
                let id = utils::normalize_optional_text(item.call_id.clone())
                    .or_else(|| utils::normalize_optional_text(item.id.clone()))
                    .ok_or_else(|| {
                        AppError::Provider(
                            "openai responses payload returned function_call without id/call_id"
                                .to_owned(),
                        )
                    })?;

                let name = utils::normalize_optional_text(item.name.clone()).ok_or_else(|| {
                    AppError::Provider(
                        "openai responses payload returned function_call without name".to_owned(),
                    )
                })?;

                Ok(CompletionToolCall::Function {
                    id,
                    name,
                    arguments_json: item.arguments.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn compact_summary_from_output(output: &[serde_json::Value]) -> Option<String> {
        let mut chunks = Vec::new();
        for item in output {
            let item_type = item
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            match item_type {
                "message" => {
                    if let Some(role) = item.get("role").and_then(serde_json::Value::as_str)
                        && role == "developer"
                    {
                        continue;
                    }
                    collect_compact_content_text(item.get("content"), &mut chunks);
                }
                "compaction" | "compaction_summary" | "context_compaction" => {
                    collect_compact_string_field(item, "summary", &mut chunks);
                    collect_compact_string_field(item, "text", &mut chunks);
                    collect_compact_string_field(item, "message", &mut chunks);
                }
                _ => {
                    collect_compact_string_field(item, "summary", &mut chunks);
                    collect_compact_string_field(item, "text", &mut chunks);
                }
            }
        }
        let summary = chunks
            .into_iter()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        (!summary.trim().is_empty()).then_some(summary)
    }

    async fn send_json<R>(
        &self,
        operation: &str,
        endpoint: String,
        body: Option<&serde_json::Value>,
        context: RequestHeaderContext<'_>,
    ) -> Result<R, AppError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let mut headers = self.auth_headers(context, api_key);
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                operation,
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                body,
            );
            let mut request =
                utils::apply_resolved_request_headers(self.client.post(endpoint.clone()), &headers);

            if let Some(body) = body {
                request = request.json(body);
            }

            request
        })
        .await?;
        utils::parse_json_response_logged(self.id.as_str(), ADAPTER_KIND, operation, response).await
    }

    fn resolved_headers(&self, context: RequestHeaderContext<'_>) -> BTreeMap<String, String> {
        let mut headers = self.extra_headers.clone();

        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            headers
                .entry("originator".to_owned())
                .or_insert_with(|| CHATGPT_CODEX_ORIGINATOR.to_owned());
            headers
                .entry(reqwest::header::USER_AGENT.as_str().to_owned())
                .or_insert_with(|| CHATGPT_CODEX_USER_AGENT.to_owned());

            if let Some(account_id) = self.chatgpt_account_id() {
                headers.insert("ChatGPT-Account-Id".to_owned(), account_id);
            }

            if let Some(window_id) = context.window_id_header() {
                headers.insert("x-codex-window-id".to_owned(), window_id);
            }
        }

        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            headers
                .entry(reqwest::header::USER_AGENT.as_str().to_owned())
                .or_insert_with(|| crate::provider::CODEX_USER_AGENT.to_owned());
            headers
                .entry("Openai-Intent".to_owned())
                .or_insert_with(|| "conversation-edits".to_owned());
            headers.insert(
                "x-initiator".to_owned(),
                context.initiator_header().to_owned(),
            );
            if context.vision_request {
                headers.insert("Copilot-Vision-Request".to_owned(), "true".to_owned());
            }
        }

        if let Some(session_affinity) = context.session_affinity_header() {
            headers.insert("x-session-affinity".to_owned(), session_affinity.to_owned());
        }

        if let Some(request_headers) = context.request_headers {
            headers = utils::merged_request_headers(&headers, request_headers);
        }

        utils::resolved_request_headers(self.id.as_str(), &headers)
    }

    fn auth_headers(
        &self,
        context: RequestHeaderContext<'_>,
        api_key: &str,
    ) -> BTreeMap<String, String> {
        let mut headers = self.resolved_headers(context);
        headers.insert(
            self.auth_header.clone(),
            utils::auth_header_value(self.auth_scheme.as_deref(), api_key),
        );
        headers
    }
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
}

#[derive(Clone, Copy, Default)]
struct RequestHeaderContext<'a> {
    prompt_cache_key: Option<&'a str>,
    session_affinity: Option<&'a str>,
    prompt_window_generation: Option<u64>,
    initiator: Option<&'a str>,
    vision_request: bool,
    request_headers: Option<&'a std::collections::BTreeMap<String, String>>,
}

impl<'a> RequestHeaderContext<'a> {
    fn from_request(request: &'a CompletionRequest) -> Self {
        Self {
            prompt_cache_key: request.prompt_cache_key.as_deref(),
            session_affinity: None,
            prompt_window_generation: request.prompt_window_generation,
            initiator: Some(OpenAiAdapter::initiator(request)),
            vision_request: OpenAiAdapter::is_vision_request(request),
            request_headers: Some(&request.request_override.headers),
        }
    }

    fn from_chat_request(
        request: &'a CompletionRequest,
        session_affinity: Option<&'a str>,
    ) -> Self {
        Self {
            session_affinity,
            ..Self::from_request(request)
        }
    }

    fn none() -> Self {
        Self::default()
    }

    fn window_id_header(&self) -> Option<String> {
        self.prompt_cache_key.map(|prompt_cache_key| {
            format!(
                "{}:{}",
                prompt_cache_key,
                self.prompt_window_generation.unwrap_or_default()
            )
        })
    }

    fn session_affinity_header(&self) -> Option<&str> {
        self.session_affinity
            .filter(|value| !value.trim().is_empty())
    }

    fn initiator_header(&self) -> &str {
        self.initiator.unwrap_or("agent")
    }
}

#[async_trait]
impl ModelRuntime for OpenAiAdapter {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        Some(self.capability_family)
    }

    fn validate_native_tools_request(
        &self,
        _adapter_id: Option<&crate::model::AdapterId>,
        request: &CompletionRequest,
    ) -> Result<(), AppError> {
        Self::responses_tool_plan(request).map(|_| ())
    }

    fn model_capabilities_for_adapter(
        &self,
        adapter_id: Option<&crate::model::AdapterId>,
        model: &ModelId,
    ) -> ModelCapabilities {
        let mut capabilities = crate::provider::default_capability_registry()
            .capabilities_for_family(self.capability_family, model.as_str());
        let _ = adapter_id;
        if self.is_dashscope_reasoning_model(model) {
            capabilities.reasoning = CapabilitySupport::Supported;
        }
        capabilities
    }

    fn model_thinking_modes_for_adapter(
        &self,
        adapter_id: Option<&crate::model::AdapterId>,
        model: &ModelId,
    ) -> BTreeMap<String, ModelThinkingMode> {
        let modes = crate::provider::default_model_mode_registry().thinking_modes_for_family(
            self.capability_family,
            adapter_id,
            model.as_str(),
            &self.model_metadata_for_adapter(adapter_id, model),
        );
        if modes.is_empty() && self.is_dashscope_reasoning_model(model) {
            return Self::dashscope_thinking_modes(model);
        }
        modes
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        matches!(self.stream_mode, OpenAiStreamMode::Sse)
            && self.should_use_responses(model.as_str())
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        Some(
            crate::provider::PromptCacheShape::new(self.id.as_str())
                .with_string("auth_scope", self.api_key.prompt_cache_scope())
                .with_string("backend", self.backend_key())
                .with_string("base_url", self.prompt_cache_base_url().as_str())
                .with_string("api_mode", self.api_mode_key())
                .with_string("stream_mode", self.stream_mode_key())
                .with_optional_string("models_url", self.models_url.as_deref())
                .with_string("auth_header", self.auth_header.as_str())
                .with_optional_string("auth_scheme", self.auth_scheme.as_deref())
                .with_string(
                    "profile",
                    match self.profile {
                        OpenAiProfile::Standard => "standard",
                        OpenAiProfile::GithubCopilot => "github_copilot",
                    },
                )
                .with_string(
                    "capability_family",
                    match self.capability_family {
                        CapabilityFamily::OpenAi => "openai",
                        CapabilityFamily::OpenAiCompatible => "openai_compatible",
                        CapabilityFamily::Anthropic => "anthropic",
                        CapabilityFamily::Gemini => "gemini",
                        CapabilityFamily::Bedrock => "bedrock",
                        CapabilityFamily::Gitlab => "gitlab",
                    },
                )
                .with_optional_string("auth_account_id", self.chatgpt_account_id())
                .with_bool("uses_responses", self.should_use_responses(model.as_str()))
                .with_optional_string("realtime_ws_url", self.realtime_ws_url.as_deref())
                .with_bool(
                    "supports_top_level_prompt_cache",
                    self.supports_top_level_prompt_cache(),
                )
                .with_json(
                    "extra_headers",
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
        )
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = if self.atomgit_coding_plan_models {
            self.atomgit_coding_plan_model_response().await?
        } else {
            let endpoint = self.model_endpoint()?;
            utils::send_with_credential_refresh(&self.api_key, |api_key| {
                let headers = self.auth_headers(RequestHeaderContext::none(), api_key);
                utils::adapter_log_http_request_json(
                    self.id.as_str(),
                    ADAPTER_KIND,
                    "list_models",
                    "GET",
                    endpoint.as_str(),
                    headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    None,
                );
                utils::apply_resolved_request_headers(self.client.get(endpoint.as_str()), &headers)
            })
            .await?
        };

        let payload: OpenAiModelListResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(payload
            .into_items()
            .into_iter()
            .map(|m| {
                let metadata = m.metadata();
                let model = ProviderModel::new(self.id.as_str(), m.id);
                let capabilities = self.model_capabilities(&model.id);
                let mut model = model.with_capabilities(capabilities);
                if !metadata.is_empty() {
                    model = model.with_metadata(metadata);
                }
                if let Some(name) = m.display_name.or(m.name) {
                    model.with_display_name(name)
                } else {
                    model
                }
            })
            .collect())
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let native_tools_require_responses =
            Self::native_tools_request_requires_responses(&request);

        if !self.should_use_responses(model.as_str()) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but the selected OpenAI API mode resolves to chat; switch this provider/model to Responses mode",
                    self.id, model
                )));
            }
            return self
                .complete_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request);
        let tool_plan = Self::responses_tool_plan(&request)?;

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            stream: false,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            text: request
                .verbosity
                .as_ref()
                .map(|verbosity| OpenAiResponsesTextConfig {
                    verbosity: verbosity.clone(),
                }),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response: OpenAiResponsesResponse = match self
            .send_json(
                "complete.responses",
                self.responses_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::from_request(&request),
            )
            .await
        {
            Ok(payload) => payload,
            Err(AppError::HttpStatus { status, .. })
                if !native_tools_require_responses
                    && self.can_fallback_to_chat()
                    && Self::responses_endpoint_unsupported(status) =>
            {
                return self
                    .complete_with_chat_api(&request, model.to_string())
                    .await;
            }
            Err(err) => return Err(err),
        };

        let response_model =
            ModelId::new(response.model.clone().unwrap_or_else(|| model.to_string()));
        let text = Self::extract_text(&response);
        let reasoning_text = Self::extract_reasoning_text(&response);
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let tool_calls = Self::parse_responses_tool_calls(response.output.as_ref())?;

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return self.complete_by_aggregating_stream(request).await;
        }

        let usage = Self::map_usage(response.usage);

        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id.as_str()),
            model: response_model,
            text,
            reasoning_text,
            finish_reason,
            tool_calls,
            usage,
            provider_metadata: response_id_metadata(response.id),
        })
    }

    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        let model = request.model.clone();
        if self.backend != OpenAiBackend::Api
            || self.profile != OpenAiProfile::Standard
            || self.is_openai_compatible_family()
            || !self.should_use_responses(model.as_str())
        {
            return Ok(None);
        }

        let mut input_request = request.clone();
        input_request.system = None;
        input_request.previous_response_id = None;
        let input = self.responses_input_for_request(&input_request);
        let tool_plan = Self::responses_tool_plan(&request)?;
        let body = OpenAiResponsesCompactRequest {
            model: model.to_string(),
            instructions: request.system.clone(),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            parallel_tool_calls: request
                .request_override
                .parallel_tool_calls()
                .unwrap_or(false),
            prompt_cache_key: request.prompt_cache_key.clone(),
            reasoning: chat_wire::reasoning_effort(request.thinking.as_ref(), model.as_str()).map(
                |effort| OpenAiCompactReasoningConfig {
                    effort: Some(effort),
                },
            ),
            text: request
                .verbosity
                .as_ref()
                .map(|verbosity| OpenAiResponsesTextConfig {
                    verbosity: verbosity.clone(),
                }),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;
        let response: OpenAiResponsesCompactResponse = self
            .send_json(
                "compact.responses",
                self.responses_compact_endpoint()?,
                Some(&body_json),
                RequestHeaderContext::from_request(&request),
            )
            .await?;
        Ok(Self::compact_summary_from_output(
            response.output.as_slice(),
        ))
    }

    #[tracing::instrument(
        skip_all,
        fields(provider = tracing::field::Empty, model = %request.model)
    )]
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        tracing::Span::current().record("provider", tracing::field::display(self.id.as_str()));
        let model = request.model.clone();
        let native_tools_require_responses =
            Self::native_tools_request_requires_responses(&request);

        if matches!(self.stream_mode, OpenAiStreamMode::RealtimeWebSocket) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but OpenAI realtime websocket mode does not support them; use SSE Responses streaming instead",
                    self.id, model
                )));
            }
            return self
                .complete_stream_with_realtime_ws(&request, model.to_string())
                .await;
        }

        if !self.should_use_responses(model.as_str()) {
            if native_tools_require_responses {
                return Err(AppError::Config(format!(
                    "provider `{}` model `{}` configures native hosted tools, but the selected OpenAI API mode resolves to chat; switch this provider/model to Responses mode",
                    self.id, model
                )));
            }
            return self
                .complete_stream_with_chat_api(&request, model.to_string())
                .await;
        }

        let input = self.responses_input_for_request(&request);
        let tool_plan = Self::responses_tool_plan(&request)?;

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            stream: true,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            response_format: chat_wire::map_response_format(request.response_format.as_ref()),
            reasoning_effort: chat_wire::reasoning_effort(
                request.thinking.as_ref(),
                model.as_str(),
            ),
            text: request
                .verbosity
                .as_ref()
                .map(|verbosity| OpenAiResponsesTextConfig {
                    verbosity: verbosity.clone(),
                }),
        };
        let body_json =
            utils::serialize_request_body_with_patch(&body, &request.request_override.body_patch)?;

        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
            let endpoint = self
                .responses_endpoint()
                .expect("responses endpoint should resolve");
            let mut headers =
                self.auth_headers(RequestHeaderContext::from_request(&request), api_key);
            headers.insert(
                reqwest::header::CONTENT_TYPE.as_str().to_owned(),
                "application/json".to_owned(),
            );
            utils::adapter_log_http_request_json(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.responses",
                "POST",
                endpoint.as_str(),
                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                Some(&body_json),
            );
            utils::apply_resolved_request_headers(self.client.post(endpoint), &headers)
                .json(&body_json)
        })
        .await?;

        if !response.status().is_success() {
            if self.can_fallback_to_chat()
                && !native_tools_require_responses
                && Self::responses_endpoint_unsupported(response.status())
            {
                return self
                    .complete_stream_with_chat_api(&request, model.to_string())
                    .await;
            }
            return Err(utils::http_status_error_from_response_logged(
                self.id.as_str(),
                ADAPTER_KIND,
                "complete_stream.responses",
                response,
            )
            .await);
        }

        utils::ensure_response_content_type(self.id.as_str(), &response, "text/event-stream")?;
        utils::adapter_log_http_response_open(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete_stream.responses",
            response.status(),
            response.headers(),
        );
        let provider_name = self.id.clone();
        let mut events = sse::json_events(response);
        let provider_id = ProviderId::new(provider_name.as_str());
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut pending_tool_calls: std::collections::BTreeMap<String, ResponsesToolState> = std::collections::BTreeMap::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut completed_emitted = false;
            let mut response_id: Option<String> = None;

            while let Some(event) = events.next().await {
                let event = event?;
                utils::adapter_log_stream_event(
                    provider_name.as_str(),
                    ADAPTER_KIND,
                    "complete_stream.responses",
                    &event,
                );

                if let Some(err) = utils::responses_stream_error(provider_name.as_str(), &event)? {
                    Err(err)?;
                }

                if let Some(delta) = utils::responses_text_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::TextDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(delta) = responses_reasoning_delta(&event) {
                    stream_has_content = true;
                    yield CompletionStreamEvent::ThinkingDelta {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        delta,
                    };
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    let key = tool_event.stream_key(provider_name.as_str())?;

                    let is_added = matches!(tool_event.kind, utils::ResponsesToolEventKind::Added);
                    let was_new = !pending_tool_calls.contains_key(&key);
                    let state = pending_tool_calls.entry(key.clone()).or_default();
                    if let Some(id) = tool_event.id.clone() {
                        state.id = Some(id);
                    }
                    if let Some(name) = tool_event.name.clone() {
                        state.name = Some(name);
                    }

                    if is_added && was_new {
                        // Register the call with the aggregator so a
                        // parameterless tool call (no Delta events) is
                        // not silently dropped.
                        stream_has_content = true;
                        yield CompletionStreamEvent::ToolCallDelta {
                            provider_id: provider_id.clone(),
                            model: model_name.clone(),
                            stream_key: key.clone(),
                            id: state.id.clone(),
                            name: state.name.clone(),
                            arguments_delta: String::new(),
                        };
                    }

                    match tool_event.kind {
                        utils::ResponsesToolEventKind::Delta => {
                            if let Some(arguments_delta) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                state.arguments.push_str(arguments_delta.as_str());
                                stream_has_content = true;
                                yield CompletionStreamEvent::ToolCallDelta {
                                    provider_id: provider_id.clone(),
                                    model: model_name.clone(),
                                    stream_key: key.clone(),
                                    id: state.id.clone(),
                                    name: state.name.clone(),
                                    arguments_delta,
                                };
                            }
                        }
                        utils::ResponsesToolEventKind::Added => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }
                        }
                        utils::ResponsesToolEventKind::Done => {
                            if let Some(arguments_snapshot) =
                                tool_event.arguments.filter(|s| !s.is_empty())
                            {
                                let arguments_delta = if arguments_snapshot.starts_with(&state.arguments)
                                {
                                    arguments_snapshot[state.arguments.len()..].to_owned()
                                } else {
                                    arguments_snapshot.clone()
                                };

                                if arguments_snapshot.starts_with(&state.arguments) {
                                    state.arguments.push_str(arguments_delta.as_str());
                                } else {
                                    state.arguments = arguments_snapshot;
                                }

                                if !arguments_delta.is_empty() {
                                    stream_has_content = true;
                                    yield CompletionStreamEvent::ToolCallDelta {
                                        provider_id: provider_id.clone(),
                                        model: model_name.clone(),
                                        stream_key: key.clone(),
                                        id: state.id.clone(),
                                        name: state.name.clone(),
                                        arguments_delta,
                                    };
                                }
                            }

                            pending_tool_calls.remove(key.as_str());
                        }
                    }
                }

                if let Some(raw_usage) = utils::responses_usage_value(&event) {
                    let usage = utils::parse_json_value::<OpenAiUsage>(
                        provider_name.as_str(),
                        "responses stream usage",
                        raw_usage,
                    )?;
                    stream_usage = Self::map_usage(Some(usage));
                }

                if stream_finish_reason.is_none() {
                    stream_finish_reason = utils::responses_finish_reason(&event);
                }

                if let Some(next_response_id) = utils::responses_response_id(&event) {
                    response_id = Some(next_response_id);
                }

                if utils::responses_is_completed(&event) {
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason: CompletionFinishReason::from_provider(
                            stream_finish_reason.as_deref(),
                        ),
                        usage: stream_usage.clone(),
                        provider_metadata: response_id_metadata(response_id.clone()),
                    };
                    completed_emitted = true;
                    break;
                }
            }

            if !completed_emitted
                && (stream_has_content || stream_finish_reason.is_some() || stream_usage.is_some())
            {
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason: CompletionFinishReason::from_provider(
                        stream_finish_reason.as_deref(),
                    ),
                    usage: stream_usage,
                    provider_metadata: response_id_metadata(response_id),
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<chat_wire::ChatResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<OpenAiResponsesTextConfig>,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesCompactRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<OpenAiCompactReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<OpenAiResponsesTextConfig>,
}

#[derive(Debug, Serialize)]
struct OpenAiCompactReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesCompactResponse {
    #[serde(default)]
    output: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesTextConfig {
    verbosity: String,
}

#[derive(Debug, Serialize)]
struct OpenAiInputMessage {
    role: String,
    content: Vec<OpenAiInputContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OpenAiInputContent {
    #[serde(rename = "input_text")]
    Text { text: String },
    #[serde(rename = "input_image")]
    Image { image_url: String },
    #[serde(rename = "input_file")]
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

impl OpenAiResponsesInputItem {
    fn is_system(&self) -> bool {
        matches!(
            self,
            Self::Message(OpenAiInputMessage { role, .. }) if role == "system"
        )
    }

    fn set_copilot_cache_control(&mut self, cache_control: prompt_cache::PromptCacheControl) {
        match self {
            Self::Message(message) => message.copilot_cache_control = Some(cache_control),
            Self::FunctionCall(item) => item.copilot_cache_control = Some(cache_control),
            Self::FunctionCallOutput(item) => item.copilot_cache_control = Some(cache_control),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiRealtimeConversationItem {
    Message(OpenAiRealtimeMessageItem),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

impl OpenAiRealtimeConversationItem {
    fn from_responses_input(value: OpenAiResponsesInputItem) -> Self {
        match value {
            OpenAiResponsesInputItem::Message(message) => {
                Self::Message(OpenAiRealtimeMessageItem {
                    kind: "message",
                    role: message.role,
                    content: message.content,
                })
            }
            OpenAiResponsesInputItem::FunctionCall(item) => Self::FunctionCall(item),
            OpenAiResponsesInputItem::FunctionCallOutput(item) => Self::FunctionCallOutput(item),
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiRealtimeMessageItem {
    #[serde(rename = "type")]
    kind: &'static str,
    role: String,
    content: Vec<OpenAiInputContent>,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    name: String,
    arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCallOutputItem {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: String,
    output: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Default)]
struct ResponsesToolState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashscopeReasoningProfile {
    Toggleable,
    AlwaysOn,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiModelListResponse {
    Wrapped { data: Vec<OpenAiModel> },
    AtomGitCodingPlan(Vec<AtomGitCodingPlanModel>),
    Bare(Vec<OpenAiModel>),
}

impl OpenAiModelListResponse {
    fn into_items(self) -> Vec<OpenAiModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::AtomGitCodingPlan(data) => data
                .into_iter()
                .filter_map(AtomGitCodingPlanModel::into_openai_model)
                .collect(),
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomGitCodingPlanType {
    Max,
    Pro,
    Lite,
}

impl AtomGitCodingPlanType {
    const CASCADE_ORDER: &'static [Self] = &[Self::Max, Self::Pro, Self::Lite];

    fn as_str(self) -> &'static str {
        match self {
            Self::Max => "Max",
            Self::Pro => "Pro",
            Self::Lite => "Lite",
        }
    }
}

#[derive(Debug, Deserialize)]
struct AtomGitCodingPlanClaimResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    duplicate: bool,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_window_tokens: Option<u64>,
    #[serde(default)]
    max_input_tokens: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
}

impl OpenAiModel {
    fn metadata(&self) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();

        if let Some(context_window_tokens) = self.context_window_tokens.or(self.max_input_tokens) {
            metadata = metadata.with_context_window_tokens(clamp_u64_to_u32(context_window_tokens));
        }
        if let Some(max_input_tokens) = self.max_input_tokens {
            metadata = metadata.with_max_input_tokens(clamp_u64_to_u32(max_input_tokens));
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            metadata = metadata.with_max_output_tokens(clamp_u64_to_u32(max_output_tokens));
        }

        metadata
    }
}

#[derive(Debug, Deserialize)]
struct AtomGitCodingPlanModel {
    display_model_name: String,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    plan_available: bool,
}

impl AtomGitCodingPlanModel {
    fn into_openai_model(self) -> Option<OpenAiModel> {
        let model_name = self.display_model_name.trim();
        if !self.plan_available || model_name.is_empty() {
            return None;
        }

        Some(OpenAiModel {
            id: model_name.to_owned(),
            display_name: Some(model_name.to_owned()),
            name: Some(model_name.to_owned()),
            context_window_tokens: self.context_window,
            max_input_tokens: None,
            max_output_tokens: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Option<Vec<OpenAiOutputItem>>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputItem {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    content: Option<Vec<OpenAiOutputContent>>,
    #[serde(default)]
    summary: Option<Vec<OpenAiReasoningSummaryContent>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputContent {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiReasoningSummaryContent {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    input_tokens_details: Option<OpenAiInputTokenDetails>,
}

fn collect_compact_content_text(value: Option<&serde_json::Value>, chunks: &mut Vec<String>) {
    match value {
        Some(serde_json::Value::String(text)) => chunks.push(text.clone()),
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                collect_compact_string_field(item, "text", chunks);
                collect_compact_string_field(item, "summary", chunks);
            }
        }
        Some(serde_json::Value::Object(_)) => {
            if let Some(value) = value {
                collect_compact_string_field(value, "text", chunks);
                collect_compact_string_field(value, "summary", chunks);
            }
        }
        _ => {}
    }
}

fn collect_compact_string_field(value: &serde_json::Value, field: &str, chunks: &mut Vec<String>) {
    if let Some(text) = value.get(field).and_then(serde_json::Value::as_str)
        && !text.trim().is_empty()
    {
        chunks.push(text.to_string());
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiInputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

fn responses_reasoning_delta(event: &serde_json::Value) -> Option<String> {
    let event_type = event.get("type")?.as_str()?;
    if event_type == "response.reasoning_summary_text.delta"
        || event_type == "response.reasoning.delta"
    {
        return event
            .get("delta")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
    }
    None
}

fn apply_chat_prompt_cache_hints(messages: &mut [chat_wire::ChatMessage]) {
    let flags = messages
        .iter()
        .map(|message| message.role == "system")
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(message) = messages.get_mut(index) {
            message.copilot_cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

fn apply_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    let flags = input
        .iter()
        .map(OpenAiResponsesInputItem::is_system)
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(item) = input.get_mut(index) {
            item.set_copilot_cache_control(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

fn clear_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    for item in input {
        match item {
            OpenAiResponsesInputItem::Message(message) => message.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCall(item) => item.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCallOutput(item) => item.copilot_cache_control = None,
        }
    }
}

fn session_text_lossy(message: &Message, projected_parts: &[wire_message::WirePart]) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
    }
}

fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_model_parses_compatible_token_limits() {
        let payload = r#"{
            "data": [
                {
                    "id": "reasoner-x",
                    "display_name": "Reasoner X",
                    "context_length": 262144,
                    "input_token_limit": 260000,
                    "max_completion_tokens": 64000
                }
            ]
        }"#;

        let parsed: OpenAiModelListResponse =
            serde_json::from_str(payload).expect("parse openai model list");
        let models = parsed.into_items();
        assert_eq!(models.len(), 1);

        let metadata = models[0].metadata();
        assert_eq!(metadata.limits.context_window_tokens, Some(262_144));
        assert_eq!(metadata.limits.max_input_tokens, Some(260_000));
        assert_eq!(metadata.limits.max_output_tokens, Some(64_000));
    }

    #[test]
    fn atomgit_coding_plan_preserves_context_window() {
        let payload = r#"[
            {
                "display_model_name": "deepseek-v4-flash",
                "context_window": 128000,
                "plan_available": true
            }
        ]"#;

        let parsed: OpenAiModelListResponse =
            serde_json::from_str(payload).expect("parse atomgit coding plan models");
        let models = parsed.into_items();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "deepseek-v4-flash");
        assert_eq!(
            models[0].metadata().limits.context_window_tokens,
            Some(128_000)
        );
    }
}
