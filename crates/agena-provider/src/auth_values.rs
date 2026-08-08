use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const OAUTH_EXPIRY_BUFFER_MS: i64 = 5 * 60 * 1_000;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of credential issuer / login flow for a provider.
pub enum CredentialIssuer {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
    GoogleAdc,
    SapAiCore,
}

impl CredentialIssuer {
    pub fn uses_http_endpoint(self) -> bool {
        matches!(self, Self::GoogleAdc | Self::SapAiCore)
    }

    pub fn requires_service_key_env(self) -> bool {
        matches!(self, Self::SapAiCore)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// User info returned by an OAuth login.
pub struct OAuthUserInfo {
    pub id: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
/// Credential material for a provider (API key or OAuth token).
pub enum AuthData {
    Api {
        key: String,
    },
    #[serde(rename = "oauth")]
    OAuth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issuer: Option<CredentialIssuer>,
        refresh: String,
        access: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id_token: Option<String>,
        expires_at_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        chatgpt_account_is_fedramp: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enterprise_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<OAuthUserInfo>,
    },
    WellKnown {
        key: String,
        token: String,
    },
}

impl AuthData {
    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::Api { key } | Self::WellKnown { key, .. } => Some(key.as_str()),
            Self::OAuth { .. } => None,
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::OAuth {
                account_id, user, ..
            } => account_id
                .as_deref()
                .or_else(|| user.as_ref().map(|user| user.id.as_str())),
            _ => None,
        }
    }

    pub fn issuer(&self) -> Option<CredentialIssuer> {
        match self {
            Self::OAuth { issuer, .. } => *issuer,
            _ => None,
        }
    }

    pub fn for_issuer(self, issuer: CredentialIssuer) -> Self {
        match self {
            Self::OAuth {
                refresh,
                access,
                id_token,
                expires_at_ms,
                account_id,
                chatgpt_account_is_fedramp,
                enterprise_url,
                user,
                ..
            } => Self::OAuth {
                issuer: Some(issuer),
                refresh,
                access,
                id_token,
                expires_at_ms,
                account_id,
                chatgpt_account_is_fedramp,
                enterprise_url,
                user,
            },
            other => other,
        }
    }

    pub fn enterprise_url(&self) -> Option<&str> {
        match self {
            Self::OAuth { enterprise_url, .. } => enterprise_url.as_deref(),
            _ => None,
        }
    }

    pub fn chatgpt_account_is_fedramp(&self) -> bool {
        matches!(
            self,
            Self::OAuth {
                chatgpt_account_is_fedramp: true,
                ..
            }
        )
    }

    pub fn is_oauth_expired(&self, now: DateTime<Utc>) -> bool {
        match self {
            Self::OAuth { expires_at_ms, .. } => {
                *expires_at_ms > 0
                    && *expires_at_ms <= now.timestamp_millis() + OAUTH_EXPIRY_BUFFER_MS
            }
            _ => false,
        }
    }

    pub fn user(&self) -> Option<&OAuthUserInfo> {
        match self {
            Self::OAuth { user, .. } => user.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// OAuth token response with refresh and access tokens.
pub struct OAuthTokenResponse {
    pub refresh: String,
    pub access: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    pub expires_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub chatgpt_account_is_fedramp: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<OAuthUserInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// GitHub Copilot deployment (GitHub.com or Enterprise).
pub enum CopilotDeployment {
    GitHubCom,
    Enterprise { domain: String },
}
