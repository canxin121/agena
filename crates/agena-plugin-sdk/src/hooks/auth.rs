use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthPurpose {
    ApiKey,
    Oauth,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInput {
    pub provider: String,
    pub purpose: AuthPurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    ApiKey,
    BearerToken,
    OauthRefresh,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthOutput {
    pub kind: AuthKind,
    pub credential: serde_json::Value,
}
