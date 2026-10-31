//! Draft authentication boundary over the shared provider OAuth protocol client.
//!
//! Unlike [`RuntimeAuthenticationService`], this port never names a persisted
//! provider id or credential store. Application updates only the returned
//! draft; AuthManager separately owns writes to the formal credential store.
//! The shared protocol functions do not receive either persistence capability.

use agena_provider::OAuthTokenResponse;
use agena_runtime_provider::{ProviderError, provider::auth as protocol};
use async_trait::async_trait;

pub use agena_provider::{
    DeviceCodeStart as RuntimeDraftAuthDeviceStart,
    OAuthAuthorizeStart as RuntimeDraftAuthBrowserStart,
};

use crate::RuntimeAuthenticationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a draft authentication flow.
pub enum RuntimeDraftAuthKind {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone)]
/// Token of a draft authentication flow.
pub struct RuntimeDraftAuthToken {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at_ms: i64,
    pub account_id: Option<String>,
}

#[async_trait]
/// Service for draft provider authentication.
pub trait RuntimeDraftAuthenticationService: Send + Sync {
    fn start_draft_auth_browser(
        &self,
        kind: RuntimeDraftAuthKind,
        instance_url: Option<String>,
        redirect_uri: String,
    ) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError>;

    async fn finish_draft_auth_browser(
        &self,
        kind: RuntimeDraftAuthKind,
        instance_url: Option<String>,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError>;

    async fn start_draft_auth_device(
        &self,
        kind: RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
    ) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError>;

    async fn poll_draft_auth_device(
        &self,
        kind: RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
        device_code: String,
        user_code: Option<String>,
    ) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError>;
}

impl From<OAuthTokenResponse> for RuntimeDraftAuthToken {
    fn from(token: OAuthTokenResponse) -> Self {
        Self {
            refresh_token: token.refresh,
            access_token: token.access,
            expires_at_ms: token.expires_at_ms,
            account_id: token.account_id,
        }
    }
}

// The draft boundary owns public error classification and redaction. Protocol
// failures retain operation/status context without exposing raw token bodies.
fn draft_protocol_error(operation: &str, error: ProviderError) -> RuntimeAuthenticationError {
    match error {
        ProviderError::Config(detail) => RuntimeAuthenticationError::bad_request(
            agena_failure::diagnostic::scrubbed_preserve(&detail, 1_000),
        ),
        ProviderError::HttpStatus { status, body, .. } => {
            RuntimeAuthenticationError::internal(format!(
                "{operation} failed with status {status}: {}",
                draft_auth_public_error_detail(body.as_bytes())
            ))
        }
        error => RuntimeAuthenticationError::internal(format!(
            "{operation}: {}",
            agena_failure::diagnostic::format_error_chain(&error)
        )),
    }
}

fn draft_auth_public_error_detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    // Protocol errors may prepend diagnostic context to an otherwise opaque
    // JSON response. Keep structured payloads behind the same allowlist even
    // with that prefix; the caller supplies public operation/status context.
    let value = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .or_else(|| {
            let start = text.find(['{', '['])?;
            serde_json::from_str::<serde_json::Value>(&text[start..]).ok()
        });
    if let Some(value) = value {
        for field in ["error_description", "message", "detail", "error"] {
            if let Some(detail) = value.get(field).and_then(serde_json::Value::as_str) {
                let detail = agena_failure::diagnostic::scrubbed_preserve(detail, 1_000);
                if !detail.is_empty() {
                    return detail;
                }
            }
        }
        return "provider returned an error response without a public diagnostic".to_owned();
    }

    let detail = agena_failure::diagnostic::scrubbed_preserve(&text, 1_000);
    if detail.is_empty() {
        "provider returned an error response without a public diagnostic".to_owned()
    } else {
        detail
    }
}

pub async fn start_copilot_draft_auth_device(
    domain: &str,
) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError> {
    protocol::start_copilot_device_code(domain)
        .await
        .map_err(|error| draft_protocol_error("github copilot device oauth start", error))
}

pub async fn poll_copilot_draft_auth_device(
    domain: &str,
    device_code: &str,
) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError> {
    protocol::poll_copilot_device_code(domain, device_code)
        .await
        .map(|token| token.map(Into::into))
        .map_err(|error| draft_protocol_error("github copilot device oauth poll", error))
}

pub async fn start_openai_draft_auth_device(
    user_agent: &str,
) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError> {
    protocol::start_openai_headless_device_code(user_agent)
        .await
        .map_err(|error| draft_protocol_error("openai device oauth start", error))
}

pub async fn poll_openai_draft_auth_device(
    user_agent: &str,
    device_auth_id: &str,
    user_code: &str,
) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError> {
    protocol::poll_openai_headless_device_code(user_agent, device_auth_id, user_code)
        .await
        .map(|token| token.map(Into::into))
        .map_err(|error| draft_protocol_error("openai device oauth poll", error))
}

pub fn start_openai_draft_auth_browser(
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError> {
    protocol::start_openai_browser_oauth(redirect_uri)
        .map_err(|error| draft_protocol_error("openai oauth start", error))
}

pub async fn finish_openai_draft_auth_browser(
    user_agent: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError> {
    protocol::exchange_openai_oauth_code(
        user_agent,
        code,
        pkce_verifier,
        redirect_uri,
        protocol::OpenAiRefreshTokenRequirement::Present,
    )
    .await
    .map(Into::into)
    .map_err(|error| draft_protocol_error("openai oauth token exchange", error))
}

pub fn start_gitlab_draft_auth_browser(
    instance_url: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError> {
    protocol::start_gitlab_oauth(instance_url, redirect_uri)
        .map_err(|error| draft_protocol_error("gitlab oauth start", error))
}

pub async fn finish_gitlab_draft_auth_browser(
    instance_url: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError> {
    protocol::exchange_gitlab_oauth_code(instance_url, code, pkce_verifier, redirect_uri)
        .await
        .map(Into::into)
        .map_err(|error| draft_protocol_error("gitlab oauth token exchange", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_errors_keep_public_classification_and_never_dump_token_objects() {
        let error = draft_protocol_error(
            "openai device oauth poll",
            ProviderError::HttpStatus {
                provider: "openai".into(),
                status: reqwest::StatusCode::BAD_REQUEST,
                body: r#"{"access_token":"private-access","refresh_token":"private-refresh"}"#
                    .into(),
                kind: agena_provider::ProviderErrorKind::InvalidRequest,
                retryable: false,
            },
        );
        assert_eq!(error.kind, crate::RuntimeAuthenticationErrorKind::Internal);
        assert!(error.message.contains("openai device oauth poll"));
        assert!(error.message.contains("400"));
        assert!(!error.message.contains("private-access"));
        assert!(!error.message.contains("private-refresh"));
        let error = draft_protocol_error(
            "start",
            ProviderError::Config("invalid redirect uri".into()),
        );
        assert_eq!(
            error.kind,
            crate::RuntimeAuthenticationErrorKind::BadRequest
        );
    }

    #[test]
    fn draft_error_projection_hides_opaque_json_with_protocol_diagnostic_prefixes() {
        let detail = draft_auth_public_error_detail(
            br#"openai oauth token exchange failed: {"opaque_provider_payload":"private-value"}"#,
        );
        assert!(!detail.contains("private-value"));
        assert_eq!(
            detail,
            "provider returned an error response without a public diagnostic"
        );
        assert_eq!(draft_auth_public_error_detail(
            br#"openai oauth token exchange failed: {"message":"Please try again","opaque_provider_payload":"private-value"}"#),
            "Please try again");
    }

    #[test]
    fn draft_browser_uses_shared_pkce_and_reports_invalid_redirects() {
        let start = start_openai_draft_auth_browser("http://localhost/callback").unwrap();
        assert!(!start.state.is_empty());
        assert!(!start.pkce_verifier.is_empty());
        assert!(start.authorize_url.contains("code_challenge_method=S256"));
        let error = start_openai_draft_auth_browser(":bad redirect").unwrap_err();
        assert_eq!(
            error.kind,
            crate::RuntimeAuthenticationErrorKind::BadRequest
        );
    }
}
