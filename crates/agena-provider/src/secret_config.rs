//! Provider credential-source values without concrete credential storage.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::AuthData;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
/// Source of a provider secret (inline or env).
pub enum ProviderSecretSourceConfig {
    Inline(String),
    Env(String),
}

impl ProviderSecretSourceConfig {
    pub fn inline(&self) -> Option<&str> {
        match self {
            Self::Inline(value) => Some(value.as_str()),
            Self::Env(_) => None,
        }
    }

    pub fn env(&self) -> Option<&str> {
        match self {
            Self::Inline(_) => None,
            Self::Env(value) => Some(value.as_str()),
        }
    }
}

impl fmt::Debug for ProviderSecretSourceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline(value) => f
                .debug_tuple("ProviderSecretSourceConfig::Inline")
                .field(&redacted(Some(value.as_str())))
                .finish(),
            Self::Env(value) => f
                .debug_tuple("ProviderSecretSourceConfig::Env")
                .field(value)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
/// GitLab API access configuration.
pub enum ProviderGitlabApiAccessConfig {
    ApiKey { source: ProviderSecretSourceConfig },
    Credential { credential: AuthData },
}

impl ProviderGitlabApiAccessConfig {
    pub fn api_key_source(&self) -> Option<&ProviderSecretSourceConfig> {
        match self {
            Self::ApiKey { source } => Some(source),
            Self::Credential { .. } => None,
        }
    }

    pub fn credential(&self) -> Option<&AuthData> {
        match self {
            Self::ApiKey { .. } => None,
            Self::Credential { credential } => Some(credential),
        }
    }
}

impl fmt::Debug for ProviderGitlabApiAccessConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { source } => f
                .debug_struct("ProviderGitlabApiAccessConfig::ApiKey")
                .field("source", source)
                .finish(),
            Self::Credential { credential } => f
                .debug_struct("ProviderGitlabApiAccessConfig::Credential")
                .field("credential", &credential_debug_kind(credential))
                .finish(),
        }
    }
}

fn redacted(value: Option<&str>) -> &'static str {
    match value {
        Some(value) if !value.is_empty() => "***redacted***",
        _ => "<none>",
    }
}

fn credential_debug_kind(value: &AuthData) -> &'static str {
    match value {
        AuthData::Api { .. } => "api",
        AuthData::OAuth { .. } => "oauth",
        AuthData::WellKnown { .. } => "well_known",
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderSecretSourceConfig;

    #[test]
    fn inline_secret_debug_output_is_redacted() {
        assert_eq!(
            format!(
                "{:?}",
                ProviderSecretSourceConfig::Inline("secret".to_owned())
            ),
            "ProviderSecretSourceConfig::Inline(\"***redacted***\")"
        );
    }
}
