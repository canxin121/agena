use std::{collections::HashMap, path::Path, sync::Arc};

use agena_provider_bedrock_auth::{AwsCredentials as Credentials, static_credentials};
use tokio::sync::Mutex;

use crate::provider::{
    ManagedCredential, default_gitlab_ai_gateway_headers, default_gitlab_feature_flags,
};
use agena_provider::{
    AuthRefreshStrategy, AuthSecretSelector, CLINE_API_BASE_URL, GitlabProviderConfig,
    ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig, ProviderProtocolPathsConfig,
};

use super::{
    AuthData, CLINE_API_PROTOCOL_PATHS, ConfigEnvironment, ConfigError, GitlabRoutedBackend,
    HttpProviderAdapterConfig, ProviderApiAuthConfig, ProviderAuthConfig,
    parse_sap_ai_core_service_key,
};

pub(crate) fn api_auth<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<&'a ProviderApiAuthConfig, ConfigError> {
    match auth {
        ProviderAuthConfig::Api(api) => Ok(api),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "adapter requires api auth".to_owned(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpAdapterKind {
    OpenAi,
    Anthropic,
    Gemini,
}

pub(crate) fn http_adapter_default_user_agent(
    auth: &ProviderAuthConfig,
    adapter: HttpAdapterKind,
    default_model: &str,
) -> String {
    credential_user_agent(auth, default_model).unwrap_or_else(|| match adapter {
        HttpAdapterKind::OpenAi => crate::codex_user_agent(),
        HttpAdapterKind::Anthropic => crate::claude_code_api_user_agent(),
        HttpAdapterKind::Gemini => crate::gemini_cli_user_agent(default_model),
    })
}

pub(crate) fn credential_user_agent(
    auth: &ProviderAuthConfig,
    default_model: &str,
) -> Option<String> {
    let ProviderAuthConfig::Credential(config) = auth else {
        return None;
    };

    match config.issuer() {
        agena_provider::CredentialIssuer::OpenaiChatgpt => Some(crate::codex_user_agent()),
        agena_provider::CredentialIssuer::GoogleAdc => {
            Some(crate::gemini_cli_user_agent(default_model))
        }
        _ => None,
    }
}

pub(crate) fn resolve_http_adapter_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapter: HttpAdapterKind,
) -> Result<String, ConfigError> {
    let (base_url, protocol_paths) = provider_endpoint_root(auth, provider_id)?;
    let normalized = normalize_base_url(base_url)?;
    let protocol_path = http_adapter_protocol_path(protocol_paths, adapter);
    if protocol_path.is_empty() {
        Ok(normalized)
    } else {
        Ok(format!("{normalized}{protocol_path}"))
    }
}

pub(crate) fn provider_endpoint_root<'a>(
    auth: &'a ProviderAuthConfig,
    provider_id: &str,
) -> Result<(&'a str, &'a ProviderProtocolPathsConfig), ConfigError> {
    match auth {
        ProviderAuthConfig::Api(config) if config.custom_base_url().is_some() => Ok((
            config
                .custom_base_url()
                .expect("guard ensures api base_url exists"),
            config
                .custom_protocol_paths()
                .expect("custom api auth always has protocol_paths"),
        )),
        ProviderAuthConfig::Api(config) if config.is_cline_api() => {
            Ok((CLINE_API_BASE_URL, &CLINE_API_PROTOCOL_PATHS))
        }
        ProviderAuthConfig::Credential(config)
            if config.issuer().uses_http_endpoint() && config.base_url().is_some() =>
        {
            Ok((
                config
                    .base_url()
                    .expect("guard ensures credential base_url exists"),
                config
                    .protocol_paths()
                    .expect("guard ensures credential protocol paths exist"),
            ))
        }
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "provider auth does not define an api base_url".to_owned(),
        }),
    }
}

pub(crate) fn normalize_base_url(value: &str) -> Result<String, ConfigError> {
    let mut url = url::Url::parse(value).map_err(|err| {
        ConfigError::Validation(format!(
            "provider auth base_url `{value}` is invalid: {err}"
        ))
    })?;
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { path.as_str() });
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

pub(crate) fn http_adapter_protocol_path(
    protocol_paths: &ProviderProtocolPathsConfig,
    adapter: HttpAdapterKind,
) -> &str {
    match adapter {
        HttpAdapterKind::OpenAi => protocol_paths.openai.as_str(),
        HttpAdapterKind::Anthropic => protocol_paths.anthropic.as_str(),
        HttpAdapterKind::Gemini => protocol_paths.gemini.as_str(),
    }
}

pub(crate) fn looks_like_cline_models_url(models_url: Option<&str>) -> bool {
    models_url.is_some_and(|value| {
        value
            .trim()
            .to_ascii_lowercase()
            .contains("/ai/cline/recommended-models")
    })
}

pub(crate) fn looks_like_cline_provider_id(provider_id: &str) -> bool {
    let normalized = provider_id.trim().to_ascii_lowercase();
    normalized == "cline"
        || normalized.contains("cline_api")
        || normalized.contains("clineapi")
        || normalized.contains("cline-pass")
        || normalized.contains("cline_pass")
}

pub(crate) fn openai_adapter_capability_family(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    capability_family: Option<ProviderCapabilityFamilyConfig>,
    models_url: Option<&str>,
) -> Option<agena_provider::CapabilityFamily> {
    if let Some(family) = capability_family {
        return Some(family.into());
    }

    if looks_like_cline_provider_id(provider_id) || looks_like_cline_models_url(models_url) {
        return Some(agena_provider::CapabilityFamily::OpenAiCompatible);
    }

    let ProviderAuthConfig::Api(config) = auth else {
        return None;
    };
    if config.is_cline_api() {
        return Some(agena_provider::CapabilityFamily::OpenAiCompatible);
    }
    config
        .custom_base_url()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| value.starts_with("https://api.cline.bot"))
        .map(|_| agena_provider::CapabilityFamily::OpenAiCompatible)
}

pub(crate) fn openai_adapter_api_credential(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    client: reqwest::Client,
    capability_family: Option<ProviderCapabilityFamilyConfig>,
    env: &dyn ConfigEnvironment,
) -> Result<ResolvedManagedCredential, ConfigError> {
    match auth {
        ProviderAuthConfig::Api(_) => api_auth_managed_credential(
            provider_id,
            "api_key",
            auth,
            AuthSecretSelector::AccessOrApiKey,
            AuthRefreshStrategy::OpenAiOAuth,
            env,
            true,
        ),
        ProviderAuthConfig::Credential(config)
            if config.issuer() == agena_provider::CredentialIssuer::GoogleAdc =>
        {
            if !matches!(
                capability_family,
                Some(ProviderCapabilityFamilyConfig::Gemini)
            ) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "credential issuer `google_adc` only supports Vertex-style `openai_chat_completions` adapters"
                            .to_owned(),
                });
            }
            Ok(ResolvedManagedCredential {
                credential: ManagedCredential::google_adc(
                    format!("{provider_id} google adc"),
                    provider_id.to_owned(),
                ),
                auth_data: None,
            })
        }
        ProviderAuthConfig::Credential(config)
            if config.issuer() == agena_provider::CredentialIssuer::SapAiCore =>
        {
            Ok(ResolvedManagedCredential {
                credential: sap_ai_core_managed_credential(provider_id, client, config, env)?,
                auth_data: None,
            })
        }
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "OpenAI protocol adapter requires compatible API or credential auth"
                .to_owned(),
        }),
    }
}

pub(crate) struct ResolvedManagedCredential {
    pub(crate) credential: ManagedCredential,
    pub(crate) auth_data: Option<Arc<Mutex<AuthData>>>,
}

pub(crate) fn gitlab_instance_url(
    config: &agena_runtime_config::ProviderGitlabAuthConfig,
) -> String {
    config
        .instance_url
        .clone()
        .unwrap_or_else(|| "https://gitlab.com".to_owned())
}

pub(crate) fn gitlab_ai_gateway_url(
    config: &agena_runtime_config::ProviderGitlabAuthConfig,
) -> String {
    config
        .ai_gateway_url
        .clone()
        .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned())
}

pub(crate) fn gitlab_proxy_base_url(
    config: &agena_runtime_config::ProviderGitlabAuthConfig,
    backend: GitlabRoutedBackend,
) -> String {
    let gateway = gitlab_ai_gateway_url(config);
    match backend {
        GitlabRoutedBackend::OpenAiResponses | GitlabRoutedBackend::OpenAiChatCompletions => {
            format!("{gateway}/ai/v1/proxy/openai/v1")
        }
        GitlabRoutedBackend::Anthropic => format!("{gateway}/ai/v1/proxy/anthropic/v1"),
    }
}

pub(crate) fn gitlab_runtime_config(
    config: &agena_runtime_config::ProviderGitlabAuthConfig,
    default_model: &str,
) -> GitlabProviderConfig {
    GitlabProviderConfig {
        instance_url: gitlab_instance_url(config),
        ai_gateway_url: gitlab_ai_gateway_url(config),
        default_model: default_model.to_owned(),
        ai_gateway_headers: if config.ai_gateway_headers.is_empty() {
            default_gitlab_ai_gateway_headers()
        } else {
            to_hash_map(&config.ai_gateway_headers)
        },
        feature_flags: if config.feature_flags.is_empty() {
            default_gitlab_feature_flags()
        } else {
            to_hash_map(&config.feature_flags)
        },
    }
}

pub(crate) fn gitlab_auth_managed_credential(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    env: &dyn ConfigEnvironment,
    config_path: Option<&Path>,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let config = match auth {
        ProviderAuthConfig::Api(api) => api.gitlab(),
        _ => None,
    }
    .ok_or_else(|| ConfigError::InvalidProviderConfig {
        provider_id: provider_id.to_owned(),
        message: "adapter requires gitlab api auth".to_owned(),
    })?;

    if let Some(value) = config
        .access
        .api_key_source()
        .and_then(|source| source.inline())
        .and_then(normalize_text)
    {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::static_value(format!("{provider_id} api_key"), value),
            auth_data: None,
        });
    }

    if let Some(env_key) = config
        .access
        .api_key_source()
        .and_then(|source| source.env())
        .and_then(normalize_text)
    {
        if env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some()
        {
            return Ok(ResolvedManagedCredential {
                credential: ManagedCredential::environment(
                    format!("{provider_id} api_key"),
                    provider_id.to_owned(),
                    "api_key",
                    env_key,
                ),
                auth_data: None,
            });
        }

        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::environment(
                format!("{provider_id} api_key"),
                provider_id.to_owned(),
                "api_key",
                env_key,
            ),
            auth_data: None,
        });
    }

    if let Some(auth_data) = config.access.credential().cloned() {
        let auth_data = Arc::new(Mutex::new(auth_data));
        let credential = match config_path {
            Some(config_path) => ManagedCredential::auth_data_shared_with_store(
                format!("{provider_id} api_key"),
                provider_id.to_owned(),
                auth_data.clone(),
                AuthSecretSelector::AccessOrApiKey,
                AuthRefreshStrategy::GitlabOAuth {
                    instance_url: gitlab_instance_url(&config),
                },
                config_path.to_path_buf(),
            ),
            None => ManagedCredential::auth_data_shared(
                format!("{provider_id} api_key"),
                provider_id.to_owned(),
                auth_data.clone(),
                AuthSecretSelector::AccessOrApiKey,
                AuthRefreshStrategy::GitlabOAuth {
                    instance_url: gitlab_instance_url(&config),
                },
            ),
        };
        return Ok(ResolvedManagedCredential {
            credential,
            auth_data: Some(auth_data),
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field: "api_key",
    })
}

pub(crate) fn gitlab_credential_instance_url(config: &ProviderCredentialAuthConfig) -> String {
    config
        .gitlab()
        .and_then(|gitlab| gitlab.instance_url.clone())
        .unwrap_or_else(|| "https://gitlab.com".to_owned())
}

pub(crate) fn gitlab_credential_ai_gateway_url(config: &ProviderCredentialAuthConfig) -> String {
    config
        .gitlab()
        .and_then(|gitlab| gitlab.ai_gateway_url.clone())
        .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned())
}

pub(crate) fn gitlab_credential_proxy_base_url(
    config: &ProviderCredentialAuthConfig,
    backend: GitlabRoutedBackend,
) -> String {
    let gateway = gitlab_credential_ai_gateway_url(config);
    match backend {
        GitlabRoutedBackend::OpenAiResponses | GitlabRoutedBackend::OpenAiChatCompletions => {
            format!("{gateway}/ai/v1/proxy/openai/v1")
        }
        GitlabRoutedBackend::Anthropic => format!("{gateway}/ai/v1/proxy/anthropic/v1"),
    }
}

pub(crate) fn gitlab_credential_runtime_config(
    config: &ProviderCredentialAuthConfig,
    default_model: &str,
) -> GitlabProviderConfig {
    let gitlab = config
        .gitlab()
        .expect("gitlab credential runtime config requires gitlab credential auth");
    GitlabProviderConfig {
        instance_url: gitlab_credential_instance_url(config),
        ai_gateway_url: gitlab_credential_ai_gateway_url(config),
        default_model: default_model.to_owned(),
        ai_gateway_headers: if gitlab.ai_gateway_headers.is_empty() {
            default_gitlab_ai_gateway_headers()
        } else {
            to_hash_map(&gitlab.ai_gateway_headers)
        },
        feature_flags: if gitlab.feature_flags.is_empty() {
            default_gitlab_feature_flags()
        } else {
            to_hash_map(&gitlab.feature_flags)
        },
    }
}

pub(crate) fn api_auth_has_direct_source(
    api: &ProviderApiAuthConfig,
    env: &dyn ConfigEnvironment,
) -> bool {
    match (
        api.api_key().and_then(normalize_text),
        api.api_key_env().and_then(normalize_text),
    ) {
        (Some(_), _) => true,
        (None, Some(env_key)) => env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some(),
        (None, None) => false,
    }
}

pub(crate) fn required_api_auth_credential(
    provider_id: &str,
    field: &'static str,
    api: &ProviderApiAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    if let Some(value) = api.api_key().and_then(normalize_text) {
        return Ok(ManagedCredential::static_value(
            format!("{provider_id} {field}"),
            value,
        ));
    }

    let Some(env_key) = api.api_key_env().and_then(normalize_text) else {
        return Err(ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field,
        });
    };

    if env
        .var(env_key.as_str())
        .and_then(|value| normalize_text(&value))
        .is_none()
    {
        return Err(ConfigError::MissingEnvironmentVariable {
            provider_id: provider_id.to_owned(),
            field,
            env_key,
        });
    }

    Ok(ManagedCredential::environment(
        format!("{provider_id} {field}"),
        provider_id.to_owned(),
        field,
        env_key,
    ))
}

pub(crate) fn api_auth_managed_credential(
    provider_id: &str,
    field: &'static str,
    auth: &ProviderAuthConfig,
    _selector: AuthSecretSelector,
    _refresh: AuthRefreshStrategy,
    env: &dyn ConfigEnvironment,
    allow_deferred_env: bool,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let api = api_auth(auth, provider_id)?;
    if let Some(value) = api.api_key().and_then(normalize_text) {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::static_value(format!("{provider_id} {field}"), value),
            auth_data: None,
        });
    }

    let env_key = api.api_key_env().and_then(normalize_text);

    if let Some(env_key) = env_key.as_ref()
        && env
            .var(env_key.as_str())
            .and_then(|value| normalize_text(&value))
            .is_some()
    {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::environment(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                field,
                env_key.clone(),
            ),
            auth_data: None,
        });
    }

    if allow_deferred_env && let Some(env_key) = env_key {
        return Ok(ResolvedManagedCredential {
            credential: ManagedCredential::environment(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                field,
                env_key,
            ),
            auth_data: None,
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field,
    })
}

pub(crate) fn require_provider_auth_credential(
    provider_id: &str,
    field: &'static str,
    auth: &ProviderAuthConfig,
    selector: AuthSecretSelector,
    refresh: AuthRefreshStrategy,
    _env: &dyn ConfigEnvironment,
    config_path: Option<&Path>,
) -> Result<ResolvedManagedCredential, ConfigError> {
    let ProviderAuthConfig::Credential(config) = auth else {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("{field} must come from provider credential auth"),
        });
    };
    if let Some(auth_data) = config.credential().cloned() {
        if !auth_supports_selector(&auth_data, selector) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "configured inline credential does not satisfy `{field}` requirements"
                ),
            });
        }

        let auth_data = Arc::new(Mutex::new(auth_data));
        let credential = match config_path {
            Some(config_path) => ManagedCredential::auth_data_shared_with_store(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                auth_data.clone(),
                selector,
                refresh,
                config_path.to_path_buf(),
            ),
            None => ManagedCredential::auth_data_shared(
                format!("{provider_id} {field}"),
                provider_id.to_owned(),
                auth_data.clone(),
                selector,
                refresh,
            ),
        };
        return Ok(ResolvedManagedCredential {
            credential,
            auth_data: Some(auth_data),
        });
    }

    Err(ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field,
    })
}

pub(crate) fn sap_ai_core_managed_credential(
    provider_id: &str,
    client: reqwest::Client,
    config: &ProviderCredentialAuthConfig,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    let service_key_env =
        config
            .service_key_env()
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: "credential issuer `sap_ai_core` requires `service_key_env`".to_owned(),
            })?;
    let service_key_raw = env
        .var(service_key_env)
        .and_then(|value| normalize_text(&value))
        .ok_or_else(|| ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("sap-ai-core requires `{service_key_env}`"),
        })?;
    let service_key = parse_sap_ai_core_service_key(service_key_raw.as_str()).map_err(|err| {
        ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("failed to parse `{service_key_env}`: {err}"),
        }
    })?;
    Ok(ManagedCredential::sap_ai_core(
        format!("{provider_id} sap ai core token"),
        client.clone(),
        provider_id.to_owned(),
        service_key,
    ))
}

pub(crate) fn static_bedrock_credentials(
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    provider_id: &str,
) -> Result<Option<Credentials>, ConfigError> {
    match (
        access_key_id.and_then(|value| normalize_text(&value)),
        secret_access_key.and_then(|value| normalize_text(&value)),
    ) {
        (Some(access_key_id), Some(secret_access_key)) => Ok(Some(static_credentials(
            access_key_id,
            secret_access_key,
            session_token.and_then(|value| normalize_text(&value)),
        ))),
        (None, None) => Ok(None),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "access_key_id and secret_access_key must be provided together".to_owned(),
        }),
    }
}

pub(crate) fn runtime_adapter_provider_id(provider_id: &str, adapter_id: &str) -> String {
    if adapter_id == "default" {
        provider_id.to_owned()
    } else {
        format!("{provider_id}::{adapter_id}")
    }
}

pub(crate) fn copilot_base_url(
    auth_data: Option<&Arc<Mutex<AuthData>>>,
    _models_url: Option<&str>,
) -> Option<String> {
    let base_url = "https://api.githubcopilot.com";
    if base_url == "https://api.githubcopilot.com"
        && auth_data.and_then(current_enterprise_url).is_some()
    {
        None
    } else {
        Some(base_url.to_owned())
    }
}

pub(crate) fn current_enterprise_url(auth_data: &Arc<Mutex<AuthData>>) -> Option<String> {
    auth_data
        .try_lock()
        .ok()
        .as_deref()
        .and_then(AuthData::enterprise_url)
        .map(ToOwned::to_owned)
}

pub(crate) fn normalize_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(crate) fn auth_supports_selector(auth: &AuthData, selector: AuthSecretSelector) -> bool {
    match selector {
        AuthSecretSelector::AccessOrApiKey => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
            AuthData::OAuth { access, .. } => !access.trim().is_empty(),
        },
        AuthSecretSelector::RefreshOrAccess => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
            AuthData::OAuth {
                refresh, access, ..
            } => !refresh.trim().is_empty() || !access.trim().is_empty(),
        },
    }
}

pub(crate) fn http_adapter_extra_headers<T>(
    adapter: &HttpProviderAdapterConfig<T>,
    default_user_agent: Option<String>,
) -> HashMap<String, String> {
    let mut headers = to_hash_map(&adapter.extra_headers);
    if let Some(user_agent) = adapter.user_agent.as_deref().and_then(normalize_text) {
        set_user_agent_header(&mut headers, user_agent);
    } else if !has_user_agent_header(&headers)
        && let Some(user_agent) = default_user_agent.as_deref().and_then(normalize_text)
    {
        set_user_agent_header(&mut headers, user_agent);
    }
    headers
}

pub(crate) fn has_user_agent_header(headers: &HashMap<String, String>) -> bool {
    headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
}

pub(crate) fn set_user_agent_header(headers: &mut HashMap<String, String>, user_agent: String) {
    if let Some(existing) = headers
        .keys()
        .find(|key| key.eq_ignore_ascii_case(reqwest::header::USER_AGENT.as_str()))
        .cloned()
    {
        headers.remove(&existing);
    }
    headers.insert(reqwest::header::USER_AGENT.as_str().to_owned(), user_agent);
}

pub(crate) fn to_hash_map<K, V>(map: &std::collections::BTreeMap<K, V>) -> HashMap<K, V>
where
    K: Clone + Eq + std::hash::Hash,
    V: Clone,
{
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
