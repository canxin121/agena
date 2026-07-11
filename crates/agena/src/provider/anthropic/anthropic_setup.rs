use super::{
    AnthropicAdapter, AnthropicAdapterOptions, AnthropicProfile, AppError, AuthData,
    DEFAULT_ANTHROPIC_BETA_HEADER, DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER, DEFAULT_COPILOT_BASE_URL,
    HashMap, ManagedCredential, ModelId, PROVIDER_ID, normalize_domain, utils,
};

impl AnthropicAdapter {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_managed(
            client,
            ManagedCredential::static_value("anthropic api key", api_key.into()),
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
        Self::new_managed_with_id(PROVIDER_ID, client, api_key, base_url, default_model)
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
            AnthropicAdapterOptions::default(),
        )
    }

    pub fn new_managed_with_options(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        options: AnthropicAdapterOptions,
    ) -> Self {
        let id = id.into();
        let base_url = utils::normalize_base_url(base_url.into().as_str());
        let mut extra_headers = HashMap::new();
        if Self::is_bundled_base_url(base_url.as_str()) {
            extra_headers.insert(
                "anthropic-beta".to_owned(),
                DEFAULT_ANTHROPIC_BETA_HEADER.to_owned(),
            );
        }
        extra_headers.insert(
            reqwest::header::USER_AGENT.as_str().to_owned(),
            crate::provider::claude_code_api_user_agent(),
        );
        if options
            .extra_headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        {
            extra_headers
                .retain(|key, _| !key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()));
        }
        if options.override_beta_header {
            match options
                .extra_beta_header
                .and_then(|header| utils::normalize_optional_text(Some(header)))
            {
                Some(value) => {
                    extra_headers.insert("anthropic-beta".to_owned(), value);
                }
                None => {
                    extra_headers.remove("anthropic-beta");
                }
            }
        }
        if options.profile == AnthropicProfile::GithubCopilot {
            extra_headers.insert(
                "anthropic-beta".to_owned(),
                DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER.to_owned(),
            );
        }
        extra_headers.extend(options.extra_headers);

        Self {
            id,
            client,
            api_key,
            base_url,
            default_model: ModelId::new(default_model),
            auth_data: options.auth_data,
            auth_header: if options.profile == AnthropicProfile::GithubCopilot {
                "authorization".to_owned()
            } else {
                options.auth_header
            },
            auth_scheme: if options.profile == AnthropicProfile::GithubCopilot {
                Some("Bearer".to_owned())
            } else {
                options.auth_scheme
            },
            models_url: options
                .models_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            messages_url: options
                .messages_url
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            profile: options.profile,
            extra_headers,
            eager_input_streaming_override: options.eager_input_streaming_override,
        }
    }

    pub(crate) fn configured_public_copilot_base_url(&self) -> bool {
        self.base_url.trim_end_matches('/') == DEFAULT_COPILOT_BASE_URL
    }

    pub(crate) fn resolved_base_url(&self) -> Result<String, AppError> {
        if self.profile != AnthropicProfile::GithubCopilot
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

    pub(crate) fn prompt_cache_base_url(&self) -> String {
        self.resolved_base_url()
            .unwrap_or_else(|_| self.base_url.clone())
    }

    pub(crate) fn models_endpoint(&self) -> Result<String, AppError> {
        Ok(self.models_url.clone().unwrap_or_else(|| {
            format!(
                "{}/models",
                self.prompt_cache_base_url().trim_end_matches('/')
            )
        }))
    }

    pub(crate) fn messages_endpoint(&self) -> Result<String, AppError> {
        if let Some(endpoint) = self.messages_url.clone() {
            return Ok(endpoint);
        }

        let base = self.resolved_base_url()?;
        Ok(match self.profile {
            AnthropicProfile::Standard => format!("{}/messages", base.trim_end_matches('/')),
            AnthropicProfile::GithubCopilot => {
                format!("{}/v1/messages", base.trim_end_matches('/'))
            }
        })
    }
}
