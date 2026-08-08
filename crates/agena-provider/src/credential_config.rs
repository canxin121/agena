//! Provider credential configuration values.

use std::{collections::BTreeMap, fmt};

use serde::Serialize;

use crate::{AuthData, CredentialIssuer, ProviderProtocolPathsConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
/// Inline credential authentication configuration.
pub struct ProviderInlineCredentialAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// HTTP credential authentication configuration.
pub struct ProviderHttpCredentialAuthConfig {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// SAP AI Core credential authentication configuration.
pub struct ProviderSapAiCoreCredentialAuthConfig {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
    pub service_key_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
/// GitLab credential authentication configuration.
pub struct ProviderGitlabCredentialAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, bool>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "issuer", rename_all = "snake_case")]
/// Credential-based authentication configuration of a provider.
pub enum ProviderCredentialAuthConfig {
    OpenaiChatgpt {
        #[serde(flatten)]
        config: ProviderInlineCredentialAuthConfig,
    },
    GithubCopilot {
        #[serde(flatten)]
        config: ProviderInlineCredentialAuthConfig,
    },
    Gitlab {
        #[serde(flatten)]
        config: ProviderGitlabCredentialAuthConfig,
    },
    GoogleAdc {
        #[serde(flatten)]
        config: ProviderHttpCredentialAuthConfig,
    },
    SapAiCore {
        #[serde(flatten)]
        config: ProviderSapAiCoreCredentialAuthConfig,
    },
}

impl ProviderCredentialAuthConfig {
    pub fn issuer(&self) -> CredentialIssuer {
        match self {
            Self::OpenaiChatgpt { .. } => CredentialIssuer::OpenaiChatgpt,
            Self::GithubCopilot { .. } => CredentialIssuer::GithubCopilot,
            Self::Gitlab { .. } => CredentialIssuer::Gitlab,
            Self::GoogleAdc { .. } => CredentialIssuer::GoogleAdc,
            Self::SapAiCore { .. } => CredentialIssuer::SapAiCore,
        }
    }

    pub fn credential(&self) -> Option<&AuthData> {
        match self {
            Self::OpenaiChatgpt { config } | Self::GithubCopilot { config } => {
                config.credential.as_ref()
            }
            Self::Gitlab { config } => config.credential.as_ref(),
            Self::GoogleAdc { .. } | Self::SapAiCore { .. } => None,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::GoogleAdc { config } => Some(config.base_url.as_str()),
            Self::SapAiCore { config } => Some(config.base_url.as_str()),
            _ => None,
        }
    }

    pub fn protocol_paths(&self) -> Option<&ProviderProtocolPathsConfig> {
        match self {
            Self::GoogleAdc { config } => Some(&config.protocol_paths),
            Self::SapAiCore { config } => Some(&config.protocol_paths),
            _ => None,
        }
    }

    pub fn service_key_env(&self) -> Option<&str> {
        match self {
            Self::SapAiCore { config } => Some(config.service_key_env.as_str()),
            _ => None,
        }
    }

    pub fn gitlab(&self) -> Option<&ProviderGitlabCredentialAuthConfig> {
        match self {
            Self::Gitlab { config } => Some(config),
            _ => None,
        }
    }

    pub fn inline(&self) -> Option<&ProviderInlineCredentialAuthConfig> {
        match self {
            Self::OpenaiChatgpt { config } | Self::GithubCopilot { config } => Some(config),
            _ => None,
        }
    }
}

impl fmt::Debug for ProviderCredentialAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ProviderCredentialAuthConfig");
        debug.field("issuer", &self.issuer());
        debug.field("credential", &self.credential().map(credential_debug_kind));
        if let Some(base_url) = self.base_url() {
            debug.field("base_url", &base_url);
        }
        if let Some(protocol_paths) = self.protocol_paths() {
            debug.field("protocol_paths", protocol_paths);
        }
        if let Some(service_key_env) = self.service_key_env() {
            debug.field("service_key_env", &service_key_env);
        }
        if let Some(gitlab) = self.gitlab() {
            debug
                .field("instance_url", &gitlab.instance_url)
                .field("ai_gateway_url", &gitlab.ai_gateway_url)
                .field("ai_gateway_headers", &gitlab.ai_gateway_headers)
                .field("feature_flags", &gitlab.feature_flags);
        }
        debug.finish()
    }
}

fn credential_debug_kind(value: &AuthData) -> &'static str {
    match value {
        AuthData::Api { .. } => "api",
        AuthData::OAuth { .. } => "oauth",
        AuthData::WellKnown { .. } => "well_known",
    }
}

fn is_default<T>(value: &T) -> bool
where
    T: Default + PartialEq,
{
    value == &T::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_auth_exposes_google_adc_provider_contract() {
        let protocol_paths = ProviderProtocolPathsConfig::default();
        let config = ProviderCredentialAuthConfig::GoogleAdc {
            config: ProviderHttpCredentialAuthConfig {
                base_url: "https://oauth2.googleapis.com".to_owned(),
                protocol_paths: protocol_paths.clone(),
            },
        };

        assert_eq!(config.issuer(), CredentialIssuer::GoogleAdc);
        assert_eq!(config.base_url(), Some("https://oauth2.googleapis.com"));
        assert_eq!(config.protocol_paths(), Some(&protocol_paths));
        assert!(config.credential().is_none());
        assert!(config.gitlab().is_none());
    }
}
