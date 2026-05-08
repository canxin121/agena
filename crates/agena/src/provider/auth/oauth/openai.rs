use std::sync::OnceLock;

use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, EndpointSet, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl, basic::BasicClient,
};
use serde::Deserialize;

use crate::error::{AppError, ProviderErrorKind};

use super::super::{DeviceCodeStart, OAuthAuthorizeStart, OAuthTokenResponse};
use super::shared::{
    OPENAI_CLIENT_ID, OPENAI_ISSUER, extract_openai_account_id, parse_device_auth_interval,
};

type OpenAiOAuthClient = BasicClient<
    EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    EndpointSet,
>;

pub fn start_openai_browser_oauth(redirect_uri: &str) -> Result<OAuthAuthorizeStart, AppError> {
    let client = openai_oauth_client(Some(redirect_uri))?;

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".to_owned()))
        .add_scope(Scope::new("profile".to_owned()))
        .add_scope(Scope::new("email".to_owned()))
        .add_scope(Scope::new("offline_access".to_owned()))
        .set_pkce_challenge(challenge);

    request = request.add_extra_param("id_token_add_organizations", "true");
    request = request.add_extra_param("codex_cli_simplified_flow", "true");
    request = request.add_extra_param("originator", "agena");

    let (url, state) = request.url();
    Ok(OAuthAuthorizeStart {
        authorize_url: url.to_string(),
        state: state.secret().to_owned(),
        pkce_verifier: verifier.secret().to_owned(),
    })
}

pub async fn exchange_openai_oauth_code(
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse, AppError> {
    let client = openai_oauth_client(Some(redirect_uri))?;

    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(oauth_http_client())
        .await
        .map_err(|error| {
            AppError::Provider(format!("openai oauth token exchange failed: {error}"))
        })?;

    let expires_at_ms = token
        .expires_in()
        .map(|duration| chrono::Utc::now().timestamp_millis() + duration.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|value| value.secret().to_owned())
            .unwrap_or_default(),
        access: token.access_token().secret().to_owned(),
        expires_at_ms,
        account_id: extract_openai_account_id(token.access_token().secret()),
    })
}

pub async fn refresh_openai_token(refresh_token: &str) -> Result<OAuthTokenResponse, AppError> {
    let client = openai_oauth_client(None)?;

    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(oauth_http_client())
        .await
        .map_err(|error| {
            AppError::Provider(format!("openai oauth token refresh failed: {error}"))
        })?;

    let expires_at_ms = token
        .expires_in()
        .map(|duration| chrono::Utc::now().timestamp_millis() + duration.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|value| value.secret().to_owned())
            .unwrap_or_else(|| refresh_token.to_owned()),
        access: token.access_token().secret().to_owned(),
        expires_at_ms,
        account_id: extract_openai_account_id(token.access_token().secret()),
    })
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
        .header(reqwest::header::USER_AGENT, "agena/0.1.0")
        .json(&serde_json::json!({
            "client_id": OPENAI_CLIENT_ID,
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<empty>".to_owned());
        return Err(AppError::HttpStatus {
            provider: "openai".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

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
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let poll_response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/token"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, "agena/0.1.0")
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

    if !poll_response.status().is_success() {
        let status = poll_response.status();
        let body = poll_response
            .text()
            .await
            .unwrap_or_else(|_| "<empty>".to_owned());
        return Err(AppError::HttpStatus {
            provider: "openai".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let poll_data: DevicePollResponse = poll_response.json().await?;

    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "authorization_code");
    form.append_pair("code", poll_data.authorization_code.as_str());
    form.append_pair(
        "redirect_uri",
        "https://auth.openai.com/deviceauth/callback",
    );
    form.append_pair("client_id", OPENAI_CLIENT_ID);
    form.append_pair("code_verifier", poll_data.code_verifier.as_str());
    let encoded_form = form.finish();

    let token_response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(encoded_form)
        .send()
        .await?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response
            .text()
            .await
            .unwrap_or_else(|_| "<empty>".to_owned());
        return Err(AppError::HttpStatus {
            provider: "openai".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let token_data: TokenResponseBody = token_response.json().await?;
    let refresh_token = token_data.refresh_token.ok_or_else(|| {
        AppError::Provider("openai device oauth response missing refresh_token".to_owned())
    })?;

    let expires_at_ms = token_data
        .expires_in
        .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds as i64 * 1000)
        .unwrap_or(0);

    Ok(Some(OAuthTokenResponse {
        refresh: refresh_token,
        access: token_data.access_token.clone(),
        expires_at_ms,
        account_id: extract_openai_account_id(token_data.access_token.as_str()),
    }))
}

fn openai_oauth_client(redirect_uri: Option<&str>) -> Result<OpenAiOAuthClient, AppError> {
    let client = BasicClient::new(ClientId::new(OPENAI_CLIENT_ID.to_owned()))
        .set_auth_uri(
            AuthUrl::new(format!("{OPENAI_ISSUER}/oauth/authorize"))
                .map_err(|error| AppError::Config(format!("invalid openai auth url: {error}")))?,
        )
        .set_token_uri(
            TokenUrl::new(format!("{OPENAI_ISSUER}/oauth/token"))
                .map_err(|error| AppError::Config(format!("invalid openai token url: {error}")))?,
        );

    if let Some(redirect_uri) = redirect_uri {
        Ok(client.set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_owned())
                .map_err(|error| AppError::Config(format!("invalid redirect uri: {error}")))?,
        ))
    } else {
        Ok(client)
    }
}

fn oauth_http_client() -> &'static oauth2::reqwest::Client {
    static CLIENT: OnceLock<oauth2::reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        oauth2::reqwest::ClientBuilder::new()
            .redirect(oauth2::reqwest::redirect::Policy::none())
            .build()
            .expect("oauth reqwest client should build")
    })
}
