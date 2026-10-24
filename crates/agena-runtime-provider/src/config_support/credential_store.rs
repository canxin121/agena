use std::sync::Arc;

use agena_provider::{AuthData, OpenAiResponsesBackendConfig, ProviderCredentialAuthConfig};
use agena_runtime_config::{
    ProviderApiAuthConfig, ProviderAuthConfig, ProviderGitlabAuthConfig, ResolvedProviderConfig,
};
use agena_secret_store::{FileSecretStore, SecretStore};

use crate::{ProviderError, provider::auth::AuthStore};

pub const PROVIDER_SECRET_NAMESPACE: &str = "agena.provider";

/// Managed API keys and OAuth records use the same private secret backend as
/// MCP and plugins. Saving or refreshing a credential never rewrites config.
#[derive(Clone)]
pub struct ProviderSecretCredentialStore {
    secrets: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for ProviderSecretCredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSecretCredentialStore")
            .finish_non_exhaustive()
    }
}

impl Default for ProviderSecretCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderSecretCredentialStore {
    pub fn new() -> Self {
        Self::with_secret_store(Arc::new(FileSecretStore::new(PROVIDER_SECRET_NAMESPACE)))
    }

    pub fn with_secret_store(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

impl AuthStore for ProviderSecretCredentialStore {
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, ProviderError> {
        self.secrets
            .get_secret(provider_id)
            .map_err(|error| ProviderError::Config(error.to_string()))?
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| {
                    ProviderError::Config("invalid managed provider credential".to_owned())
                })
            })
            .transpose()
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), ProviderError> {
        if matches!(auth, AuthData::OAuth { issuer: None, .. }) {
            return Err(ProviderError::Config(
                "managed OAuth credential requires an issuer".to_owned(),
            ));
        }
        let encoded = serde_json::to_string(&auth).map_err(|_| {
            ProviderError::Config("cannot encode managed provider credential".to_owned())
        })?;
        self.secrets
            .set_secret(provider_id, &encoded)
            .map_err(|error| ProviderError::Config(error.to_string()))
    }

    fn remove(&self, provider_id: &str) -> Result<(), ProviderError> {
        self.secrets
            .delete_secret(provider_id)
            .map_err(|error| ProviderError::Config(error.to_string()))
    }
}

/// Resolve a managed record without exposing the secret backend to adapters.
pub fn managed_provider_auth_data(provider_id: &str) -> Result<Option<AuthData>, ProviderError> {
    ProviderSecretCredentialStore::new().get(provider_id)
}

pub fn provider_auth_data(resolved: &ResolvedProviderConfig) -> Option<AuthData> {
    match &resolved.auth {
        ProviderAuthConfig::None => None,
        ProviderAuthConfig::Api(api) => secret_auth_data(api),
        ProviderAuthConfig::Credential(config) => config.credential().cloned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// OAuth target of a provider credential flow.
pub enum ProviderOAuthTarget {
    OpenAi,
    Gitlab { instance_url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Device-auth target of a provider credential flow.
pub enum ProviderDeviceAuthTarget {
    OpenAi,
    Copilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
/// Error resolving a provider auth target.
pub enum ProviderAuthTargetError {
    #[error(
        "provider auth target is ambiguous because multiple compatible providers are configured"
    )]
    AmbiguousProvider,
    #[error(
        "provider auth target is ambiguous because multiple GitLab configurations are available"
    )]
    AmbiguousGitlab,
}

pub fn provider_supports_openai_oauth(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::OpenaiChatgpt { .. })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiResponses(config)
                if matches!(
                    config.options.backend,
                    OpenAiResponsesBackendConfig::ChatgptCodex
                )
        )
    })
}

pub fn provider_has_gitlab_adapter(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Api(ref api)
            if api.gitlab().is_some()
    ) || resolved.adapters.values().any(|adapter| {
        matches!(
            adapter.definition,
            super::ProviderAdapterDefinition::Gitlab(_)
        )
    }) || matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::Gitlab { .. })
    )
}

pub fn provider_gitlab_instance_url(resolved: &ResolvedProviderConfig) -> Option<String> {
    if let ProviderAuthConfig::Api(api) = &resolved.auth
        && let Some(config) = api.gitlab()
    {
        return Some(
            config
                .instance_url
                .clone()
                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
        );
    }
    if let ProviderAuthConfig::Credential(config) = &resolved.auth
        && let Some(gitlab) = config.gitlab()
    {
        return Some(
            gitlab
                .instance_url
                .clone()
                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
        );
    }

    let mut urls = resolved
        .adapters
        .values()
        .filter_map(|adapter| {
            if let super::ProviderAdapterDefinition::Gitlab(options) = &adapter.definition {
                options.instance_url.clone()
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    match urls.as_slice() {
        [] => None,
        [instance_url] => Some(instance_url.clone()),
        _ => None,
    }
}

pub fn provider_supports_copilot_device(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::GithubCopilot { .. })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiResponses(config)
                if matches!(
                    config.options.backend,
                    OpenAiResponsesBackendConfig::Api
                )
        ) || matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiChatCompletions(_)
        )
    })
}

pub fn provider_supports_api_key_write(resolved: &ResolvedProviderConfig) -> bool {
    match resolved.auth {
        ProviderAuthConfig::Api(_) => {
            !provider_supports_openai_oauth(resolved) && !provider_supports_copilot_device(resolved)
        }
        ProviderAuthConfig::Credential(_) => false,
        ProviderAuthConfig::None => false,
    }
}

pub fn resolve_provider_oauth_target(
    resolved: &ResolvedProviderConfig,
) -> Result<Option<ProviderOAuthTarget>, ProviderAuthTargetError> {
    let openai = provider_supports_openai_oauth(resolved);
    let gitlab = provider_has_gitlab_adapter(resolved);
    match (openai, gitlab) {
        (true, false) => Ok(Some(ProviderOAuthTarget::OpenAi)),
        (false, true) => provider_gitlab_instance_url(resolved)
            .map(|instance_url| ProviderOAuthTarget::Gitlab { instance_url })
            .map(Some)
            .ok_or(ProviderAuthTargetError::AmbiguousGitlab),
        (false, false) => Ok(None),
        _ => Err(ProviderAuthTargetError::AmbiguousProvider),
    }
}

pub fn resolve_provider_device_auth_target(
    resolved: &ResolvedProviderConfig,
) -> Result<Option<ProviderDeviceAuthTarget>, ProviderAuthTargetError> {
    let openai = provider_supports_openai_oauth(resolved);
    let copilot = provider_supports_copilot_device(resolved);
    match (openai, copilot) {
        (true, false) => Ok(Some(ProviderDeviceAuthTarget::OpenAi)),
        (false, true) => Ok(Some(ProviderDeviceAuthTarget::Copilot)),
        (false, false) => Ok(None),
        _ => Err(ProviderAuthTargetError::AmbiguousProvider),
    }
}

fn secret_auth_data(secret: &ProviderApiAuthConfig) -> Option<AuthData> {
    if let Some(key) = normalize_text(secret.api_key()) {
        return Some(AuthData::Api { key });
    }
    if let Some(gitlab) = secret.gitlab() {
        return gitlab_auth_data(&gitlab);
    }
    None
}

fn gitlab_auth_data(secret: &ProviderGitlabAuthConfig) -> Option<AuthData> {
    if let Some(credential) = secret.access.credential().cloned() {
        return Some(credential);
    }
    if let Some(key) = secret
        .access
        .api_key_source()
        .and_then(|source| normalize_text(source.inline()))
    {
        return Some(AuthData::Api { key });
    }
    None
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
