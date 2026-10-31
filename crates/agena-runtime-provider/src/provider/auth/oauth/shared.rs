use std::{sync::OnceLock, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointSet, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, StandardTokenResponse, TokenResponse, TokenUrl,
    basic::BasicClient, basic::BasicTokenType,
};
use serde_json::Value;

use crate::ProviderError;
use agena_provider::ProviderErrorKind;

use agena_provider::OAuthAuthorizeStart;

use agena_provider::OAuthTokenResponse;

pub(super) const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const OPENAI_ISSUER: &str = "https://auth.openai.com";
pub(super) const COPILOT_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;

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

pub(super) fn provider_http_client() -> Result<&'static reqwest::Client, ProviderError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(OAUTH_HTTP_TIMEOUT)
                .build()
                .map_err(|error| agena_failure::diagnostic::format_error_chain(&error))
        })
        .as_ref()
        .map_err(|error| {
            ProviderError::Provider(format!(
                "cannot initialize authentication HTTP client: {error}"
            ))
        })
}

pub(super) async fn response_body_bounded(
    mut response: reqwest::Response,
) -> Result<Vec<u8>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_OAUTH_RESPONSE_BYTES as u64)
    {
        return Err(ProviderError::Provider(
            "OAuth response exceeds the 1 MiB limit".to_owned(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_OAUTH_RESPONSE_BYTES {
            return Err(ProviderError::Provider(
                "OAuth response exceeds the 1 MiB limit".to_owned(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn response_json_bounded<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ProviderError> {
    Ok(serde_json::from_slice(
        &response_body_bounded(response).await?,
    )?)
}

pub(super) fn oauth_client(
    provider: &str,
    client_id: impl Into<String>,
    client_secret: Option<String>,
    auth_url: String,
    token_url: String,
    redirect_uri: Option<&str>,
) -> Result<OAuthClient, ProviderError> {
    let client = BasicClient::new(ClientId::new(client_id.into()))
        .set_auth_uri(AuthUrl::new(auth_url).map_err(|error| {
            ProviderError::Config(format!("invalid {provider} auth url: {error}"))
        })?)
        .set_token_uri(TokenUrl::new(token_url).map_err(|error| {
            ProviderError::Config(format!("invalid {provider} token url: {error}"))
        })?);

    let client = if let Some(client_secret) = client_secret {
        client.set_client_secret(ClientSecret::new(client_secret))
    } else {
        client
    };

    if let Some(redirect_uri) = redirect_uri {
        Ok(client
            .set_redirect_uri(RedirectUrl::new(redirect_uri.to_owned()).map_err(|error| {
                ProviderError::Config(format!("invalid redirect uri: {error}"))
            })?))
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
) -> Result<OAuthTokenResponse, ProviderError> {
    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(http_client)
        .await
        .map_err(|error| ProviderError::Provider(format!("{error_context}: {error}")))?;

    Ok(oauth_token_response(token, None, account_id_from_access))
}

pub(super) async fn refresh_oauth_token(
    client: OAuthClient,
    refresh_token: &str,
    http_client: &oauth2::reqwest::Client,
    error_context: &str,
    account_id_from_access: impl FnOnce(&str) -> Option<String>,
) -> Result<OAuthTokenResponse, ProviderError> {
    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(http_client)
        .await
        .map_err(|error| ProviderError::Provider(format!("{error_context}: {error}")))?;

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
        .map(|millis| chrono::Utc::now().timestamp_millis().saturating_add(millis))
        .unwrap_or(0)
}

pub(super) fn expires_at_ms_from_seconds<T>(expires_in: Option<T>) -> i64
where
    T: TryInto<i64>,
{
    expires_in
        .and_then(|seconds| seconds.try_into().ok())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| {
            chrono::Utc::now()
                .timestamp_millis()
                .saturating_add(seconds.saturating_mul(1_000))
        })
        .unwrap_or(0)
}

pub(super) fn oauth_error_message(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let json = match serde_json::from_str::<Value>(body) {
        Ok(json) => Some(json),
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode an OAuth error response as JSON",
                    &error,
                ),
                "OAuth error response is plain text"
            );
            None
        }
    };
    if let Some(json) = json.as_ref() {
        if let Some(error) = json.get("error") {
            match error {
                Value::Object(obj) => {
                    for key in ["message", "error_description", "description", "detail"] {
                        if let Some(message) = obj.get(key).and_then(Value::as_str) {
                            let message = message.trim();
                            if !message.is_empty() {
                                return Some(message.to_owned());
                            }
                        }
                    }
                }
                Value::String(message) => {
                    let message = message.trim();
                    if !message.is_empty() {
                        return Some(message.to_owned());
                    }
                }
                _ => {}
            }
        }

        for key in ["error_description", "message", "detail"] {
            if let Some(message) = json.get(key).and_then(Value::as_str) {
                let message = message.trim();
                if !message.is_empty() {
                    return Some(message.to_owned());
                }
            }
        }
    }

    Some(body.to_owned())
}

pub(super) fn oauth_error_code(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let json = match serde_json::from_str::<Value>(body) {
        Ok(json) => json,
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode an OAuth error code response as JSON",
                    &error,
                ),
                "OAuth response did not contain a structured error code"
            );
            return None;
        }
    };
    let Value::Object(map) = json else {
        tracing::debug!("OAuth JSON response was not an object and had no structured error code");
        return None;
    };

    if let Some(error_value) = map.get("error") {
        match error_value {
            Value::Object(obj) => {
                if let Some(code) = obj.get("code").and_then(Value::as_str) {
                    return Some(code.to_owned());
                }
                if let Some(code) = obj.get("error").and_then(Value::as_str) {
                    return Some(code.to_owned());
                }
            }
            Value::String(code) => {
                let code = code.trim();
                if !code.is_empty() {
                    return Some(code.to_owned());
                }
            }
            _ => {}
        }
    }

    map.get("code")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            map.get("error_code")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

pub(super) fn oauth_error_summary(body: &str) -> String {
    oauth_error_message(body).unwrap_or_else(|| "Unknown error".to_owned())
}

pub(super) async fn ensure_http_success(
    provider: &str,
    context: Option<&str>,
    response: reqwest::Response,
) -> Result<reqwest::Response, ProviderError> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response_body_bounded(response)
        .await
        .map(|body| String::from_utf8_lossy(&body).into_owned())
        .unwrap_or_else(|_| "<empty or oversized>".to_owned());
    let body = oauth_error_summary(body.as_str());
    let body = match context {
        Some(context) => format!("{context}: {body}"),
        None => body,
    };

    Err(ProviderError::HttpStatus {
        provider: provider.to_owned(),
        status,
        body,
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
    })
}

pub(super) fn oauth_http_client() -> Result<&'static oauth2::reqwest::Client, ProviderError> {
    static CLIENT: OnceLock<Result<oauth2::reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            oauth2::reqwest::ClientBuilder::new()
                .redirect(oauth2::reqwest::redirect::Policy::none())
                .timeout(OAUTH_HTTP_TIMEOUT)
                .build()
                .map_err(|error| agena_failure::diagnostic::format_error_chain(&error))
        })
        .as_ref()
        .map_err(|error| {
            ProviderError::Provider(format!("cannot initialize OAuth HTTP client: {error}"))
        })
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
        id_token: None,
        expires_at_ms: expires_at_ms_from_duration(token.expires_in()),
        account_id: account_id_from_access(access.as_str()),
        chatgpt_account_is_fedramp: false,
        user: None,
    }
}

fn decode_openai_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn openai_account_id_from_claims(json: &Value) -> Option<String> {
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

fn openai_fedramp_from_claims(json: &Value) -> Option<bool> {
    json.get("chatgpt_account_is_fedramp")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|value| value.get("chatgpt_account_is_fedramp"))
                .and_then(|value| value.as_bool())
        })
}

/// Parse each token once; account id and FedRAMP claims independently prefer
/// the id token and fall back to the access token, as the protocol requires.
pub(super) fn openai_account_identity(
    id_token: Option<&str>,
    access_token: &str,
) -> (Option<String>, bool) {
    let id = id_token.and_then(decode_openai_jwt_payload);
    let access = decode_openai_jwt_payload(access_token);
    let account_id = id
        .as_ref()
        .and_then(openai_account_id_from_claims)
        .or_else(|| access.as_ref().and_then(openai_account_id_from_claims));
    let fedramp = id
        .as_ref()
        .and_then(openai_fedramp_from_claims)
        .or_else(|| access.as_ref().and_then(openai_fedramp_from_claims))
        .unwrap_or(false);
    (account_id, fedramp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    #[test]
    fn interval_and_expiry_keep_missing_zero_and_string_semantics() {
        for (value, expected) in [
            (None, 5),
            (Some(json!(0)), 1),
            (Some(json!(" 12 ")), 12),
            (Some(json!(-1)), 5),
            (Some(json!("invalid")), 5),
            (Some(Value::Null), 5),
        ] {
            assert_eq!(parse_device_auth_interval(value, 5), expected);
        }
        assert_eq!(expires_at_ms_from_seconds(None::<u64>), 0);
        assert_eq!(expires_at_ms_from_seconds(Some(0_u64)), 0);
        assert_eq!(expires_at_ms_from_seconds(Some(u64::MAX)), 0);
        assert_eq!(expires_at_ms_from_seconds(Some(i64::MAX)), i64::MAX);
        let before = chrono::Utc::now().timestamp_millis();
        let expires = expires_at_ms_from_seconds(Some(60_u64));
        let after = chrono::Utc::now().timestamp_millis();
        assert!((before + 60_000..=after + 60_000).contains(&expires));
    }

    #[test]
    fn account_claims_keep_independent_id_token_precedence_and_legacy_fallbacks() {
        let id =
            jwt(json!({"chatgpt_account_id": "id-account", "chatgpt_account_is_fedramp": false}));
        let access = jwt(json!({"https://api.openai.com/auth": {
            "chatgpt_account_id": "access-account", "chatgpt_account_is_fedramp": true
        }}));
        assert_eq!(
            openai_account_identity(Some(&id), &access),
            (Some("id-account".into()), false)
        );
        let id = jwt(json!({"organizations": [{"id": "organization"}]}));
        assert_eq!(
            openai_account_identity(Some(&id), &access),
            (Some("organization".into()), true)
        );
        assert_eq!(
            openai_account_identity(Some("malformed"), &access),
            (Some("access-account".into()), true)
        );
        assert_eq!(openai_account_identity(None, "malformed"), (None, false));
    }

    async fn http_response(status: u16, body: &str) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            // The response-budget test intentionally closes before reading
            // the oversized body; a peer reset is expected in that fixture.
            let _ = socket.write_all(response.as_bytes()).await;
        });
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn protocol_errors_preserve_retry_categories_and_response_budget() {
        for (status, kind, retryable) in [
            (401, ProviderErrorKind::Authentication, false),
            (403, ProviderErrorKind::Authentication, false),
            (429, ProviderErrorKind::RateLimited, true),
            (503, ProviderErrorKind::Unavailable, true),
            (400, ProviderErrorKind::InvalidRequest, false),
        ] {
            let response = http_response(status, r#"{"error":"failure"}"#).await;
            let error = ensure_http_success("provider", Some("exchange"), response)
                .await
                .unwrap_err();
            match error {
                ProviderError::HttpStatus {
                    status: actual,
                    kind: actual_kind,
                    retryable: actual_retryable,
                    ..
                } => {
                    assert_eq!(actual.as_u16(), status);
                    assert_eq!(actual_kind, kind);
                    assert_eq!(actual_retryable, retryable);
                }
                other => panic!("unexpected protocol error: {other}"),
            }
        }
        let response = http_response(200, &"x".repeat(MAX_OAUTH_RESPONSE_BYTES + 1)).await;
        assert!(response_body_bounded(response).await.is_err());
    }
}
