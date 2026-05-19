use super::*;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthProviderResource {
    pub provider_id: String,
    pub configured: bool,
    pub credential_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<AuthCredentialType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthApiKeyWriteRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiBrowserFinishRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserFinishRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthAtomGitBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthAtomGitBrowserPollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthOpenAiDeviceStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiDevicePollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthCopilotDeviceStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthCopilotDevicePollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub device_code: String,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthBrowserStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthDeviceStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_domain: Option<String>,
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthLoginResultResource {
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AuthProviderResource>,
}
