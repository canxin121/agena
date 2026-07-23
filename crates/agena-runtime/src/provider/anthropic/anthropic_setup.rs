use super::{
    AnthropicAdapter, AnthropicAdapterOptions, AnthropicProfile, AppError, AuthData,
    DEFAULT_ANTHROPIC_BETA_HEADER, DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER, DEFAULT_COPILOT_BASE_URL,
    HashMap, ManagedCredential, ModelId, normalize_domain, utils,
};

impl AnthropicAdapter {
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
            extra_headers.insert("x-app".to_owned(), "cli".to_owned());
        }
        extra_headers.insert(
            reqwest::header::USER_AGENT.as_str().to_owned(),
            agena_runtime::claude_code_api_user_agent(),
        );
        if options.profile == AnthropicProfile::GithubCopilot {
            extra_headers.insert(
                "anthropic-beta".to_owned(),
                DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER.to_owned(),
            );
        }
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
        for (key, value) in options.extra_headers {
            utils::insert_header_case_insensitive(&mut extra_headers, key, value);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_with_beta_override(
        profile: AnthropicProfile,
        extra_beta_header: Option<&str>,
    ) -> AnthropicAdapter {
        AnthropicAdapter::new_managed_with_options(
            "anthropic",
            reqwest::Client::new(),
            ManagedCredential::static_value("anthropic api key", "test-key"),
            if profile == AnthropicProfile::GithubCopilot {
                DEFAULT_COPILOT_BASE_URL
            } else {
                "https://api.anthropic.com/v1"
            },
            "claude-opus-4-8",
            AnthropicAdapterOptions {
                profile,
                extra_beta_header: extra_beta_header.map(ToOwned::to_owned),
                override_beta_header: extra_beta_header.is_some(),
                ..AnthropicAdapterOptions::default()
            },
        )
    }

    #[test]
    fn first_party_defaults_match_current_claude_code_headers() {
        let adapter = AnthropicAdapter::new_managed_with_options(
            "anthropic",
            reqwest::Client::new(),
            ManagedCredential::static_value("anthropic api key", "test-key"),
            "https://api.anthropic.com/v1",
            "claude-opus-4-8",
            AnthropicAdapterOptions::default(),
        );

        assert_eq!(
            adapter
                .extra_headers
                .get("anthropic-beta")
                .map(String::as_str),
            Some(DEFAULT_ANTHROPIC_BETA_HEADER)
        );
        assert_eq!(
            adapter.extra_headers.get("x-app").map(String::as_str),
            Some("cli")
        );
        let user_agent = adapter
            .extra_headers
            .get(reqwest::header::USER_AGENT.as_str())
            .expect("first-party Claude user agent");
        assert!(user_agent.starts_with("claude-cli/"));
        assert!(user_agent.ends_with(" (external, cli)"));
        assert!(!user_agent.to_ascii_lowercase().contains("agena"));
    }

    #[test]
    fn configured_beta_header_overrides_profile_defaults() {
        for profile in [AnthropicProfile::Standard, AnthropicProfile::GithubCopilot] {
            let adapter = adapter_with_beta_override(profile, Some("custom-beta-2026-07-14"));
            assert_eq!(
                adapter
                    .extra_headers
                    .get("anthropic-beta")
                    .map(String::as_str),
                Some("custom-beta-2026-07-14")
            );
        }
    }

    #[test]
    fn extra_headers_override_defaults_case_insensitively() {
        let adapter = AnthropicAdapter::new_managed_with_options(
            "anthropic",
            reqwest::Client::new(),
            ManagedCredential::static_value("anthropic api key", "test-key"),
            "https://api.anthropic.com/v1",
            "claude-opus-4-8",
            AnthropicAdapterOptions {
                extra_headers: HashMap::from([(
                    "Anthropic-Beta".to_owned(),
                    "header-override-2026-07-14".to_owned(),
                )]),
                ..AnthropicAdapterOptions::default()
            },
        );

        assert_eq!(
            adapter
                .extra_headers
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case("anthropic-beta"))
                .count(),
            1
        );
        assert_eq!(
            adapter
                .extra_headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("anthropic-beta"))
                .map(|(_, value)| value.as_str()),
            Some("header-override-2026-07-14")
        );
    }
}
