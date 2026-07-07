use async_trait::async_trait;
use futures_core::Stream;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use tokio::sync::Mutex;

use super::copilot_models::CopilotModelExtension;
use super::protocol_ids::{self, ProviderItemId, ProviderStreamKey};
use super::tool_stream::{
    ToolStreamAccumulator, ToolStreamInput, ToolStreamInputKind, ToolStreamUpdate,
};

use crate::{
    config::{NativeToolFreshness, ProviderNativeToolKind, ProviderNativeToolRoute},
    error::AppError,
    message::{
        ArtifactRef, AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage,
        OperationBlock, SearchResultItem, StructuredObject, ToolInvocation, ToolOutput,
    },
    model::{
        CapabilitySupport, ModelCapabilities, ModelId, ModelInputModality, ModelMetadata,
        ModelThinkingMode, ModelTokenLimits, ProviderId,
    },
    provider::{
        CapabilityFamily, CompletionFinishReason, CompletionRequest, CompletionResponse,
        CompletionStreamEvent, CompletionToolCall, CompletionUsage, ManagedCredential,
        ModelRuntime, ProviderModel, StreamResumePolicy,
        auth::AuthData,
        chat_wire::{self, ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions},
        prompt_cache, sse, utils, wire_message,
    },
    role::Role,
};

const CHATGPT_CODEX_ORIGINATOR: &str = crate::provider::CODEX_ORIGINATOR;
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
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
}

#[derive(Clone)]
pub struct OpenAiAdapterOptions {
    pub backend: OpenAiBackend,
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub api_mode: OpenAiApiMode,
    pub api_mode_explicit: bool,
    pub profile: OpenAiProfile,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub capability_family: CapabilityFamily,
    pub extra_headers: HashMap<String, String>,
    pub stream_mode: OpenAiStreamMode,
    pub realtime_ws_url: Option<String>,
    pub top_level_prompt_cache_override: Option<bool>,
}

impl Default for OpenAiAdapterOptions {
    fn default() -> Self {
        Self {
            backend: OpenAiBackend::Api,
            auth_data: None,
            api_mode: OpenAiApiMode::Responses,
            api_mode_explicit: false,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            stream_mode: OpenAiStreamMode::Sse,
            realtime_ws_url: None,
            top_level_prompt_cache_override: None,
        }
    }
}

#[derive(Debug)]
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
        Self::new_managed_with_options(
            id,
            client,
            api_key,
            base_url,
            default_model,
            OpenAiAdapterOptions::default(),
        )
    }

    pub fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: OpenAiAdapterOptions,
    ) -> Self {
        let id = id.into();
        let mut extra_headers = HashMap::from([
            (
                reqwest::header::USER_AGENT.as_str().to_owned(),
                crate::provider::codex_user_agent(),
            ),
            ("originator".to_owned(), CHATGPT_CODEX_ORIGINATOR.to_owned()),
        ]);
        if options
            .extra_headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        {
            extra_headers
                .retain(|key, _| !key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()));
        }
        if options
            .extra_headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("originator"))
        {
            extra_headers.retain(|key, _| !key.eq_ignore_ascii_case("originator"));
        }
        extra_headers.extend(options.extra_headers);
        Self {
            id,
            client,
            api_key,
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: ModelId::new(default_model),
            backend: options.backend,
            auth_data: options.auth_data,
            api_mode: options.api_mode,
            api_mode_explicit: options.api_mode_explicit,
            profile: options.profile,
            models_url: options
                .models_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            auth_header: options.auth_header,
            auth_scheme: options.auth_scheme,
            capability_family: options.capability_family,
            extra_headers,
            stream_mode: options.stream_mode,
            realtime_ws_url: options
                .realtime_ws_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            top_level_prompt_cache_override: options.top_level_prompt_cache_override,
        }
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

        let Some(domain) = auth_data
            .try_lock()
            .ok()
            .as_deref()
            .and_then(AuthData::enterprise_url)
            .map(ToOwned::to_owned)
        else {
            return Ok(self.base_url.clone());
        };

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

    fn list_models_endpoint(&self) -> Result<String, AppError> {
        let endpoint = self.model_endpoint()?;
        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            Ok(append_query_param(
                endpoint.as_str(),
                "client_version",
                openai_client_version().as_str(),
            ))
        } else {
            Ok(endpoint)
        }
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
                modes.insert(
                    "no-thinking".to_owned(),
                    ModelThinkingMode {
                        display_name: Some("Off".to_string()),
                        description: None,
                        thinking: Some(crate::provider::ThinkingRequest::Disabled),
                        request_override: crate::model::ModelSpeedModeRequestOverride {
                            headers: BTreeMap::new(),
                            body_patch: BTreeMap::from([(
                                "enable_thinking".to_owned(),
                                serde_json::Value::Bool(false),
                            )]),
                        },
                        adapter_overrides: BTreeMap::new(),
                    },
                );

                modes.insert(
                    "thinking-enabled".to_owned(),
                    ModelThinkingMode {
                        display_name: Some("Think".to_string()),
                        description: Some("Enable DashScope reasoning output".to_string()),
                        thinking: None,
                        request_override: crate::model::ModelSpeedModeRequestOverride {
                            headers: BTreeMap::new(),
                            body_patch: BTreeMap::from([(
                                "enable_thinking".to_owned(),
                                serde_json::Value::Bool(true),
                            )]),
                        },
                        adapter_overrides: BTreeMap::new(),
                    },
                );
            }
            Some(DashscopeReasoningProfile::AlwaysOn) => {
                modes.insert(
                    "thinking-enabled".to_owned(),
                    ModelThinkingMode {
                        display_name: Some("Think".to_string()),
                        description: Some("Use the model's built-in reasoning output".to_string()),
                        thinking: None,
                        request_override: crate::model::ModelSpeedModeRequestOverride {
                            headers: BTreeMap::new(),
                            body_patch: BTreeMap::from([(
                                "enable_thinking".to_owned(),
                                serde_json::Value::Bool(true),
                            )]),
                        },
                        adapter_overrides: BTreeMap::new(),
                    },
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

    fn unwrap_chat_completion_response(
        payload: OpenAiChatCompletionResponse,
    ) -> ChatCompletionResponse {
        match payload {
            OpenAiChatCompletionResponse::Bare(response) => response,
            OpenAiChatCompletionResponse::Wrapped { data, .. } => data,
        }
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

    fn chatgpt_account_is_fedramp(&self) -> bool {
        self.auth_data
            .as_ref()
            .and_then(|auth| auth.try_lock().ok())
            .as_deref()
            .is_some_and(AuthData::chatgpt_account_is_fedramp)
    }

    fn supports_codex_compat_headers(&self) -> bool {
        matches!(self.backend, OpenAiBackend::ChatgptCodex)
            || (matches!(self.profile, OpenAiProfile::Standard)
                && !self.is_openai_compatible_family())
    }

    fn should_require_sse_content_type(&self) -> bool {
        !matches!(self.backend, OpenAiBackend::ChatgptCodex)
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

        if !self.api_mode_explicit && !self.base_url_supports_responses_by_default() {
            return false;
        }

        match self.api_mode {
            OpenAiApiMode::Responses => true,
            OpenAiApiMode::Chat => false,
            OpenAiApiMode::Auto => {
                model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4")
            }
        }
    }

    fn base_url_supports_responses_by_default(&self) -> bool {
        url::Url::parse(self.base_url.as_str())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host == "api.openai.com" || host.ends_with(".openai.com"))
    }

    fn copilot_should_use_responses(model: &str) -> bool {
        let is_gpt5 = model
            .strip_prefix("gpt-")
            .and_then(|x| x.split(['-', '.']).next())
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
            tools: self.chat_tools_for_request(request),
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

        let payload: OpenAiChatCompletionResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "complete.chat",
            response,
        )
        .await?;
        let payload = Self::unwrap_chat_completion_response(payload);
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
            tools: self.chat_tools_for_request(request),
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
            Self::realtime_conversation_items_for_messages(request.messages.as_slice())?;
        let tool_plan = self.responses_tool_plan(request)?;
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

            let mut tool_stream = ToolStreamAccumulator::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut stream_tool_call_seen = false;
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

                if let Some(native_event) =
                    responses_native_tool_event(&provider_id, &model_name, &event)?
                {
                    stream_has_content = true;
                    yield native_event;
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    stream_tool_call_seen = true;
                    let input = responses_tool_stream_input(provider_name.as_str(), tool_event)?;
                    for update in tool_stream.ingest(provider_name.as_str(), input)? {
                        stream_has_content = true;
                        yield completion_event_from_tool_stream_update(
                            &provider_id,
                            &model_name,
                            update,
                        );
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
                    let finish_reason = responses_finish_reason_with_tool_calls(
                        CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                        stream_tool_call_seen,
                    );
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason,
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
                let finish_reason = responses_finish_reason_with_tool_calls(
                    CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                    stream_tool_call_seen,
                );
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason,
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
        &self,
        request: &CompletionRequest,
    ) -> Result<OpenAiResponsesToolPlan, AppError> {
        let mut tools = Vec::new();
        let mut include = Vec::new();
        let mut namespace_tools: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        for tool in crate::tool::model_tool_specs(request.tools.as_slice()) {
            let wire_name = responses_wire_tool_name(tool.model_name.as_str());
            let mut map = serde_json::Map::new();
            map.insert(
                "type".to_owned(),
                serde_json::Value::String("function".to_owned()),
            );
            map.insert("name".to_owned(), serde_json::Value::String(wire_name.name));
            map.insert(
                "description".to_owned(),
                serde_json::Value::String(tool.description),
            );
            map.insert("parameters".to_owned(), tool.input_schema);
            if tool.strict {
                map.insert("strict".to_owned(), serde_json::Value::Bool(true));
            }
            if let Some(namespace) = wire_name.namespace {
                namespace_tools
                    .entry(namespace)
                    .or_default()
                    .push(serde_json::Value::Object(map));
            } else {
                tools.push(serde_json::Value::Object(map));
            }
        }
        for (namespace, namespace_tools) in namespace_tools {
            tools.push(serde_json::json!({
                "type": "namespace",
                "name": namespace,
                "tools": namespace_tools,
            }));
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
                    tools.push(serde_json::Value::Object(map));
                    include.push("web_search_call.action.sources".to_owned());
                }
                ProviderNativeToolKind::FileSearch => {
                    let config = &request.native_tools.hosted.file_search;
                    if matches!(self.backend, OpenAiBackend::ChatgptCodex)
                        || config.vector_store_ids.is_empty()
                    {
                        continue;
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
                    tools.push(serde_json::Value::Object(map));
                    if config.include_results.unwrap_or(false) {
                        include.push("file_search_call.results".to_owned());
                    }
                }
                ProviderNativeToolKind::CodeExecution => {
                    if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
                        continue;
                    }
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
                    tools.push(serde_json::Value::Object(map));
                    include.push("code_interpreter_call.outputs".to_owned());
                }
                ProviderNativeToolKind::ImageGeneration => {
                    let config = &request.native_tools.hosted.image_generation;
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "type".to_owned(),
                        serde_json::Value::String("image_generation".to_owned()),
                    );
                    map.insert(
                        "output_format".to_owned(),
                        serde_json::Value::String("png".to_owned()),
                    );
                    if let Some(background) = config.background.as_ref() {
                        map.insert(
                            "background".to_owned(),
                            serde_json::Value::String(background.clone()),
                        );
                    }
                    if let Some(size) = config.size.as_ref() {
                        map.insert("size".to_owned(), serde_json::Value::String(size.clone()));
                    }
                    if let Some(quality) = config.quality.as_ref() {
                        map.insert(
                            "quality".to_owned(),
                            serde_json::Value::String(quality.clone()),
                        );
                    }
                    if let Some(moderation) = config.moderation.as_ref() {
                        map.insert(
                            "moderation".to_owned(),
                            serde_json::Value::String(moderation.clone()),
                        );
                    }
                    Self::merge_tool_provider_options(
                        &mut map,
                        config.provider_options.as_ref(),
                        "image_generation",
                    )?;
                    tools.push(serde_json::Value::Object(map));
                }
                other => {
                    return Err(AppError::Config(format!(
                        "openai native tool `{}` is not supported by the current runtime",
                        other.config_key()
                    )));
                }
            }
        }

        Ok(OpenAiResponsesToolPlan { tools, include })
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
        for message in &mut messages {
            if let Some(tool_calls) = message.tool_calls.as_mut() {
                for tool_call in tool_calls {
                    tool_call.function.name =
                        openai_chat_tool_name(tool_call.function.name.as_str());
                }
            }
        }
        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            apply_chat_prompt_cache_hints(messages.as_mut_slice());
        }
        messages
    }

    fn chat_tools_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Option<Vec<chat_wire::ChatToolDefinition>> {
        (!request.tools.is_empty()).then(|| {
            request
                .tools
                .iter()
                .map(crate::tool::ModelToolSpec::from_registered_tool)
                .map(|tool| chat_wire::ChatToolDefinition {
                    kind: "function".to_owned(),
                    function: chat_wire::ChatFunctionDefinition {
                        name: openai_chat_tool_name(tool.model_name.as_str()),
                        description: tool.description,
                        parameters: tool.input_schema,
                        strict: tool.strict,
                    },
                })
                .collect()
        })
    }

    #[allow(dead_code)]
    fn completion_response_stream(
        response: CompletionResponse,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>> {
        let provider_id = response.provider_id.clone();
        let model = response.model.clone();
        let mut events = Vec::new();
        if !response.text.is_empty() {
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: response.text,
            }));
        }
        if let Some(reasoning) = response.reasoning_text
            && !reasoning.is_empty()
        {
            events.push(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: reasoning,
            }));
        }
        for call in response.tool_calls {
            let CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } = call;
            events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: id.clone(),
                id: Some(id),
                name: Some(name),
                arguments_json,
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: response.finish_reason,
            usage: response.usage,
            provider_metadata: response.provider_metadata,
        }));
        Box::pin(futures_util::stream::iter(events))
    }

    fn responses_input_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<Vec<OpenAiResponsesInputItem>, AppError> {
        let mut input = Self::to_responses_input_with_system(request, false)?;
        clear_responses_prompt_cache_hints(input.as_mut_slice());
        Ok(input)
    }

    fn responses_instructions(request: &CompletionRequest) -> Option<String> {
        request
            .system
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn responses_parallel_tool_calls(request: &CompletionRequest) -> bool {
        request
            .request_override
            .parallel_tool_calls()
            .unwrap_or(false)
    }

    fn responses_request_max_output_tokens(&self, request: &CompletionRequest) -> Option<u32> {
        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            None
        } else {
            request.max_output_tokens
        }
    }

    fn responses_service_tier(request: &CompletionRequest) -> Option<String> {
        request
            .request_override
            .body_patch
            .get("service_tier")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn responses_client_metadata(
        context: RequestHeaderContext<'_>,
    ) -> Option<HashMap<String, String>> {
        context
            .responses_api_metadata
            .map(crate::provider::ResponsesApiRequestMetadata::client_metadata)
            .or_else(|| {
                let mut metadata = HashMap::new();
                if let Some(window_id) = context.window_id_header() {
                    metadata.insert("x-codex-window-id".to_owned(), window_id);
                }
                (!metadata.is_empty()).then_some(metadata)
            })
    }

    fn responses_include(
        mut include: Vec<String>,
        reasoning: Option<&OpenAiResponsesReasoningConfig>,
    ) -> Option<Vec<String>> {
        if reasoning.is_some()
            && !include
                .iter()
                .any(|value| value == "reasoning.encrypted_content")
        {
            include.push("reasoning.encrypted_content".to_owned());
        }
        (!include.is_empty()).then_some(include)
    }

    fn responses_reasoning_config(
        request: &CompletionRequest,
        model: &str,
    ) -> Option<OpenAiResponsesReasoningConfig> {
        chat_wire::reasoning_effort(request.thinking.as_ref(), model).map(|effort| {
            OpenAiResponsesReasoningConfig {
                effort: Some(effort),
            }
        })
    }

    fn responses_text_config(request: &CompletionRequest) -> Option<OpenAiResponsesTextConfig> {
        let verbosity = request.verbosity.clone();
        let format =
            OpenAiResponsesTextFormat::from_response_format(request.response_format.as_ref());
        (verbosity.is_some() || format.is_some())
            .then_some(OpenAiResponsesTextConfig { verbosity, format })
    }

    fn responses_tool_plan_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<OpenAiResponsesToolPlan, AppError> {
        self.responses_tool_plan(request)
    }

    fn to_responses_input_with_system(
        request: &CompletionRequest,
        include_system: bool,
    ) -> Result<Vec<OpenAiResponsesInputItem>, AppError> {
        let mut input = Vec::new();

        if include_system
            && let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty())
        {
            Self::push_responses_text_message(&mut input, "system", system.clone());
        }

        for message in &request.messages {
            Self::append_responses_items_for_message(&mut input, message);
        }

        validate_responses_input(input.as_slice())?;
        Ok(input)
    }

    fn realtime_conversation_items_for_messages(
        messages: &[Message],
    ) -> Result<Vec<OpenAiRealtimeConversationItem>, AppError> {
        let mut input = Vec::new();
        for message in messages {
            Self::append_responses_items_for_message(&mut input, message);
        }
        validate_responses_input(input.as_slice())?;
        clear_responses_prompt_cache_hints(input.as_mut_slice());
        Ok(input
            .into_iter()
            .map(OpenAiRealtimeConversationItem::from_responses_input)
            .collect())
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
                .unwrap_or_else(|| OpenAiInputContent::InputText {
                    text: wire_message::hint_text(item),
                }),
            AttachmentKind::Audio
            | AttachmentKind::Video
            | AttachmentKind::Pdf
            | AttachmentKind::File => Self::responses_file_content(item).unwrap_or_else(|| {
                OpenAiInputContent::InputText {
                    text: wire_message::hint_text(item),
                }
            }),
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
            content: vec![OpenAiInputContent::text_for_role(role, text)],
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
                                if let Some(call_id) = responses_input_call_id(id.as_str())
                                    && !name.trim().is_empty()
                                {
                                    let wire_name = responses_wire_tool_name(&name);
                                    input.push(OpenAiResponsesInputItem::FunctionCall(
                                        OpenAiFunctionCallItem {
                                            kind: "function_call",
                                            call_id,
                                            namespace: wire_name.namespace,
                                            name: wire_name.name,
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
                                if let Some(call_id) =
                                    responses_input_call_id(tool_call_id.as_str())
                                {
                                    pending_output = Some((call_id, output_json, Vec::new()));
                                }
                            }
                        }
                    }
                    Self::flush_responses_function_output(input, &mut pending_output);
                    Self::flush_assistant_responses_text(input, &mut text_chunks);
                }
            }
            Role::Tool => {
                for part in projected_parts {
                    if let wire_message::WirePart::ToolResult {
                        tool_call_id,
                        output_json,
                        ..
                    } = part
                        && let Some(call_id) = responses_input_call_id(tool_call_id.as_str())
                    {
                        input.push(OpenAiResponsesInputItem::FunctionCallOutput(
                            OpenAiFunctionCallOutputItem {
                                kind: "function_call_output",
                                call_id,
                                output: Self::multimodal_function_output_value(
                                    output_json.as_str(),
                                    &[],
                                ),
                                copilot_cache_control: None,
                            },
                        ));
                    }
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
                    OpenAiInputContent::InputText { text: text.clone() }
                }
                wire_message::WirePart::Attachment { item } => {
                    Self::responses_content_from_attachment(item)
                }
                wire_message::WirePart::ToolCall { name, .. } => OpenAiInputContent::InputText {
                    text: format!("[tool_call:{name}]"),
                },
                wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                    OpenAiInputContent::InputText {
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
            content.push(OpenAiInputContent::InputText {
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
                let id = responses_output_call_id(item.call_id.as_deref(), item.id.as_deref())
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
                let name = responses_model_tool_name(item.namespace.as_deref(), name.as_str());

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
        utils::ensure_header_case_insensitive(&mut headers, "originator", || {
            CHATGPT_CODEX_ORIGINATOR.to_owned()
        });
        utils::ensure_header_case_insensitive(
            &mut headers,
            reqwest::header::USER_AGENT.as_str(),
            crate::provider::codex_user_agent,
        );

        if self.supports_codex_compat_headers()
            && let Some(metadata) = context.responses_api_metadata
        {
            for (key, value) in metadata.session_headers() {
                utils::insert_header_case_insensitive(&mut headers, key, value);
            }
        }

        if matches!(self.backend, OpenAiBackend::ChatgptCodex) {
            if let Some(account_id) = self.chatgpt_account_id() {
                utils::insert_header_case_insensitive(
                    &mut headers,
                    "ChatGPT-Account-ID",
                    account_id,
                );
            }
            if self.chatgpt_account_is_fedramp() {
                utils::insert_header_case_insensitive(&mut headers, "X-OpenAI-Fedramp", "true");
            }
        }

        if self.supports_codex_compat_headers() {
            if let Some(metadata) = context.responses_api_metadata {
                for (key, value) in metadata.compatibility_headers() {
                    utils::insert_header_case_insensitive(&mut headers, key, value);
                }
            } else if let Some(window_id) = context.window_id_header() {
                utils::insert_header_case_insensitive(&mut headers, "x-codex-window-id", window_id);
            }
        }

        if matches!(self.profile, OpenAiProfile::GithubCopilot) {
            utils::ensure_header_case_insensitive(
                &mut headers,
                reqwest::header::USER_AGENT.as_str(),
                crate::provider::codex_user_agent,
            );
            utils::ensure_header_case_insensitive(&mut headers, "Openai-Intent", || {
                "conversation-edits".to_owned()
            });
            utils::insert_header_case_insensitive(
                &mut headers,
                "x-initiator",
                context.initiator_header(),
            );
            if context.vision_request {
                utils::insert_header_case_insensitive(
                    &mut headers,
                    "Copilot-Vision-Request",
                    "true",
                );
            }
        }

        if let Some(session_affinity) = context.session_affinity_header() {
            utils::insert_header_case_insensitive(
                &mut headers,
                "x-session-affinity",
                session_affinity,
            );
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

    fn provider_model_from_listed_model(&self, model: OpenAiListedModel) -> Option<ProviderModel> {
        match model {
            OpenAiListedModel::Compatible(model) => {
                if self.profile == OpenAiProfile::GithubCopilot
                    && (!model.copilot.visible() || model.copilot.uses_messages_endpoint())
                {
                    return None;
                }

                let metadata = model.metadata();
                let model_id = ModelId::new(model.id);
                let mut capabilities = self.model_capabilities(&model_id);
                if self.profile == OpenAiProfile::GithubCopilot {
                    capabilities = model
                        .copilot
                        .capabilities()
                        .merged_with_fallbacks_from(&capabilities);
                }

                let display_name = model
                    .display_name
                    .or(model.name)
                    .and_then(|value| utils::normalize_optional_text(Some(value)));
                Some(ProviderModel {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    capabilities,
                    metadata,
                    thinking_modes: BTreeMap::new(),
                    speed_modes: BTreeMap::new(),
                })
            }
            OpenAiListedModel::Recommended(model) => {
                let metadata = model.metadata();
                let display_name = model
                    .display_name
                    .or(model.name)
                    .and_then(|value| utils::normalize_optional_text(Some(value)));
                let model_id = utils::normalize_optional_text(Some(model.id))?;
                let model_id = ModelId::new(model_id);
                let capabilities = self.model_capabilities(&model_id);
                Some(ProviderModel {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    capabilities,
                    metadata,
                    thinking_modes: BTreeMap::new(),
                    speed_modes: BTreeMap::new(),
                })
            }
            OpenAiListedModel::Codex(model) => {
                let metadata = model.metadata();
                let capabilities = model.capabilities();
                let display_name =
                    utils::normalize_optional_text(model.display_name.or(model.name));
                let slug = utils::normalize_optional_text(Some(model.slug))?;
                let model_id = ModelId::new(slug);
                let capabilities =
                    capabilities.merged_with_fallbacks_from(&self.model_capabilities(&model_id));
                Some(ProviderModel {
                    provider_id: ProviderId::new(self.id.as_str()),
                    adapter_id: None,
                    id: model_id,
                    catalog_model_id: None,
                    display_name,
                    capabilities,
                    metadata,
                    thinking_modes: BTreeMap::new(),
                    speed_modes: BTreeMap::new(),
                })
            }
        }
    }
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
}

#[derive(Clone, Copy, Default)]
struct RequestHeaderContext<'a> {
    responses_api_metadata: Option<&'a crate::provider::ResponsesApiRequestMetadata>,
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
            responses_api_metadata: request.responses_api_metadata.as_ref(),
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
        self.responses_api_metadata
            .map(|metadata| metadata.window_id.clone())
            .or_else(|| {
                self.prompt_cache_key.map(|prompt_cache_key| {
                    format!(
                        "{}:{}",
                        prompt_cache_key,
                        self.prompt_window_generation.unwrap_or_default()
                    )
                })
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
        self.responses_tool_plan_for_request(request).map(|_| ())
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
        let _ = model;
        // OpenAI-compatible backends frequently diverge on `previous_response_id`
        // semantics. Replaying the normalized transcript is slower but reliable.
        false
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        let mut fields = vec![
            ("auth_scope", self.api_key.prompt_cache_scope()),
            ("backend", self.backend_key().to_owned()),
            ("base_url", self.prompt_cache_base_url().to_owned()),
            ("api_mode", self.api_mode_key().to_owned()),
            ("stream_mode", self.stream_mode_key().to_owned()),
            ("auth_header", self.auth_header.clone()),
            (
                "profile",
                match self.profile {
                    OpenAiProfile::Standard => "standard",
                    OpenAiProfile::GithubCopilot => "github_copilot",
                }
                .to_owned(),
            ),
            (
                "capability_family",
                match self.capability_family {
                    CapabilityFamily::OpenAi => "openai",
                    CapabilityFamily::OpenAiCompatible => "openai_compatible",
                    CapabilityFamily::Anthropic => "anthropic",
                    CapabilityFamily::Gemini => "gemini",
                    CapabilityFamily::Bedrock => "bedrock",
                    CapabilityFamily::Gitlab => "gitlab",
                }
                .to_owned(),
            ),
            (
                "uses_responses",
                self.should_use_responses(model.as_str()).to_string(),
            ),
            (
                "supports_top_level_prompt_cache",
                self.supports_top_level_prompt_cache().to_string(),
            ),
            (
                "extra_headers",
                crate::provider::PromptCacheShape::json_field_value(
                    &utils::prompt_cache_header_entries(&self.extra_headers),
                ),
            ),
        ];
        if let Some(models_url) = self.models_url.as_deref() {
            fields.push(("models_url", models_url.to_owned()));
        }
        if let Some(auth_scheme) = self.auth_scheme.as_deref() {
            fields.push(("auth_scheme", auth_scheme.to_owned()));
        }
        if let Some(auth_account_id) = self.chatgpt_account_id() {
            fields.push(("auth_account_id", auth_account_id));
        }
        if let Some(realtime_ws_url) = self.realtime_ws_url.as_deref() {
            fields.push(("realtime_ws_url", realtime_ws_url.to_owned()));
        }
        Some(crate::provider::PromptCacheShape::from_fields(
            self.id.as_str(),
            fields,
        ))
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let endpoint = self.list_models_endpoint()?;
        let response = utils::send_with_credential_refresh(&self.api_key, |api_key| {
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
        .await?;

        let payload: OpenAiModelListResponse = utils::parse_json_response_logged(
            self.id.as_str(),
            ADAPTER_KIND,
            "list_models",
            response,
        )
        .await?;
        Ok(payload
            .into_items(self.id.as_str(), self.models_url.as_deref())
            .into_iter()
            .filter_map(|model| self.provider_model_from_listed_model(model))
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

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = Self::responses_reasoning_config(&request, model.as_str());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            include: Self::responses_include(tool_plan.include, reasoning.as_ref()),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            store: false,
            stream: false,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            reasoning,
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
            client_metadata: Self::responses_client_metadata(RequestHeaderContext::from_request(
                &request,
            )),
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
        let reasoning_text = Self::extract_reasoning_text(&response);
        let finish_reason = CompletionFinishReason::from_provider(response.stop_reason.as_deref());
        let text = Self::extract_text(&response);
        let tool_calls = Self::parse_responses_tool_calls(response.output.as_ref())?;
        let finish_reason =
            responses_finish_reason_with_tool_calls(finish_reason, !tool_calls.is_empty());

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
        let input = self.responses_input_for_request(&input_request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let body = OpenAiResponsesCompactRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            include: (!tool_plan.include.is_empty()).then_some(tool_plan.include),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            prompt_cache_key: request.prompt_cache_key.clone(),
            reasoning: Self::responses_reasoning_config(&request, model.as_str()),
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
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

        let input = self.responses_input_for_request(&request)?;
        let tool_plan = self.responses_tool_plan_for_request(&request)?;
        let reasoning = Self::responses_reasoning_config(&request, model.as_str());

        let body = OpenAiResponsesRequest {
            model: model.to_string(),
            instructions: Self::responses_instructions(&request),
            input,
            tools: tool_plan.tools,
            tool_choice: "auto".to_owned(),
            parallel_tool_calls: Self::responses_parallel_tool_calls(&request),
            include: Self::responses_include(tool_plan.include, reasoning.as_ref()),
            max_output_tokens: self.responses_request_max_output_tokens(&request),
            temperature: request.temperature,
            prompt_cache_key: request.prompt_cache_key.clone(),
            previous_response_id: request.previous_response_id.clone(),
            store: false,
            stream: true,
            stop: (!request.stop_sequences.is_empty()).then(|| request.stop_sequences.clone()),
            top_p: request.top_p,
            seed: request.seed,
            reasoning,
            service_tier: Self::responses_service_tier(&request),
            text: Self::responses_text_config(&request),
            client_metadata: Self::responses_client_metadata(RequestHeaderContext::from_request(
                &request,
            )),
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
                reqwest::header::ACCEPT.as_str().to_owned(),
                "text/event-stream".to_owned(),
            );
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

        if self.should_require_sse_content_type() {
            utils::ensure_response_content_type(self.id.as_str(), &response, "text/event-stream")?;
        }
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
            let mut tool_stream = ToolStreamAccumulator::new();
            let mut stream_usage: Option<CompletionUsage> = None;
            let mut stream_finish_reason: Option<String> = None;
            let mut stream_has_content = false;
            let mut stream_tool_call_seen = false;
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

                if let Some(native_event) =
                    responses_native_tool_event(&provider_id, &model_name, &event)?
                {
                    stream_has_content = true;
                    yield native_event;
                }

                if let Some(tool_event) = utils::responses_tool_event(provider_name.as_str(), &event)? {
                    stream_tool_call_seen = true;
                    let input = responses_tool_stream_input(provider_name.as_str(), tool_event)?;
                    for update in tool_stream.ingest(provider_name.as_str(), input)? {
                        stream_has_content = true;
                        yield completion_event_from_tool_stream_update(
                            &provider_id,
                            &model_name,
                            update,
                        );
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
                    let finish_reason = responses_finish_reason_with_tool_calls(
                        CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                        stream_tool_call_seen,
                    );
                    yield CompletionStreamEvent::Completed {
                        provider_id: provider_id.clone(),
                        model: model_name.clone(),
                        finish_reason,
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
                let finish_reason = responses_finish_reason_with_tool_calls(
                    CompletionFinishReason::from_provider(stream_finish_reason.as_deref()),
                    stream_tool_call_seen,
                );
                yield CompletionStreamEvent::Completed {
                    provider_id: provider_id.clone(),
                    model: model_name.clone(),
                    finish_reason,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    tool_choice: String,
    parallel_tool_calls: bool,
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
    store: bool,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<OpenAiResponsesReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<OpenAiResponsesTextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_metadata: Option<HashMap<String, String>>,
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
    reasoning: Option<OpenAiResponsesReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<OpenAiResponsesTextConfig>,
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesReasoningConfig {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<OpenAiResponsesTextFormat>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponsesTextFormat {
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        strict: bool,
    },
}

impl OpenAiResponsesTextFormat {
    fn from_response_format(format: Option<&crate::provider::ResponseFormat>) -> Option<Self> {
        match format? {
            crate::provider::ResponseFormat::Text => None,
            crate::provider::ResponseFormat::JsonObject => Some(Self::JsonObject),
            crate::provider::ResponseFormat::JsonSchema {
                name,
                schema,
                strict,
            } => Some(Self::JsonSchema {
                name: name.clone(),
                schema: schema.clone(),
                strict: *strict,
            }),
        }
    }
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
    InputText { text: String },
    #[serde(rename = "output_text")]
    OutputText { text: String },
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

impl OpenAiInputContent {
    fn text_for_role(role: &str, text: String) -> Self {
        if role == "assistant" {
            Self::OutputText { text }
        } else {
            Self::InputText { text }
        }
    }
}

fn validate_responses_input(input: &[OpenAiResponsesInputItem]) -> Result<(), AppError> {
    let mut seen_tool_calls = BTreeSet::new();

    for (index, item) in input.iter().enumerate() {
        match item {
            OpenAiResponsesInputItem::Message(message) => {
                validate_responses_message(index, message)?;
            }
            OpenAiResponsesInputItem::FunctionCall(item) => {
                if !protocol_ids::valid_openai_responses_call_id(item.call_id.as_str()) {
                    return Err(AppError::Internal(format!(
                        "invalid OpenAI Responses function_call call_id at input[{index}]"
                    )));
                }
                if item.name.trim().is_empty() {
                    return Err(AppError::Internal(format!(
                        "invalid OpenAI Responses function_call name at input[{index}]"
                    )));
                }
                seen_tool_calls.insert(item.call_id.clone());
            }
            OpenAiResponsesInputItem::FunctionCallOutput(item) => {
                if !protocol_ids::valid_openai_responses_call_id(item.call_id.as_str()) {
                    return Err(AppError::Internal(format!(
                        "invalid OpenAI Responses function_call_output call_id at input[{index}]"
                    )));
                }
                if !seen_tool_calls.contains(item.call_id.as_str()) {
                    return Err(AppError::Internal(format!(
                        "OpenAI Responses function_call_output at input[{index}] references unknown call_id `{}`",
                        item.call_id
                    )));
                }
            }
        }
    }

    Ok(())
}

fn validate_responses_message(index: usize, message: &OpenAiInputMessage) -> Result<(), AppError> {
    let role = message.role.trim();
    if role.is_empty() {
        return Err(AppError::Internal(format!(
            "OpenAI Responses message at input[{index}] has empty role"
        )));
    }
    if message.content.is_empty() {
        return Err(AppError::Internal(format!(
            "OpenAI Responses message at input[{index}] has empty content"
        )));
    }

    for content in &message.content {
        match (role, content) {
            ("assistant", OpenAiInputContent::InputText { .. }) => {
                return Err(AppError::Internal(format!(
                    "OpenAI Responses assistant message at input[{index}] used input_text; assistant history must use output_text"
                )));
            }
            (role, OpenAiInputContent::OutputText { .. }) if role != "assistant" => {
                return Err(AppError::Internal(format!(
                    "OpenAI Responses {role} message at input[{index}] used output_text"
                )));
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,
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

struct OpenAiResponsesWireToolName {
    namespace: Option<String>,
    name: String,
}

fn responses_wire_tool_name(name: &str) -> OpenAiResponsesWireToolName {
    responses_native_tool_name(name).unwrap_or_else(|| OpenAiResponsesWireToolName {
        namespace: None,
        name: crate::tool::model_safe_tool_name(name),
    })
}

fn openai_chat_tool_name(name: &str) -> String {
    crate::tool::model_safe_tool_name(name)
}

fn responses_native_tool_name(name: &str) -> Option<OpenAiResponsesWireToolName> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((namespace, local_name)) = trimmed.split_once('.')
        && !local_name.contains('.')
        && responses_simple_tool_identifier(namespace)
        && responses_simple_tool_identifier(local_name)
    {
        return Some(OpenAiResponsesWireToolName {
            namespace: Some(namespace.to_owned()),
            name: local_name.to_owned(),
        });
    }

    if responses_simple_tool_identifier(trimmed) {
        return Some(OpenAiResponsesWireToolName {
            namespace: None,
            name: trimmed.to_owned(),
        });
    }

    None
}

fn responses_model_tool_name(namespace: Option<&str>, name: &str) -> String {
    let name = name.trim();
    match namespace.map(str::trim).filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}.{name}"),
        None => name.to_owned(),
    }
}

fn responses_simple_tool_identifier(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn responses_tool_stream_input(
    provider_id: &str,
    event: utils::ResponsesToolEvent,
) -> Result<ToolStreamInput, AppError> {
    let stream_key_candidates = event
        .stream_key_candidates(provider_id)?
        .into_iter()
        .filter_map(ProviderStreamKey::new)
        .collect::<Vec<_>>();
    if stream_key_candidates.is_empty() {
        return Err(AppError::Provider(format!(
            "{provider_id} returned tool event without usable stream key candidates"
        )));
    }

    let model_call_id = event
        .call_id
        .as_deref()
        .and_then(protocol_ids::openai_responses_call_id)
        .or_else(|| {
            event
                .id
                .as_deref()
                .and_then(protocol_ids::openai_responses_call_id)
        });

    let name = event
        .name
        .as_deref()
        .map(|name| responses_model_tool_name(event.namespace.as_deref(), name));

    Ok(ToolStreamInput {
        kind: match event.kind {
            utils::ResponsesToolEventKind::Added => ToolStreamInputKind::Start,
            utils::ResponsesToolEventKind::Delta => ToolStreamInputKind::Delta,
            utils::ResponsesToolEventKind::Done => ToolStreamInputKind::Finish,
        },
        stream_key_candidates,
        provider_item_id: event.item_id.and_then(ProviderItemId::new),
        model_call_id,
        name,
        arguments: event.arguments,
    })
}

fn completion_event_from_tool_stream_update(
    provider_id: &ProviderId,
    model: &ModelId,
    update: ToolStreamUpdate,
) -> CompletionStreamEvent {
    match update {
        ToolStreamUpdate::Registered {
            stream_key,
            id,
            name,
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_delta: String::new(),
        },
        ToolStreamUpdate::ArgumentsDelta {
            stream_key,
            id,
            name,
            arguments_delta,
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_delta,
        },
        ToolStreamUpdate::ArgumentsSnapshot {
            stream_key,
            id,
            name,
            arguments_json,
        } => CompletionStreamEvent::ToolCallSnapshot {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_json,
        },
    }
}

#[derive(Debug)]
enum OpenAiNativeToolEvent {
    Started {
        stream_key: String,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        raw: Option<serde_json::Value>,
    },
    Completed {
        stream_key: String,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        output_text: String,
        blocks: Vec<OperationBlock>,
        details: ToolOutput,
        raw: Option<serde_json::Value>,
    },
}

fn responses_native_tool_event(
    provider_id: &ProviderId,
    model: &ModelId,
    event: &serde_json::Value,
) -> Result<Option<CompletionStreamEvent>, AppError> {
    let Some(event_type) = event.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    if !matches!(
        event_type,
        "response.output_item.added" | "response.output_item.done"
    ) {
        return Ok(None);
    }

    let Some(item) = event.get("item") else {
        return Ok(None);
    };
    let Some(item_kind) = item.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };

    let native_event = match item_kind {
        "web_search_call" => openai_web_search_tool_event(event_type, event, item)?,
        "file_search_call" => openai_file_search_tool_event(event_type, event, item)?,
        "code_interpreter_call" => openai_code_interpreter_tool_event(event_type, event, item)?,
        "image_generation_call" => openai_image_generation_tool_event(event_type, event, item)?,
        _ => None,
    };

    Ok(native_event.map(|native_event| match native_event {
        OpenAiNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        } => CompletionStreamEvent::NativeToolCallStarted {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            invocation,
            title,
            raw,
        },
        OpenAiNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            output_text,
            blocks,
            details,
            raw,
        } => CompletionStreamEvent::NativeToolCallCompleted {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            invocation,
            title,
            output_text,
            blocks,
            details,
            raw,
        },
    }))
}

fn openai_web_search_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiNativeToolEvent>, AppError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key = native_responses_stream_key(id.as_deref(), output_index).ok_or_else(|| {
        AppError::Provider(
            "openai responses web_search_call event was missing both item id and output index"
                .to_owned(),
        )
    })?;

    let action = item.get("action");
    let invocation = openai_web_search_invocation(action)?;
    let details = ToolOutput::from_json_payload(Some(item)).map_err(AppError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title: String::new(),
            raw,
        }
    } else {
        OpenAiNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title: String::new(),
            output_text: String::new(),
            blocks: openai_web_search_blocks(action),
            details,
            raw,
        }
    }))
}

fn openai_file_search_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiNativeToolEvent>, AppError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key = native_responses_stream_key(id.as_deref(), output_index).ok_or_else(|| {
        AppError::Provider(
            "openai responses file_search_call event was missing both item id and output index"
                .to_owned(),
        )
    })?;

    let queries = openai_file_search_queries(item);
    let invocation = openai_file_search_invocation(queries.as_slice())?;
    let title = openai_file_search_title(queries.as_slice());
    let details = ToolOutput::from_json_payload(Some(item)).map_err(AppError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        OpenAiNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            output_text: String::new(),
            blocks: openai_file_search_blocks(queries.as_slice(), item.get("results")),
            details,
            raw,
        }
    }))
}

fn openai_code_interpreter_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiNativeToolEvent>, AppError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key =
        native_responses_stream_key(id.as_deref(), output_index).ok_or_else(|| {
            AppError::Provider(
                "openai responses code_interpreter_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let code = item
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let invocation = openai_code_interpreter_invocation(code)?;
    let title = if code.is_some() {
        "code execution".to_owned()
    } else {
        "code interpreter".to_owned()
    };
    let details = ToolOutput::from_json_payload(Some(item)).map_err(AppError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        let blocks = openai_code_interpreter_blocks(item.get("outputs"));
        OpenAiNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            output_text: openai_code_interpreter_output_text(blocks.as_slice()),
            blocks,
            details,
            raw,
        }
    }))
}

fn openai_image_generation_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiNativeToolEvent>, AppError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key =
        native_responses_stream_key(id.as_deref(), output_index).ok_or_else(|| {
            AppError::Provider(
                "openai responses image_generation_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let revised_prompt = item
        .get("revised_prompt")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let invocation = openai_image_generation_invocation(revised_prompt)?;
    let title = revised_prompt
        .map(|prompt| format!("image generation {prompt}"))
        .unwrap_or_else(|| "image generation".to_owned());
    let details = ToolOutput::from_json_payload(Some(item)).map_err(AppError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        OpenAiNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            output_text: revised_prompt.unwrap_or_default().to_owned(),
            blocks: openai_image_generation_blocks(item),
            details,
            raw,
        }
    }))
}

fn native_responses_stream_key(
    item_id: Option<&str>,
    output_index: Option<usize>,
) -> Option<String> {
    item_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("item:{value}"))
        .or_else(|| output_index.map(|value| format!("idx:{value}")))
}

fn openai_web_search_invocation(
    action: Option<&serde_json::Value>,
) -> Result<ToolInvocation, AppError> {
    let action_type = action
        .and_then(|value| value.get("type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    let input = match action_type {
        "search" => {
            let detail = web_search_action_detail(action);
            StructuredObject::try_from(if detail.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ "query": detail })
            })
            .map_err(AppError::Provider)?
        }
        "open_page" => {
            let payload = if let Some(url) = action
                .and_then(|value| value.get("url"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                serde_json::json!({ "url": url })
            } else {
                serde_json::json!({})
            };
            StructuredObject::try_from(payload).map_err(AppError::Provider)?
        }
        "find_in_page" => {
            let url = action
                .and_then(|value| value.get("url"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let pattern = action
                .and_then(|value| value.get("pattern"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let payload = match (url, pattern) {
                (Some(url), Some(pattern)) => serde_json::json!({
                    "url": url,
                    "pattern": pattern
                }),
                (Some(url), None) => serde_json::json!({ "url": url }),
                (None, Some(pattern)) => serde_json::json!({ "pattern": pattern }),
                (None, None) => serde_json::json!({}),
            };
            StructuredObject::try_from(payload).map_err(AppError::Provider)?
        }
        _ => StructuredObject::default(),
    };

    Ok(ToolInvocation::new("web.run", input))
}

fn openai_web_search_blocks(action: Option<&serde_json::Value>) -> Vec<OperationBlock> {
    let Some(sources) = action
        .and_then(|value| value.get("sources"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    let results = sources
        .iter()
        .filter_map(openai_web_search_result)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Vec::new();
    }

    let query = web_search_action_detail(action);
    vec![OperationBlock::SearchResults {
        query: (!query.is_empty()).then_some(query),
        results,
    }]
}

fn openai_file_search_queries(item: &serde_json::Value) -> Vec<String> {
    item.get("queries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn openai_file_search_invocation(queries: &[String]) -> Result<ToolInvocation, AppError> {
    let input = match queries {
        [] => StructuredObject::default(),
        [query] => StructuredObject::try_from(serde_json::json!({ "query": query }))
            .map_err(AppError::Provider)?,
        [first, ..] => StructuredObject::try_from(serde_json::json!({
            "query": first,
            "queries": queries,
        }))
        .map_err(AppError::Provider)?,
    };
    Ok(ToolInvocation::new("file_search", input))
}

fn openai_file_search_title(queries: &[String]) -> String {
    queries
        .first()
        .map(|query| format!("file search {query}"))
        .unwrap_or_else(|| "file search".to_owned())
}

fn openai_file_search_blocks(
    queries: &[String],
    results: Option<&serde_json::Value>,
) -> Vec<OperationBlock> {
    let Some(results) = results.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let results = results
        .iter()
        .filter_map(openai_file_search_result)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Vec::new();
    }

    let query = queries.first().cloned().map(|first| {
        if queries.len() > 1 {
            format!("{first} ...")
        } else {
            first
        }
    });
    vec![OperationBlock::SearchResults { query, results }]
}

fn openai_file_search_result(source: &serde_json::Value) -> Option<SearchResultItem> {
    let file_id = source
        .get("file_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let filename = source
        .get("filename")
        .or_else(|| source.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let uri = file_id
        .as_deref()
        .map(|value| format!("file:{value}"))
        .or_else(|| filename.clone())?;
    let title = filename
        .or(file_id)
        .unwrap_or_else(|| "file result".to_owned());
    let snippet = source
        .get("text")
        .or_else(|| source.get("snippet"))
        .or_else(|| source.get("summary"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let score = source
        .get("score")
        .or_else(|| source.get("rank"))
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32);

    Some(SearchResultItem {
        title,
        uri,
        snippet,
        score,
    })
}

fn openai_code_interpreter_invocation(code: Option<&str>) -> Result<ToolInvocation, AppError> {
    let input = match code {
        Some(code) => StructuredObject::try_from(serde_json::json!({ "code": code }))
            .map_err(AppError::Provider)?,
        None => StructuredObject::default(),
    };
    Ok(ToolInvocation::new("code_execution", input))
}

fn openai_code_interpreter_blocks(outputs: Option<&serde_json::Value>) -> Vec<OperationBlock> {
    let Some(outputs) = outputs.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let mut blocks = Vec::new();
    for output in outputs {
        let output_type = output
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match output_type {
            "logs" => {
                let Some(logs) = output
                    .get("logs")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                blocks.push(OperationBlock::Text {
                    text: logs.to_owned(),
                });
            }
            "files" => {
                let Some(files) = output.get("files").and_then(serde_json::Value::as_array) else {
                    continue;
                };
                for file in files {
                    let Some(file_id) = file
                        .get("file_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let mime_type = file
                        .get("mime_type")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("application/octet-stream")
                        .to_owned();
                    let name = file
                        .get("filename")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| file_id.to_owned());
                    blocks.push(OperationBlock::Media {
                        mime_type: mime_type.clone(),
                        artifact: ArtifactRef {
                            uri: format!("file:{file_id}"),
                            mime: mime_type,
                            name: Some(name),
                            size_bytes: None,
                            sha256: None,
                        },
                    });
                }
            }
            _ => {
                let pretty = serde_json::to_string_pretty(output)
                    .unwrap_or_else(|_| output.to_string())
                    .trim()
                    .to_owned();
                if !pretty.is_empty() {
                    blocks.push(OperationBlock::Text { text: pretty });
                }
            }
        }
    }

    blocks
}

fn openai_code_interpreter_output_text(blocks: &[OperationBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            OperationBlock::Text { text } if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn openai_image_generation_invocation(
    revised_prompt: Option<&str>,
) -> Result<ToolInvocation, AppError> {
    let input = match revised_prompt {
        Some(prompt) => StructuredObject::try_from(serde_json::json!({ "description": prompt }))
            .map_err(AppError::Provider)?,
        None => StructuredObject::default(),
    };
    Ok(ToolInvocation::new("image_generation", input))
}

fn openai_image_generation_blocks(item: &serde_json::Value) -> Vec<OperationBlock> {
    let Some(result) = item
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    let mime_type = item
        .get("mime_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("image/png")
        .to_owned();
    let extension = mime_type
        .strip_prefix("image/")
        .filter(|value| !value.is_empty())
        .unwrap_or("png");
    let extension = extension.to_owned();
    let data_url = format!("data:{mime_type};base64,{result}");

    vec![OperationBlock::Media {
        mime_type: mime_type.clone(),
        artifact: ArtifactRef {
            uri: data_url,
            mime: mime_type,
            name: Some(format!("generated-image.{extension}")),
            size_bytes: None,
            sha256: None,
        },
    }]
}

fn openai_web_search_result(source: &serde_json::Value) -> Option<SearchResultItem> {
    let uri = source
        .get("url")
        .or_else(|| source.get("uri"))
        .or_else(|| source.get("link"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let title = source
        .get("title")
        .or_else(|| source.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uri.clone());
    let snippet = source
        .get("snippet")
        .or_else(|| source.get("summary"))
        .or_else(|| source.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let score = source
        .get("score")
        .or_else(|| source.get("rank"))
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32);

    Some(SearchResultItem {
        title,
        uri,
        snippet,
        score,
    })
}

fn web_search_action_detail(action: Option<&serde_json::Value>) -> String {
    let Some(action) = action else {
        return String::new();
    };
    let action_type = action
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match action_type {
        "search" => action
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                let items = action
                    .get("queries")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let first = items
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_default()
                    .to_owned();
                if items.len() > 1 && !first.is_empty() {
                    format!("{first} ...")
                } else {
                    first
                }
            }),
        "open_page" => action
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default(),
        "find_in_page" => {
            let url = action
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let pattern = action
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match (pattern, url) {
                (Some(pattern), Some(url)) => format!("'{pattern}' in {url}"),
                (Some(pattern), None) => format!("'{pattern}'"),
                (None, Some(url)) => url.to_owned(),
                (None, None) => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn responses_finish_reason_with_tool_calls(
    finish_reason: Option<CompletionFinishReason>,
    saw_tool_call: bool,
) -> Option<CompletionFinishReason> {
    if saw_tool_call && matches!(finish_reason, None | Some(CompletionFinishReason::Stop)) {
        return Some(CompletionFinishReason::ToolCalls);
    }
    finish_reason
}

fn responses_output_call_id(call_id: Option<&str>, item_id: Option<&str>) -> Option<String> {
    call_id
        .and_then(protocol_ids::openai_responses_call_id)
        .map(|id| id.into_string())
        .or_else(|| item_id.and_then(responses_input_call_id))
}

fn responses_input_call_id(raw: &str) -> Option<String> {
    protocol_ids::openai_responses_call_id(raw).map(|id| id.into_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashscopeReasoningProfile {
    Toggleable,
    AlwaysOn,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiModelListResponse {
    CodexWrapped {
        models: Vec<OpenAiCodexModel>,
    },
    Wrapped {
        data: Vec<OpenAiCompatibleModel>,
    },
    ClineRecommended {
        #[serde(default)]
        recommended: Vec<OpenAiRecommendedModel>,
        #[serde(default)]
        free: Vec<OpenAiRecommendedModel>,
        #[serde(rename = "clinePass", default)]
        cline_pass: Vec<OpenAiRecommendedModel>,
    },
    Bare(Vec<OpenAiCompatibleModel>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiChatCompletionResponse {
    Wrapped {
        data: ChatCompletionResponse,
        #[serde(default, rename = "success")]
        _success: Option<bool>,
    },
    Bare(ChatCompletionResponse),
}

impl OpenAiModelListResponse {
    fn into_items(self, provider_id: &str, models_url: Option<&str>) -> Vec<OpenAiListedModel> {
        match self {
            Self::CodexWrapped { models } => {
                models.into_iter().map(OpenAiListedModel::Codex).collect()
            }
            Self::Wrapped { data } => data
                .into_iter()
                .map(OpenAiListedModel::Compatible)
                .collect(),
            Self::ClineRecommended {
                recommended,
                free,
                cline_pass,
            } => cline_recommended_models_for_provider(
                provider_id,
                models_url,
                recommended,
                free,
                cline_pass,
            )
            .into_iter()
            .map(OpenAiListedModel::Recommended)
            .collect(),
            Self::Bare(data) => data
                .into_iter()
                .map(OpenAiListedModel::Compatible)
                .collect(),
        }
    }
}

#[derive(Debug)]
enum OpenAiListedModel {
    Compatible(OpenAiCompatibleModel),
    Recommended(OpenAiRecommendedModel),
    Codex(OpenAiCodexModel),
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleModel {
    id: String,
    #[serde(default, flatten)]
    copilot: CopilotModelExtension,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, alias = "context_length")]
    context_window_tokens: Option<u64>,
    #[serde(default, alias = "input_token_limit")]
    max_input_tokens: Option<u64>,
    #[serde(default, alias = "max_completion_tokens")]
    max_output_tokens: Option<u64>,
}

impl OpenAiCompatibleModel {
    fn metadata(&self) -> ModelMetadata {
        let metadata = ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: self
                    .context_window_tokens
                    .or(self.max_input_tokens)
                    .map(clamp_u64_to_u32),
                max_input_tokens: self.max_input_tokens.map(clamp_u64_to_u32),
                max_output_tokens: self.max_output_tokens.map(clamp_u64_to_u32),
            },
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        };
        metadata.merged_with_fallbacks_from(&self.copilot.metadata(self.id.as_str()))
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiRecommendedModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

impl OpenAiRecommendedModel {
    fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits::default(),
            description: self
                .description
                .clone()
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
    }
}

fn cline_recommended_models_for_provider(
    provider_id: &str,
    models_url: Option<&str>,
    recommended: Vec<OpenAiRecommendedModel>,
    free: Vec<OpenAiRecommendedModel>,
    cline_pass: Vec<OpenAiRecommendedModel>,
) -> Vec<OpenAiRecommendedModel> {
    let provider_key = provider_id.trim().to_ascii_lowercase();
    let models_url_key = models_url.unwrap_or_default().trim().to_ascii_lowercase();
    let is_cline_pass_provider = provider_key.contains("cline-pass")
        || provider_key.contains("cline_pass")
        || provider_key.contains("cline_api")
        || provider_key.contains("clineapi")
        || models_url_key.contains("/ai/cline/recommended-models");
    let mut selected = if is_cline_pass_provider {
        cline_pass
    } else {
        let mut combined = recommended;
        combined.extend(free);
        if combined.is_empty() {
            cline_pass
        } else {
            combined
        }
    };

    let mut seen = BTreeSet::new();
    selected.retain(|model| {
        let Some(id) = utils::normalize_optional_text(Some(model.id.clone())) else {
            return false;
        };
        seen.insert(id)
    });
    selected
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexModel {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<OpenAiCodexReasoningLevel>,
    #[serde(default)]
    support_verbosity: Option<bool>,
    #[serde(default)]
    default_verbosity: Option<String>,
    #[serde(default)]
    supports_parallel_tool_calls: Option<bool>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_context_window: Option<u64>,
    #[serde(default)]
    input_modalities: Vec<String>,
}

impl OpenAiCodexModel {
    fn metadata(&self) -> ModelMetadata {
        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let default_verbosity = self
            .default_verbosity
            .as_ref()
            .and_then(|value| utils::normalize_optional_text(Some(value.clone())));
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: self
                    .context_window
                    .or(self.max_context_window)
                    .map(clamp_u64_to_u32),
                max_input_tokens: None,
                max_output_tokens: None,
            },
            description,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: self.default_thinking_mode_key(),
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.support_verbosity,
            default_verbosity,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
    }

    fn capabilities(&self) -> ModelCapabilities {
        let supports = |modality: ModelInputModality| {
            if self.input_modalities.is_empty() {
                return CapabilitySupport::Unknown;
            }
            if self
                .input_modalities
                .iter()
                .any(|value| model_supports_input_modality(value.as_str(), modality))
            {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            }
        };
        let text_input = match supports(ModelInputModality::Text) {
            CapabilitySupport::Unsupported => CapabilitySupport::Unsupported,
            _ => CapabilitySupport::Supported,
        };
        ModelCapabilities {
            text_input,
            image_input: supports(ModelInputModality::Image),
            document_input: supports(ModelInputModality::Document),
            audio_input: supports(ModelInputModality::Audio),
            video_input: supports(ModelInputModality::Video),
            file_input: supports(ModelInputModality::File),
            tool_calling: if self.supports_parallel_tool_calls == Some(true) {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unknown
            },
            reasoning: if self.supported_reasoning_levels.is_empty() {
                CapabilitySupport::Unknown
            } else {
                CapabilitySupport::Supported
            },
            ..ModelCapabilities::default()
        }
    }

    fn default_thinking_mode_key(&self) -> Option<String> {
        let normalized = self
            .default_reasoning_level
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_ascii_lowercase();
        if normalized == "none" {
            Some("no-thinking".to_owned())
        } else {
            Some(format!("thinking-{normalized}"))
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OpenAiCodexReasoningLevel {
    effort: String,
    #[serde(default)]
    description: Option<String>,
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
    namespace: Option<String>,
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
        || event_type == "response.reasoning_text.delta"
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

fn openai_client_version() -> String {
    crate::provider::CODEX_PACKAGE_VERSION.to_owned()
}

fn append_query_param(endpoint: &str, key: &str, value: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}{key}={value}")
}

fn model_supports_input_modality(input_modality: &str, modality: ModelInputModality) -> bool {
    let normalized = input_modality.trim().to_ascii_lowercase();
    match modality {
        ModelInputModality::Text => normalized == "text",
        ModelInputModality::Image => normalized == "image",
        ModelInputModality::Document => normalized == "document",
        ModelInputModality::Audio => normalized == "audio",
        ModelInputModality::Video => normalized == "video",
        ModelInputModality::File => normalized == "file",
    }
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}
