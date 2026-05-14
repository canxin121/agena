use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialIssuer {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthData {
    Api {
        key: String,
    },
    #[serde(rename = "oauth", alias = "o_auth")]
    OAuth {
        #[serde(default, alias = "provider", skip_serializing_if = "Option::is_none")]
        issuer: Option<CredentialIssuer>,
        #[serde(alias = "refreshToken")]
        refresh: String,
        #[serde(alias = "accessToken")]
        access: String,
        #[serde(alias = "expiresAtMs")]
        expires_at_ms: i64,
        #[serde(default, alias = "accountId", skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(
            default,
            alias = "enterpriseUrl",
            skip_serializing_if = "Option::is_none"
        )]
        enterprise_url: Option<String>,
    },
    WellKnown {
        key: String,
        token: String,
    },
}

impl AuthData {
    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::Api { key } => Some(key.as_str()),
            Self::WellKnown { key, .. } => Some(key.as_str()),
            Self::OAuth { .. } => None,
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::OAuth { account_id, .. } => account_id.as_deref(),
            _ => None,
        }
    }

    pub fn issuer(&self) -> Option<CredentialIssuer> {
        match self {
            Self::OAuth { issuer, .. } => *issuer,
            _ => None,
        }
    }

    pub fn with_issuer(self, issuer: CredentialIssuer) -> Self {
        match self {
            Self::OAuth {
                refresh,
                access,
                expires_at_ms,
                account_id,
                enterprise_url,
                ..
            } => Self::OAuth {
                issuer: Some(issuer),
                refresh,
                access,
                expires_at_ms,
                account_id,
                enterprise_url,
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

    pub fn is_oauth_expired(&self, now: DateTime<Utc>) -> bool {
        match self {
            Self::OAuth { expires_at_ms, .. } => {
                *expires_at_ms > 0 && *expires_at_ms <= now.timestamp_millis()
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthTokenResponse {
    pub refresh: String,
    pub access: String,
    pub expires_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthAuthorizeStart {
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCodeStart {
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CopilotDeployment {
    GitHubCom,
    Enterprise { domain: String },
}

#[cfg(test)]
mod tests {
    use super::AuthData;

    #[test]
    fn oauth_deserializes_camel_case_aliases() {
        let payload = serde_json::json!({
            "type": "oauth",
            "provider": "github_copilot",
            "refreshToken": "r1",
            "accessToken": "a1",
            "expiresAtMs": 123,
            "accountId": "acct",
            "enterpriseUrl": "github.example.com"
        });

        let parsed: AuthData = serde_json::from_value(payload).expect("should deserialize oauth");
        assert_eq!(
            parsed,
            AuthData::OAuth {
                issuer: Some(super::CredentialIssuer::GithubCopilot),
                refresh: "r1".to_owned(),
                access: "a1".to_owned(),
                expires_at_ms: 123,
                account_id: Some("acct".to_owned()),
                enterprise_url: Some("github.example.com".to_owned()),
            }
        );
    }

    #[test]
    fn oauth_serializes_snake_case_fields() {
        let auth = AuthData::OAuth {
            issuer: Some(super::CredentialIssuer::OpenaiChatgpt),
            refresh: "r1".to_owned(),
            access: "a1".to_owned(),
            expires_at_ms: 123,
            account_id: Some("acct".to_owned()),
            enterprise_url: Some("github.example.com".to_owned()),
        };

        let value = serde_json::to_value(auth).expect("should serialize oauth");
        let object = value.as_object().expect("oauth should serialize as object");

        assert!(object.contains_key("refresh"));
        assert!(object.contains_key("access"));
        assert!(object.contains_key("expires_at_ms"));
        assert!(object.contains_key("issuer"));
        assert!(object.contains_key("account_id"));
        assert!(object.contains_key("enterprise_url"));
        assert!(!object.contains_key("refreshToken"));
        assert!(!object.contains_key("accessToken"));
        assert!(!object.contains_key("expiresAtMs"));
        assert!(!object.contains_key("accountId"));
        assert!(!object.contains_key("enterpriseUrl"));
    }
}
