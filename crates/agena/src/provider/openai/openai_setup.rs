use super::{
    AppError, AuthData, CHATGPT_CODEX_ORIGINATOR, CapabilityFamily, DEFAULT_COPILOT_BASE_URL,
    DashscopeReasoningProfile, HashMap, ManagedCredential, ModelId, OpenAiChatCompletionResponse,
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiProfile,
    OpenAiRealtimeAdapter, OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter,
    OpenAiResponsesAdapterOptions, OpenAiResponsesBackend, OpenAiTransport, OpenAiTransportOptions,
    append_query_param, openai_client_version, utils,
};
use crate::{model::ModelThinkingMode, provider::chat_wire::ChatCompletionResponse};
use std::collections::BTreeMap;

impl From<OpenAiResponsesAdapterOptions> for OpenAiTransportOptions {
    fn from(options: OpenAiResponsesAdapterOptions) -> Self {
        Self {
            backend: options.backend,
            auth_data: options.auth_data,
            profile: options.profile,
            models_url: options.models_url,
            auth_header: options.auth_header,
            auth_scheme: options.auth_scheme,
            capability_family: options.capability_family,
            extra_headers: options.extra_headers,
            top_level_prompt_cache_override: options.top_level_prompt_cache_override,
        }
    }
}

impl From<OpenAiChatCompletionsAdapterOptions> for OpenAiTransportOptions {
    fn from(options: OpenAiChatCompletionsAdapterOptions) -> Self {
        Self {
            backend: OpenAiResponsesBackend::Api,
            auth_data: options.auth_data,
            profile: options.profile,
            models_url: options.models_url,
            auth_header: options.auth_header,
            auth_scheme: options.auth_scheme,
            capability_family: options.capability_family,
            extra_headers: options.extra_headers,
            top_level_prompt_cache_override: options.top_level_prompt_cache_override,
        }
    }
}

impl From<&OpenAiRealtimeAdapterOptions> for OpenAiTransportOptions {
    fn from(options: &OpenAiRealtimeAdapterOptions) -> Self {
        Self {
            backend: OpenAiResponsesBackend::Api,
            auth_data: options.auth_data.clone(),
            profile: OpenAiProfile::Standard,
            models_url: options.models_url.clone(),
            auth_header: options.auth_header.clone(),
            auth_scheme: options.auth_scheme.clone(),
            capability_family: options.capability_family,
            extra_headers: options.extra_headers.clone(),
            top_level_prompt_cache_override: None,
        }
    }
}

impl OpenAiResponsesAdapter {
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
            OpenAiResponsesAdapterOptions::default(),
        )
    }

    pub fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: OpenAiResponsesAdapterOptions,
    ) -> Self {
        Self {
            transport: OpenAiTransport::new_managed_with_options(
                id,
                client,
                api_key,
                base_url,
                default_model,
                options.into(),
            ),
        }
    }
}

impl OpenAiChatCompletionsAdapter {
    pub fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: OpenAiChatCompletionsAdapterOptions,
    ) -> Self {
        Self {
            transport: OpenAiTransport::new_managed_with_options(
                id,
                client,
                api_key,
                base_url,
                default_model,
                options.into(),
            ),
        }
    }
}

impl OpenAiRealtimeAdapter {
    pub fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: OpenAiRealtimeAdapterOptions,
    ) -> Self {
        let realtime_ws_url = options
            .realtime_ws_url
            .clone()
            .and_then(|value| utils::normalize_optional_text(Some(value)));
        Self {
            transport: OpenAiTransport::new_managed_with_options(
                id,
                client,
                api_key,
                base_url,
                default_model,
                (&options).into(),
            ),
            realtime_ws_url,
        }
    }
}

impl OpenAiTransport {
    fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: OpenAiTransportOptions,
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
            profile: options.profile,
            models_url: options
                .models_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            auth_header: options.auth_header,
            auth_scheme: options.auth_scheme,
            capability_family: options.capability_family,
            extra_headers,
            top_level_prompt_cache_override: options.top_level_prompt_cache_override,
        }
    }

    pub(super) fn configured_public_copilot_base_url(&self) -> bool {
        self.base_url.trim_end_matches('/') == DEFAULT_COPILOT_BASE_URL
    }

    pub(super) fn resolved_base_url(&self) -> Result<String, AppError> {
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

        Ok(format!(
            "https://copilot-api.{}",
            utils::normalize_domain(&domain)
        ))
    }

    pub(super) fn prompt_cache_base_url(&self) -> String {
        self.resolved_base_url()
            .unwrap_or_else(|_| self.base_url.clone())
    }

    pub(super) fn model_endpoint(&self) -> Result<String, AppError> {
        Ok(self.models_url.clone().unwrap_or_else(|| {
            format!(
                "{}/models",
                self.prompt_cache_base_url().trim_end_matches('/')
            )
        }))
    }

    pub(super) fn list_models_endpoint(&self) -> Result<String, AppError> {
        let endpoint = self.model_endpoint()?;
        if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex) {
            Ok(append_query_param(
                endpoint.as_str(),
                "client_version",
                openai_client_version().as_str(),
            ))
        } else {
            Ok(endpoint)
        }
    }

    pub(super) fn responses_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/responses",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn responses_compact_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/responses/compact",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn chat_endpoint(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/chat/completions",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn is_openai_compatible_family(&self) -> bool {
        matches!(self.capability_family, CapabilityFamily::OpenAiCompatible)
    }

    pub(super) fn uses_chat_compatible_request_fields(&self) -> bool {
        self.is_openai_compatible_family()
    }

    /// `prompt_cache_key` is an OpenAI extension, not part of the portable
    /// Chat Completions schema. Only emit it for endpoints known to implement
    /// the field; callers can still add it explicitly with a body override.
    pub(super) fn supports_chat_prompt_cache_key(&self) -> bool {
        if self.profile == OpenAiProfile::GithubCopilot {
            return false;
        }
        if self.is_official_openai_endpoint() {
            return true;
        }
        self.matches_known_chat_extension_provider(&[
            "openrouter.ai",
            "zenmux.ai",
            "api.kilo.ai",
            "opencode.ai",
        ]) || matches!(
            self.id.to_ascii_lowercase().as_str(),
            "openrouter" | "zenmux" | "kilo" | "opencode"
        )
    }

    /// xAI Chat Completions uses an affinity header rather than the
    /// Responses-only `prompt_cache_key` request field.
    pub(super) fn is_xai_endpoint(&self) -> bool {
        self.matches_known_chat_extension_provider(&["api.x.ai"])
            || matches!(self.id.to_ascii_lowercase().as_str(), "xai" | "grok")
    }

    /// Some compatible servers reject the optional `stream_options` object.
    /// Keep usage streaming on for implementations known to support it and
    /// omit the extension for an otherwise unknown compatible endpoint.
    pub(super) fn supports_chat_stream_usage(&self) -> bool {
        if self.profile == OpenAiProfile::GithubCopilot || self.is_official_openai_endpoint() {
            return true;
        }
        self.matches_known_chat_extension_provider(&[
            "api.x.ai",
            "openrouter.ai",
            "zenmux.ai",
            "api.kilo.ai",
            "opencode.ai",
            "dashscope.aliyuncs.com",
        ]) || matches!(
            self.id.to_ascii_lowercase().as_str(),
            "xai" | "grok" | "openrouter" | "zenmux" | "kilo" | "opencode" | "alibaba-cn"
        )
    }

    fn matches_known_chat_extension_provider(&self, host_suffixes: &[&str]) -> bool {
        url::Url::parse(self.base_url.as_str())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| {
                host_suffixes
                    .iter()
                    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            })
    }

    pub(super) fn supports_top_level_prompt_cache(&self) -> bool {
        if let Some(enabled) = self.top_level_prompt_cache_override {
            return enabled;
        }
        self.uses_chat_compatible_request_fields()
            && matches!(self.id.as_str(), "openrouter" | "zenmux" | "kilo")
    }

    pub(super) fn is_dashscope_compatible(&self) -> bool {
        self.id.eq_ignore_ascii_case("alibaba-cn")
            || self.id.to_ascii_lowercase().contains("dashscope")
            || url::Url::parse(self.base_url.as_str())
                .ok()
                .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                .is_some_and(|host| host.contains("dashscope") && host.contains("aliyuncs.com"))
    }

    pub(super) fn dashscope_reasoning_profile(model: &str) -> Option<DashscopeReasoningProfile> {
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

    pub(super) fn is_dashscope_reasoning_model(&self, model: &ModelId) -> bool {
        self.is_openai_compatible_family()
            && self.is_dashscope_compatible()
            && Self::dashscope_reasoning_profile(model.as_ref()).is_some()
    }

    pub(super) fn assistant_reasoning_field_for_model(
        &self,
        model: &ModelId,
    ) -> Option<&'static str> {
        self.is_dashscope_reasoning_model(model)
            .then_some("reasoning_content")
    }

    pub(super) fn apply_dashscope_reasoning_overrides(
        &self,
        model: &ModelId,
        thinking: Option<&crate::provider::ThinkingRequest>,
        request_override: &mut crate::model::ModelSpeedModeRequestOverride,
    ) {
        if !self.is_dashscope_compatible() {
            return;
        }

        let Some(profile) = Self::dashscope_reasoning_profile(model.as_ref()) else {
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

    pub(super) fn dashscope_thinking_modes(model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        let mut modes = BTreeMap::new();
        match Self::dashscope_reasoning_profile(model.as_ref()) {
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

    pub(super) fn backend_key(&self) -> &'static str {
        match self.backend {
            OpenAiResponsesBackend::Api => "api",
            OpenAiResponsesBackend::ChatgptCodex => "chatgpt_codex",
        }
    }

    pub(super) fn unwrap_chat_completion_response(
        payload: OpenAiChatCompletionResponse,
    ) -> ChatCompletionResponse {
        match payload {
            OpenAiChatCompletionResponse::Bare(response) => response,
            OpenAiChatCompletionResponse::Wrapped { data, .. } => data,
        }
    }

    pub(super) fn chatgpt_account_id(&self) -> Option<String> {
        self.auth_data
            .as_ref()
            .and_then(|auth| auth.try_lock().ok())
            .as_deref()
            .and_then(AuthData::account_id)
            .map(ToOwned::to_owned)
            .and_then(|value| utils::normalize_optional_text(Some(value)))
    }

    pub(super) fn chatgpt_account_is_fedramp(&self) -> bool {
        self.auth_data
            .as_ref()
            .and_then(|auth| auth.try_lock().ok())
            .as_deref()
            .is_some_and(AuthData::chatgpt_account_is_fedramp)
    }

    pub(super) fn supports_codex_compat_headers(&self) -> bool {
        matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex)
            || (matches!(self.profile, OpenAiProfile::Standard)
                && !self.is_openai_compatible_family())
    }

    pub(super) fn should_require_sse_content_type(&self) -> bool {
        !matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex)
    }

    pub(super) fn realtime_ws_endpoint(
        &self,
        model: &str,
        realtime_ws_url: Option<&str>,
    ) -> Result<url::Url, AppError> {
        let mut endpoint = if let Some(ws_url) = realtime_ws_url {
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

    pub(super) fn realtime_handshake_request(
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

    pub(super) fn is_official_openai_endpoint(&self) -> bool {
        url::Url::parse(self.base_url.as_str())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host == "api.openai.com" || host.ends_with(".openai.com"))
    }
}
