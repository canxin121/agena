use serde::Deserialize;

use crate::ProviderError;
use crate::RUNTIME_CODEX_ORIGINATOR;
use agena_provider::{DeviceCodeStart, OAuthAuthorizeStart, ProviderErrorKind};

use super::shared::{
    OPENAI_CLIENT_ID, OPENAI_ISSUER, ensure_http_success, expires_at_ms_from_seconds,
    oauth_authorize_start, oauth_client, oauth_error_code, oauth_error_summary,
    openai_account_identity, parse_device_auth_interval, provider_http_client,
    response_body_bounded, response_json_bounded,
};
use agena_provider::OAuthTokenResponse;

/// Request-local identity keeps the shared HTTP pool independent of any
/// runtime generation, including token refreshes and device authorization.
fn openai_auth_post(
    user_agent: &str,
    endpoint: impl reqwest::IntoUrl,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    Ok(provider_http_client()?
        .post(endpoint)
        .header(reqwest::header::USER_AGENT, user_agent))
}

/// Payload compatibility is explicit: device and draft flows historically
/// accept a present empty refresh value; saved browser login rejects it.
/// This controls decoding only and grants no credential-store access.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpenAiRefreshTokenRequirement {
    Present,
    NonEmpty,
}

pub fn start_openai_browser_oauth(
    redirect_uri: &str,
) -> Result<OAuthAuthorizeStart, ProviderError> {
    let client = openai_oauth_client(Some(redirect_uri))?;

    Ok(oauth_authorize_start(
        &client,
        &[
            "openid",
            "profile",
            "email",
            "offline_access",
            "api.connectors.read",
            "api.connectors.invoke",
        ],
        &[
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", RUNTIME_CODEX_ORIGINATOR),
        ],
    ))
}

pub async fn exchange_openai_oauth_code(
    user_agent: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
    refresh_requirement: OpenAiRefreshTokenRequirement,
) -> Result<OAuthTokenResponse, ProviderError> {
    let encoded_form = authorization_code_form(code, pkce_verifier, redirect_uri);

    request_openai_oauth_token(
        user_agent,
        encoded_form,
        "openai oauth token exchange failed",
        None,
        false,
        refresh_requirement,
    )
    .await
}

pub async fn refresh_openai_token(
    user_agent: &str,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, ProviderError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(ProviderError::Config(
            "openai oauth refresh token is empty".to_owned(),
        ));
    }

    let encoded_form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "refresh_token");
        form.append_pair("refresh_token", refresh_token);
        form.append_pair("client_id", OPENAI_CLIENT_ID);
        form.finish()
    };

    request_openai_oauth_token(
        user_agent,
        encoded_form,
        "openai oauth token refresh failed",
        Some(refresh_token),
        true,
        OpenAiRefreshTokenRequirement::NonEmpty,
    )
    .await
}

pub async fn start_openai_headless_device_code(
    user_agent: &str,
) -> Result<DeviceCodeStart, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct DeviceCodeResponse {
        device_auth_id: String,
        user_code: String,
        #[serde(default)]
        interval: Option<serde_json::Value>,
    }

    let response = openai_auth_post(
        user_agent,
        format!("{OPENAI_ISSUER}/api/accounts/deviceauth/usercode"),
    )?
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .json(&serde_json::json!({
        "client_id": OPENAI_CLIENT_ID,
    }))
    .send()
    .await?;

    let response = ensure_http_success("openai", None, response).await?;
    let data: DeviceCodeResponse = response_json_bounded(response).await?;
    Ok(DeviceCodeStart {
        verification_url: format!("{OPENAI_ISSUER}/codex/device"),
        user_code: data.user_code,
        device_code: data.device_auth_id,
        interval_seconds: parse_device_auth_interval(data.interval, 5),
    })
}

pub async fn poll_openai_headless_device_code(
    user_agent: &str,
    device_auth_id: &str,
    user_code: &str,
) -> Result<Option<OAuthTokenResponse>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct DevicePollResponse {
        authorization_code: String,
        code_verifier: String,
    }

    let poll_response = openai_auth_post(
        user_agent,
        format!("{OPENAI_ISSUER}/api/accounts/deviceauth/token"),
    )?
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .json(&serde_json::json!({
        "device_auth_id": device_auth_id,
        "user_code": user_code,
    }))
    .send()
    .await?;

    if openai_device_authorization_pending(poll_response.status()) {
        return Ok(None);
    }

    let poll_response = ensure_http_success("openai", None, poll_response).await?;
    let poll_data: DevicePollResponse = response_json_bounded(poll_response).await?;

    let encoded_form = authorization_code_form(
        &poll_data.authorization_code,
        &poll_data.code_verifier,
        &format!("{OPENAI_ISSUER}/deviceauth/callback"),
    );

    let token_response = openai_auth_post(user_agent, format!("{OPENAI_ISSUER}/oauth/token"))?
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(encoded_form)
        .send()
        .await?;

    let token_response = ensure_http_success("openai", None, token_response).await?;
    let mut token_data: OpenAiTokenResponseBody = response_json_bounded(token_response).await?;
    let refresh_token = token_data.refresh_token.take().ok_or_else(|| {
        ProviderError::Provider("openai device oauth response missing refresh_token".to_owned())
    })?;
    Ok(Some(token_data.into_response(refresh_token)))
}

fn openai_device_authorization_pending(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    )
}

fn authorization_code_form(code: &str, verifier: &str, redirect_uri: &str) -> String {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "authorization_code");
    form.append_pair("code", code);
    form.append_pair("redirect_uri", redirect_uri);
    form.append_pair("client_id", OPENAI_CLIENT_ID);
    form.append_pair("code_verifier", verifier);
    form.finish()
}

fn openai_oauth_client(
    redirect_uri: Option<&str>,
) -> Result<super::shared::OAuthClient, ProviderError> {
    oauth_client(
        "openai",
        OPENAI_CLIENT_ID,
        None,
        format!("{OPENAI_ISSUER}/oauth/authorize"),
        format!("{OPENAI_ISSUER}/oauth/token"),
        redirect_uri,
    )
}

#[derive(Debug, Deserialize)]
struct OpenAiTokenResponseBody {
    #[serde(default)]
    id_token: Option<String>,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

async fn request_openai_oauth_token(
    user_agent: &str,
    encoded_form: String,
    error_context: &str,
    refresh_fallback: Option<&str>,
    refresh_flow: bool,
    refresh_requirement: OpenAiRefreshTokenRequirement,
) -> Result<OAuthTokenResponse, ProviderError> {
    let response = openai_auth_post(user_agent, format!("{OPENAI_ISSUER}/oauth/token"))?
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(encoded_form)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(openai_oauth_token_error(response, error_context, refresh_flow).await);
    }

    let token: OpenAiTokenResponseBody = response_json_bounded(response).await?;
    token.require_refresh(refresh_fallback, refresh_requirement, error_context)
}

impl OpenAiTokenResponseBody {
    fn require_refresh(
        mut self,
        fallback: Option<&str>,
        requirement: OpenAiRefreshTokenRequirement,
        error_context: &str,
    ) -> Result<OAuthTokenResponse, ProviderError> {
        let refresh = self
            .refresh_token
            .take()
            .or_else(|| fallback.map(str::to_owned))
            .filter(|value| {
                requirement == OpenAiRefreshTokenRequirement::Present || !value.trim().is_empty()
            })
            .ok_or_else(|| {
                ProviderError::Provider(format!("{error_context}: missing refresh_token"))
            })?;
        Ok(self.into_response(refresh))
    }

    fn into_response(self, refresh: String) -> OAuthTokenResponse {
        let (account_id, chatgpt_account_is_fedramp) =
            openai_account_identity(self.id_token.as_deref(), &self.access_token);
        OAuthTokenResponse {
            refresh,
            access: self.access_token,
            id_token: self.id_token,
            expires_at_ms: expires_at_ms_from_seconds(self.expires_in),
            account_id,
            chatgpt_account_is_fedramp,
            user: None,
        }
    }
}

async fn openai_oauth_token_error(
    response: reqwest::Response,
    error_context: &str,
    refresh_flow: bool,
) -> ProviderError {
    let status = response.status();
    let body = response_body_bounded(response)
        .await
        .map(|body| String::from_utf8_lossy(&body).into_owned())
        .unwrap_or_default();
    let detail = oauth_error_summary(body.as_str());

    if refresh_flow
        && status == reqwest::StatusCode::UNAUTHORIZED
        && let Some(message) = openai_refresh_failure_message(body.as_str())
    {
        return ProviderError::Provider(message);
    }

    ProviderError::HttpStatus {
        provider: "openai".to_owned(),
        status,
        body: format!("{error_context}: {detail}"),
        kind: if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            ProviderErrorKind::Authentication
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            ProviderErrorKind::RateLimited
        } else if status.is_server_error() {
            ProviderErrorKind::Unavailable
        } else {
            ProviderErrorKind::InvalidRequest
        },
        retryable: status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
    }
}

fn openai_refresh_failure_message(body: &str) -> Option<String> {
    let code = oauth_error_code(body)?.to_ascii_lowercase();
    let message = match code.as_str() {
        "refresh_token_expired" => {
            "Your access token could not be refreshed because your refresh token has expired. Please sign in again."
        }
        "refresh_token_reused" => {
            "Your access token could not be refreshed because your refresh token was already used. Please sign in again."
        }
        "refresh_token_invalidated" => {
            "Your access token could not be refreshed because your refresh token was revoked. Please sign in again."
        }
        _ => return None,
    };
    Some(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browser_start_keeps_pkce_state_scopes_and_originator() {
        let start = start_openai_browser_oauth("http://localhost:3210/callback").unwrap();
        let url = url::Url::parse(&start.authorize_url).unwrap();
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["state"], start.state);
        assert!(!start.state.is_empty());
        assert_eq!(query["client_id"], OPENAI_CLIENT_ID);
        assert_eq!(query["originator"], RUNTIME_CODEX_ORIGINATOR);
        assert_eq!(query["redirect_uri"], "http://localhost:3210/callback");
        assert_eq!(query["code_challenge_method"], "S256");
        let challenge = oauth2::PkceCodeChallenge::from_code_verifier_sha256(
            &oauth2::PkceCodeVerifier::new(start.pkce_verifier),
        );
        assert_eq!(&query["code_challenge"], challenge.as_str());
        assert!(
            query["scope"]
                .split_whitespace()
                .any(|scope| scope == "offline_access")
        );
        assert!(
            query["scope"]
                .split_whitespace()
                .any(|scope| scope == "api.connectors.invoke")
        );
    }

    #[test]
    fn token_form_and_request_keep_exact_request_local_identity() {
        let form =
            authorization_code_form("code & +", "verifier/=", "http://localhost/callback?q=1");
        let values: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(values["code"], "code & +");
        assert_eq!(values["code_verifier"], "verifier/=");
        assert_eq!(values["grant_type"], "authorization_code");
        assert_eq!(values["client_id"], OPENAI_CLIENT_ID);
        let request = openai_auth_post("test-agent/1", "https://example.invalid/oauth/token")
            .unwrap()
            .body(form)
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[reqwest::header::USER_AGENT],
            "test-agent/1"
        );
    }

    #[test]
    fn token_refresh_requirements_preserve_draft_and_saved_login_compatibility() {
        let parse = |value| serde_json::from_value::<OpenAiTokenResponseBody>(value).unwrap();
        let empty = json!({"access_token": "access", "refresh_token": ""});
        assert!(
            parse(empty.clone())
                .require_refresh(None, OpenAiRefreshTokenRequirement::Present, "exchange")
                .is_ok()
        );
        assert!(
            parse(empty)
                .require_refresh(None, OpenAiRefreshTokenRequirement::NonEmpty, "exchange")
                .is_err()
        );
        assert!(
            parse(json!({"access_token": "access"}))
                .require_refresh(None, OpenAiRefreshTokenRequirement::Present, "exchange")
                .is_err()
        );
        let refreshed = parse(json!({"access_token": "access"}))
            .require_refresh(
                Some("previous"),
                OpenAiRefreshTokenRequirement::NonEmpty,
                "refresh",
            )
            .unwrap();
        assert_eq!(refreshed.refresh, "previous");
        assert_eq!(refreshed.expires_at_ms, 0);
    }

    #[test]
    fn pending_statuses_and_refresh_failures_keep_protocol_meaning() {
        assert!(openai_device_authorization_pending(
            reqwest::StatusCode::FORBIDDEN
        ));
        assert!(openai_device_authorization_pending(
            reqwest::StatusCode::NOT_FOUND
        ));
        for status in [
            reqwest::StatusCode::OK,
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(!openai_device_authorization_pending(status));
        }
        assert!(
            openai_refresh_failure_message(r#"{"error":"refresh_token_expired"}"#)
                .unwrap()
                .contains("expired")
        );
        assert!(
            openai_refresh_failure_message(r#"{"error":{"code":"refresh_token_reused"}}"#)
                .unwrap()
                .contains("already used")
        );
        assert!(
            openai_refresh_failure_message(r#"{"error":"refresh_token_invalidated"}"#)
                .unwrap()
                .contains("revoked")
        );
    }
}
