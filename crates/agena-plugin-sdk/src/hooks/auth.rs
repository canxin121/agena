use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Purpose of an auth hook invocation.
pub enum AuthPurpose {
    ApiKey,
    Oauth,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of an auth hook.
pub struct AuthInput {
    pub provider: String,
    pub purpose: AuthPurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of credential produced by an auth hook.
pub enum AuthKind {
    ApiKey,
    BearerToken,
    OauthRefresh,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Output of an auth hook.
pub struct AuthOutput {
    pub kind: AuthKind,
    pub credential: serde_json::Value,
}
