use super::{
    AuthData, CHATGPT_CODEX_ORIGINATOR, CapabilityFamily, DEFAULT_COPILOT_BASE_URL,
    DashscopeReasoningProfile, HashMap, ManagedCredential, ModelId, OpenAiChatCompletionResponse,
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiProfile,
    OpenAiResponsesAdapter,
    OpenAiResponsesAdapterOptions, OpenAiResponsesBackend, OpenAiTransport, OpenAiTransportOptions,
    ProviderError, append_query_param, normalize_domain, utils,
};
use crate::provider::chat_wire::ChatCompletionResponse;
use agena_domain::ModelThinkingMode;
use std::collections::BTreeMap;

impl From<OpenAiResponsesAdapterOptions> for OpenAiTransportOptions {
    fn from(options: OpenAiResponsesAdapterOptions) -> Self {
        Self {
            client_identity: options.client_identity.clone(),
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
            client_identity: options.client_identity.clone(),
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

impl OpenAiResponsesAdapter {
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
                options.client_identity.codex_user_agent(),
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
            api_key: api_key.with_client_identity(options.client_identity.clone()),
            client_identity: options.client_identity,
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

    pub(super) fn resolved_base_url(&self) -> Result<String, ProviderError> {
        if self.profile != OpenAiProfile::GithubCopilot
            || !self.configured_public_copilot_base_url()
        {
            return Ok(self.base_url.clone());
        }

        let Some(auth_data) = self.auth_data.as_ref() else {
            return Ok(self.base_url.clone());
        };

        let auth = auth_data.try_lock().map_err(|error| {
            ProviderError::Internal(format!(
                "cannot resolve GitHub Copilot base URL while authentication data is locked: {error}"
            ))
        })?;
        let Some(domain) = auth.enterprise_url().map(ToOwned::to_owned) else {
            return Ok(self.base_url.clone());
        };

        Ok(format!("https://copilot-api.{}", normalize_domain(&domain)))
    }

    pub(super) fn prompt_cache_base_url(&self) -> String {
        self.resolved_base_url().unwrap_or_else(|error| {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "resolve OpenAI prompt-cache base URL",
                    &error,
                ),
                "prompt-cache shape is using the configured OpenAI base URL"
            );
            self.base_url.clone()
        })
    }

    pub(super) fn model_endpoint(&self) -> Result<String, ProviderError> {
        if let Some(models_url) = &self.models_url {
            return Ok(models_url.clone());
        }
        Ok(format!(
            "{}/models",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn list_models_endpoint(&self) -> Result<String, ProviderError> {
        let endpoint = self.model_endpoint()?;
        if matches!(self.backend, OpenAiResponsesBackend::ChatgptCodex) {
            Ok(append_query_param(
                endpoint.as_str(),
                "client_version",
                self.client_identity.codex_package_version().as_str(),
            ))
        } else {
            Ok(endpoint)
        }
    }

    pub(super) fn responses_endpoint(&self) -> Result<String, ProviderError> {
        Ok(format!(
            "{}/responses",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn responses_compact_endpoint(&self) -> Result<String, ProviderError> {
        Ok(format!(
            "{}/responses/compact",
            self.resolved_base_url()?.trim_end_matches('/')
        ))
    }

    pub(super) fn chat_endpoint(&self) -> Result<String, ProviderError> {
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
        thinking: Option<&agena_domain::ThinkingRequest>,
        request_override: &mut agena_domain::ModelSpeedModeRequestOverride,
    ) {
        if !self.is_dashscope_compatible() {
            return;
        }

        let Some(profile) = Self::dashscope_reasoning_profile(model.as_ref()) else {
            return;
        };

        match thinking {
            Some(agena_domain::ThinkingRequest::Disabled) => {
                if matches!(profile, DashscopeReasoningProfile::Toggleable) {
                    request_override
                        .body_patch
                        .insert("enable_thinking".to_owned(), serde_json::Value::Bool(false));
                }
                request_override.body_patch.remove("thinking_budget");
            }
            Some(agena_domain::ThinkingRequest::Budget { budget_tokens }) => {
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

    pub(super) fn dashscope_thinking_modes(model: &ModelId) -> Vec<ModelThinkingMode> {
        let mut modes = Vec::new();
        match Self::dashscope_reasoning_profile(model.as_ref()) {
            Some(DashscopeReasoningProfile::Toggleable) => {
                modes.push(ModelThinkingMode {
                    is_default: false,
                    preset: None,
                    display_name: Some("Off".to_string()),
                    description: None,
                    thinking: Some(agena_domain::ThinkingRequest::Disabled),
                    request_override: agena_domain::ModelSpeedModeRequestOverride {
                        headers: BTreeMap::new(),
                        body_patch: BTreeMap::from([(
                            "enable_thinking".to_owned(),
                            serde_json::Value::Bool(false),
                        )]),
                    },
                    adapter_overrides: BTreeMap::new(),
                });

                modes.push(ModelThinkingMode {
                    is_default: false,
                    preset: Some("enabled".to_owned()),
                    display_name: Some("Think".to_string()),
                    description: Some("Enable DashScope reasoning output".to_string()),
                    thinking: None,
                    request_override: agena_domain::ModelSpeedModeRequestOverride {
                        headers: BTreeMap::new(),
                        body_patch: BTreeMap::from([(
                            "enable_thinking".to_owned(),
                            serde_json::Value::Bool(true),
                        )]),
                    },
                    adapter_overrides: BTreeMap::new(),
                });
            }
            Some(DashscopeReasoningProfile::AlwaysOn) => {
                modes.push(ModelThinkingMode {
                    is_default: false,
                    preset: Some("enabled".to_owned()),
                    display_name: Some("Think".to_string()),
                    description: Some("Use the model's built-in reasoning output".to_string()),
                    thinking: None,
                    request_override: agena_domain::ModelSpeedModeRequestOverride {
                        headers: BTreeMap::new(),
                        body_patch: BTreeMap::from([(
                            "enable_thinking".to_owned(),
                            serde_json::Value::Bool(true),
                        )]),
                    },
                    adapter_overrides: BTreeMap::new(),
                });
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
            .and_then(|auth| match auth.try_lock() {
                Ok(auth) => Some(auth),
                Err(error) => {
                    tracing::warn!(
                        operation = "read ChatGPT account id",
                        error = %error,
                        "ChatGPT account metadata is temporarily locked"
                    );
                    None
                }
            })
            .as_deref()
            .and_then(AuthData::account_id)
            .map(ToOwned::to_owned)
            .and_then(|value| utils::normalize_optional_text(Some(value)))
    }

    pub(super) fn chatgpt_account_is_fedramp(&self) -> bool {
        self.auth_data
            .as_ref()
            .and_then(|auth| match auth.try_lock() {
                Ok(auth) => Some(auth),
                Err(error) => {
                    tracing::warn!(
                        operation = "read ChatGPT FedRAMP account flag",
                        error = %error,
                        "ChatGPT account metadata is temporarily locked"
                    );
                    None
                }
            })
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

    pub(super) fn is_official_openai_endpoint(&self) -> bool {
        url::Url::parse(self.base_url.as_str())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host == "api.openai.com" || host.ends_with(".openai.com"))
    }
}
