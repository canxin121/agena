use std::time::Duration;

use agena_provider::OAuthCallback;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// Authentication operations owned by the concrete runtime.  Transports use
/// this port instead of inspecting resolved provider configuration or opening
/// the credential store themselves.
#[async_trait]
pub trait RuntimeAuthenticationService: Send + Sync {
    fn auth_providers(&self) -> Result<Vec<RuntimeAuthProvider>, RuntimeAuthenticationError>;
    fn auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<RuntimeAuthProvider, RuntimeAuthenticationError>;
    fn set_auth_api_key(
        &self,
        provider_id: &str,
        api_key: String,
    ) -> Result<(), RuntimeAuthenticationError>;
    async fn start_auth_browser(
        &self,
        provider_id: &str,
        kind: RuntimeAuthLoginKind,
        redirect_uri: String,
    ) -> Result<RuntimeAuthBrowserStart, RuntimeAuthenticationError>;
    /// Waits for the Runtime-owned local browser callback listener without
    /// exposing its TCP/parser implementation to a process consumer.
    fn wait_auth_browser_callback(
        &self,
        port: u16,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<OAuthCallback, RuntimeAuthenticationError>;
    async fn finish_auth_browser(
        &self,
        provider_id: &str,
        kind: RuntimeAuthLoginKind,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<(), RuntimeAuthenticationError>;
    async fn start_auth_device(
        &self,
        provider_id: &str,
        kind: RuntimeAuthLoginKind,
        enterprise_domain: Option<String>,
    ) -> Result<RuntimeAuthDeviceStart, RuntimeAuthenticationError>;
    async fn poll_auth_device(
        &self,
        provider_id: &str,
        kind: RuntimeAuthLoginKind,
        device_code: String,
        user_code: Option<String>,
        enterprise_domain: Option<String>,
    ) -> Result<bool, RuntimeAuthenticationError>;
    fn remove_auth_provider(&self, provider_id: &str) -> Result<(), RuntimeAuthenticationError>;
    async fn refresh_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<(), RuntimeAuthenticationError>;
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct RuntimeAuthenticationError {
    pub kind: RuntimeAuthenticationErrorKind,
    pub message: String,
}

impl RuntimeAuthenticationError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeAuthenticationErrorKind::BadRequest,
            message: message.into(),
        }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeAuthenticationErrorKind::NotFound,
            message: message.into(),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeAuthenticationErrorKind::Internal,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAuthenticationErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAuthCredentialIssuer {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
    GoogleAdc,
    SapAiCore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAuthLoginKind {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone)]
pub struct RuntimeAuthProvider {
    pub provider_id: String,
    pub credential_present: bool,
    pub credential_type: Option<RuntimeAuthCredentialType>,
    pub credential_issuer: Option<RuntimeAuthCredentialIssuer>,
    pub key_preview: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub account_id: Option<String>,
    pub enterprise_url: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub api_key_write_supported: bool,
    pub browser_login_kind: Option<RuntimeAuthLoginKind>,
    pub browser_login_instance_url: Option<String>,
    pub device_login_kind: Option<RuntimeAuthLoginKind>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAuthBrowserStart {
    pub instance_url: Option<String>,
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeAuthDeviceStart {
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}
