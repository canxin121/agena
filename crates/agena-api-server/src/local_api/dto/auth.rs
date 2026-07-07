use super::*;
use agena::provider::auth::{DeviceCodeStart, OAuthAuthorizeStart};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialIssuerResource {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
    GoogleAdc,
    SapAiCore,
}

impl From<agena::provider::auth::CredentialIssuer> for AuthCredentialIssuerResource {
    fn from(value: agena::provider::auth::CredentialIssuer) -> Self {
        match value {
            agena::provider::auth::CredentialIssuer::OpenaiChatgpt => Self::OpenaiChatgpt,
            agena::provider::auth::CredentialIssuer::GithubCopilot => Self::GithubCopilot,
            agena::provider::auth::CredentialIssuer::Gitlab => Self::Gitlab,
            agena::provider::auth::CredentialIssuer::GoogleAdc => Self::GoogleAdc,
            agena::provider::auth::CredentialIssuer::SapAiCore => Self::SapAiCore,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthLoginKindResource {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthProviderResource {
    pub provider_id: String,
    pub configured: bool,
    pub credential_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<AuthCredentialType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_issuer: Option<AuthCredentialIssuerResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "is_false")]
    pub expired: bool,
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
    pub api_key_write_supported: bool,
    pub refresh_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_login_kind: Option<AuthLoginKindResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_login_instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_login_kind: Option<AuthLoginKindResource>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthApiKeyWriteRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthProviderRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

impl AuthProviderRequest {
    pub fn normalized_provider_id(&self, default: &str) -> String {
        self.provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default)
            .to_owned()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthRedirectRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthCodeExchangeRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthStatePollRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthUserCodeDevicePollRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthEnterpriseDeviceRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthEnterpriseDevicePollRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
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

impl AuthBrowserStartResource {
    pub fn from_start(
        provider_id: String,
        instance_url: Option<String>,
        start: OAuthAuthorizeStart,
    ) -> Self {
        Self {
            provider_id,
            instance_url,
            authorize_url: start.authorize_url,
            state: start.state,
            pkce_verifier: start.pkce_verifier,
        }
    }
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

impl AuthDeviceStartResource {
    pub fn from_start(
        provider_id: String,
        enterprise_domain: Option<String>,
        start: DeviceCodeStart,
    ) -> Self {
        Self {
            provider_id,
            enterprise_domain,
            verification_url: start.verification_url,
            user_code: start.user_code,
            device_code: start.device_code,
            interval_seconds: start.interval_seconds,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthLoginResultResource {
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AuthProviderResource>,
}
