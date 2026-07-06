use serde::Deserialize;

use crate::{
    error::{AppError, ProviderErrorKind},
    provider::CODEX_ORIGINATOR,
};

use super::super::{DeviceCodeStart, OAuthAuthorizeStart, OAuthTokenResponse};
use super::shared::{
    OPENAI_CLIENT_ID, OPENAI_ISSUER, ensure_http_success, expires_at_ms_from_seconds,
    extract_openai_account_id, extract_openai_fedramp_account, oauth_authorize_start, oauth_client,
    oauth_error_code, oauth_error_summary, parse_device_auth_interval,
};

pub fn start_openai_browser_oauth(redirect_uri: &str) -> Result<OAuthAuthorizeStart, AppError> {
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
            ("originator", CODEX_ORIGINATOR),
        ],
    ))
}

pub async fn exchange_openai_oauth_code(
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse, AppError> {
    let encoded_form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", code);
        form.append_pair("redirect_uri", redirect_uri);
        form.append_pair("client_id", OPENAI_CLIENT_ID);
        form.append_pair("code_verifier", pkce_verifier);
        form.finish()
    };

    request_openai_oauth_token(
        encoded_form,
        "openai oauth token exchange failed",
        None,
        false,
    )
    .await
}

pub async fn refresh_openai_token(refresh_token: &str) -> Result<OAuthTokenResponse, AppError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AppError::Config(
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
        encoded_form,
        "openai oauth token refresh failed",
        Some(refresh_token),
        true,
    )
    .await
}

pub async fn revoke_openai_token(refresh_token: &str, access_token: &str) -> Result<(), AppError> {
    let refresh_token = refresh_token.trim();
    let access_token = access_token.trim();
    let (token, token_type_hint, client_id) = if !refresh_token.is_empty() {
        (
            refresh_token,
            "refresh_token",
            Some(OPENAI_CLIENT_ID.to_owned()),
        )
    } else if !access_token.is_empty() {
        (access_token, "access_token", None)
    } else {
        return Ok(());
    };

    let mut body = serde_json::Map::from_iter([
        (
            "token".to_owned(),
            serde_json::Value::String(token.to_owned()),
        ),
        (
            "token_type_hint".to_owned(),
            serde_json::Value::String(token_type_hint.to_owned()),
        ),
    ]);
    if let Some(client_id) = client_id {
        body.insert("client_id".to_owned(), serde_json::Value::String(client_id));
    }

    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/revoke"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            crate::provider::codex_user_agent(),
        )
        .json(&serde_json::Value::Object(body))
        .send()
        .await?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(AppError::Provider(format!(
        "openai oauth token revoke failed with status {status}: {}",
        oauth_error_summary(body.as_str())
    )))
}

pub async fn start_openai_headless_device_code() -> Result<DeviceCodeStart, AppError> {
    #[derive(Debug, Deserialize)]
    struct DeviceCodeResponse {
        device_auth_id: String,
        user_code: String,
        #[serde(default)]
        interval: Option<serde_json::Value>,
    }

    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/usercode"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            crate::provider::codex_user_agent(),
        )
        .json(&serde_json::json!({
            "client_id": OPENAI_CLIENT_ID,
        }))
        .send()
        .await?;

    let response = ensure_http_success("openai", None, response).await?;
    let data: DeviceCodeResponse = response.json().await?;
    Ok(DeviceCodeStart {
        verification_url: format!("{OPENAI_ISSUER}/codex/device"),
        user_code: data.user_code,
        device_code: data.device_auth_id,
        interval_seconds: parse_device_auth_interval(data.interval, 5),
    })
}

pub async fn poll_openai_headless_device_code(
    device_auth_id: &str,
    user_code: &str,
) -> Result<Option<OAuthTokenResponse>, AppError> {
    #[derive(Debug, Deserialize)]
    struct DevicePollResponse {
        authorization_code: String,
        code_verifier: String,
    }

    #[derive(Debug, Deserialize)]
    struct TokenResponseBody {
        #[serde(default)]
        id_token: Option<String>,
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let poll_response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/token"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            crate::provider::codex_user_agent(),
        )
        .json(&serde_json::json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .send()
        .await?;

    if poll_response.status() == reqwest::StatusCode::FORBIDDEN
        || poll_response.status() == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(None);
    }

    let poll_response = ensure_http_success("openai", None, poll_response).await?;
    let poll_data: DevicePollResponse = poll_response.json().await?;

    let encoded_form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", poll_data.authorization_code.as_str());
        form.append_pair(
            "redirect_uri",
            "https://auth.openai.com/deviceauth/callback",
        );
        form.append_pair("client_id", OPENAI_CLIENT_ID);
        form.append_pair("code_verifier", poll_data.code_verifier.as_str());
        form.finish()
    };

    let token_response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(
            reqwest::header::USER_AGENT,
            crate::provider::codex_user_agent(),
        )
        .body(encoded_form)
        .send()
        .await?;

    let token_response = ensure_http_success("openai", None, token_response).await?;
    let token_data: TokenResponseBody = token_response.json().await?;
    let refresh_token = token_data.refresh_token.ok_or_else(|| {
        AppError::Provider("openai device oauth response missing refresh_token".to_owned())
    })?;

    Ok(Some(OAuthTokenResponse {
        refresh: refresh_token,
        access: token_data.access_token.clone(),
        id_token: token_data.id_token.clone(),
        expires_at_ms: expires_at_ms_from_seconds(token_data.expires_in),
        account_id: openai_account_id_from_token_bodies(
            token_data.id_token.as_deref(),
            token_data.access_token.as_str(),
        ),
        chatgpt_account_is_fedramp: openai_fedramp_from_token_bodies(
            token_data.id_token.as_deref(),
            token_data.access_token.as_str(),
        ),
        user: None,
    }))
}

fn openai_oauth_client(redirect_uri: Option<&str>) -> Result<super::shared::OAuthClient, AppError> {
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
    encoded_form: String,
    error_context: &str,
    refresh_fallback: Option<&str>,
    refresh_flow: bool,
) -> Result<OAuthTokenResponse, AppError> {
    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(
            reqwest::header::USER_AGENT,
            crate::provider::codex_user_agent(),
        )
        .body(encoded_form)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(openai_oauth_token_error(response, error_context, refresh_flow).await);
    }

    let token: OpenAiTokenResponseBody = response.json().await?;
    let refresh = token
        .refresh_token
        .or_else(|| refresh_fallback.map(str::to_owned))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Provider(format!("{error_context}: missing refresh_token")))?;

    Ok(OAuthTokenResponse {
        refresh,
        access: token.access_token.clone(),
        id_token: token.id_token.clone(),
        expires_at_ms: expires_at_ms_from_seconds(token.expires_in),
        account_id: openai_account_id_from_token_bodies(
            token.id_token.as_deref(),
            token.access_token.as_str(),
        ),
        chatgpt_account_is_fedramp: openai_fedramp_from_token_bodies(
            token.id_token.as_deref(),
            token.access_token.as_str(),
        ),
        user: None,
    })
}

fn openai_account_id_from_token_bodies(
    id_token: Option<&str>,
    access_token: &str,
) -> Option<String> {
    id_token
        .and_then(extract_openai_account_id)
        .or_else(|| extract_openai_account_id(access_token))
}

fn openai_fedramp_from_token_bodies(id_token: Option<&str>, access_token: &str) -> bool {
    id_token
        .and_then(extract_openai_fedramp_account)
        .or_else(|| extract_openai_fedramp_account(access_token))
        .unwrap_or(false)
}

async fn openai_oauth_token_error(
    response: reqwest::Response,
    error_context: &str,
    refresh_flow: bool,
) -> AppError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = oauth_error_summary(body.as_str());

    if refresh_flow
        && status == reqwest::StatusCode::UNAUTHORIZED
        && let Some(message) = openai_refresh_failure_message(body.as_str())
    {
        return AppError::Provider(message);
    }

    AppError::HttpStatus {
        provider: "openai".to_owned(),
        status,
        body: format!("{error_context}: {detail}"),
        kind: ProviderErrorKind::ApiError,
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
