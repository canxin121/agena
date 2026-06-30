use std::{sync::OnceLock, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointSet, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, StandardTokenResponse, TokenResponse, TokenUrl,
    basic::BasicClient, basic::BasicTokenType,
};

use crate::error::{AppError, ProviderErrorKind};

use super::super::{OAuthAuthorizeStart, OAuthTokenResponse};

pub(super) const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const OPENAI_ISSUER: &str = "https://auth.openai.com";
pub(super) const COPILOT_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

pub(super) type OAuthClient = BasicClient<
    EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    EndpointSet,
>;

type OAuthProviderToken = StandardTokenResponse<oauth2::EmptyExtraTokenFields, BasicTokenType>;

pub(super) fn normalize_domain(url_or_domain: &str) -> String {
    url_or_domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

pub(super) fn oauth_client(
    provider: &str,
    client_id: impl Into<String>,
    client_secret: Option<String>,
    auth_url: String,
    token_url: String,
    redirect_uri: Option<&str>,
) -> Result<OAuthClient, AppError> {
    let client =
        BasicClient::new(ClientId::new(client_id.into()))
            .set_auth_uri(AuthUrl::new(auth_url).map_err(|error| {
                AppError::Config(format!("invalid {provider} auth url: {error}"))
            })?)
            .set_token_uri(TokenUrl::new(token_url).map_err(|error| {
                AppError::Config(format!("invalid {provider} token url: {error}"))
            })?);

    let client = if let Some(client_secret) = client_secret {
        client.set_client_secret(ClientSecret::new(client_secret))
    } else {
        client
    };

    if let Some(redirect_uri) = redirect_uri {
        Ok(client.set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_owned())
                .map_err(|error| AppError::Config(format!("invalid redirect uri: {error}")))?,
        ))
    } else {
        Ok(client)
    }
}

pub(super) fn oauth_authorize_start(
    client: &OAuthClient,
    scopes: &[&str],
    extra_params: &[(&str, &str)],
) -> OAuthAuthorizeStart {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge);

    for scope in scopes {
        request = request.add_scope(Scope::new((*scope).to_owned()));
    }
    for (key, value) in extra_params {
        request = request.add_extra_param(*key, *value);
    }

    let (url, state) = request.url();
    OAuthAuthorizeStart {
        authorize_url: url.to_string(),
        state: state.secret().to_owned(),
        pkce_verifier: verifier.secret().to_owned(),
    }
}

pub(super) async fn exchange_oauth_code(
    client: OAuthClient,
    code: &str,
    pkce_verifier: &str,
    http_client: &oauth2::reqwest::Client,
    error_context: &str,
    account_id_from_access: impl FnOnce(&str) -> Option<String>,
) -> Result<OAuthTokenResponse, AppError> {
    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(http_client)
        .await
        .map_err(|error| AppError::Provider(format!("{error_context}: {error}")))?;

    Ok(oauth_token_response(token, None, account_id_from_access))
}

pub(super) async fn refresh_oauth_token(
    client: OAuthClient,
    refresh_token: &str,
    http_client: &oauth2::reqwest::Client,
    error_context: &str,
    account_id_from_access: impl FnOnce(&str) -> Option<String>,
) -> Result<OAuthTokenResponse, AppError> {
    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(http_client)
        .await
        .map_err(|error| AppError::Provider(format!("{error_context}: {error}")))?;

    Ok(oauth_token_response(
        token,
        Some(refresh_token),
        account_id_from_access,
    ))
}

pub(super) fn parse_device_auth_interval(
    raw: Option<serde_json::Value>,
    default_seconds: u64,
) -> u64 {
    let Some(raw) = raw else {
        return default_seconds;
    };

    if let Some(interval) = raw.as_u64() {
        return interval.max(1);
    }

    if let Some(interval) = raw
        .as_str()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return interval.max(1);
    }

    default_seconds
}

pub(super) fn expires_at_ms_from_duration(expires_in: Option<Duration>) -> i64 {
    expires_in
        .map(|duration| duration.as_millis())
        .and_then(|millis| i64::try_from(millis).ok())
        .filter(|millis| *millis > 0)
        .map(|millis| chrono::Utc::now().timestamp_millis() + millis)
        .unwrap_or(0)
}

pub(super) fn expires_at_ms_from_seconds<T>(expires_in: Option<T>) -> i64
where
    T: TryInto<i64>,
{
    expires_in
        .and_then(|seconds| seconds.try_into().ok())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds.saturating_mul(1_000))
        .unwrap_or(0)
}

pub(super) async fn ensure_http_success(
    provider: &str,
    context: Option<&str>,
    response: reqwest::Response,
) -> Result<reqwest::Response, AppError> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<empty>".to_owned());
    let body = match context {
        Some(context) => format!("{context}: {body}"),
        None => body,
    };

    Err(AppError::HttpStatus {
        provider: provider.to_owned(),
        status,
        body,
        kind: ProviderErrorKind::ApiError,
        retryable: false,
    })
}

pub(super) fn oauth_http_client(use_codex_user_agent: bool) -> &'static oauth2::reqwest::Client {
    static DEFAULT_CLIENT: OnceLock<oauth2::reqwest::Client> = OnceLock::new();
    static CODEX_CLIENT: OnceLock<oauth2::reqwest::Client> = OnceLock::new();

    if use_codex_user_agent {
        CODEX_CLIENT.get_or_init(|| {
            oauth2::reqwest::ClientBuilder::new()
                .redirect(oauth2::reqwest::redirect::Policy::none())
                .user_agent(crate::provider::codex_user_agent())
                .build()
                .expect("oauth reqwest client should build")
        })
    } else {
        DEFAULT_CLIENT.get_or_init(|| {
            oauth2::reqwest::ClientBuilder::new()
                .redirect(oauth2::reqwest::redirect::Policy::none())
                .build()
                .expect("oauth reqwest client should build")
        })
    }
}

fn oauth_token_response(
    token: OAuthProviderToken,
    refresh_fallback: Option<&str>,
    account_id_from_access: impl FnOnce(&str) -> Option<String>,
) -> OAuthTokenResponse {
    let access = token.access_token().secret().to_owned();

    OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|value| value.secret().to_owned())
            .unwrap_or_else(|| refresh_fallback.unwrap_or_default().to_owned()),
        access: access.clone(),
        expires_at_ms: expires_at_ms_from_duration(token.expires_in()),
        account_id: account_id_from_access(access.as_str()),
        user: None,
    }
}

pub(super) fn extract_openai_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    json.get("chatgpt_account_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|value| value.get("chatgpt_account_id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            json.get("organizations")
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}
