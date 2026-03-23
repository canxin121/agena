use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse, TokenUrl, basic::BasicClient,
};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

use crate::{
    auth::{DeviceCodeStart, OAuthAuthorizeStart, OAuthTokenResponse},
    error::{AppError, ProviderErrorKind},
};

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_ISSUER: &str = "https://auth.openai.com";
const COPILOT_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

pub fn start_openai_browser_oauth(redirect_uri: &str) -> Result<OAuthAuthorizeStart, AppError> {
    let client = BasicClient::new(
        ClientId::new(OPENAI_CLIENT_ID.to_owned()),
        None,
        AuthUrl::new(format!("{OPENAI_ISSUER}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid openai auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{OPENAI_ISSUER}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid openai token url: {e}")))?,
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|e| AppError::Config(format!("invalid redirect uri: {e}")))?,
    );

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
    let client = BasicClient::new(
        ClientId::new(OPENAI_CLIENT_ID.to_owned()),
        None,
        AuthUrl::new(format!("{OPENAI_ISSUER}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid openai auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{OPENAI_ISSUER}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid openai token url: {e}")))?,
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|e| AppError::Config(format!("invalid redirect uri: {e}")))?,
    );

    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AppError::Provider(format!("openai oauth token exchange failed: {e}")))?;

    let claims_account = extract_openai_account_id(token.access_token().secret());

    let expires_at_ms = token
        .expires_in()
        .map(|d| chrono::Utc::now().timestamp_millis() + d.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|x| x.secret().to_owned())
            .unwrap_or_default(),
        access: token.access_token().secret().to_owned(),
        expires_at_ms,
        account_id: claims_account,
    })
}

pub async fn refresh_openai_token(refresh_token: &str) -> Result<OAuthTokenResponse, AppError> {
    let client = BasicClient::new(
        ClientId::new(OPENAI_CLIENT_ID.to_owned()),
        None,
        AuthUrl::new(format!("{OPENAI_ISSUER}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid openai auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{OPENAI_ISSUER}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid openai token url: {e}")))?,
        ),
    );

    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AppError::Provider(format!("openai oauth token refresh failed: {e}")))?;

    let expires_at_ms = token
        .expires_in()
        .map(|d| chrono::Utc::now().timestamp_millis() + d.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|x| x.secret().to_owned())
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
        .map(|seconds| chrono::Utc::now().timestamp_millis() + (seconds as i64 * 1000))
        .unwrap_or(0);

    Ok(Some(OAuthTokenResponse {
        refresh: refresh_token,
        access: token_data.access_token.clone(),
        expires_at_ms,
        account_id: extract_openai_account_id(token_data.access_token.as_str()),
    }))
}

pub async fn start_copilot_device_code(domain: &str) -> Result<DeviceCodeStart, AppError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/device/code");

    #[derive(Debug, Deserialize)]
    struct DeviceCodeResponse {
        verification_uri: String,
        user_code: String,
        device_code: String,
        interval: Option<u64>,
    }

    let response = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "scope": "read:user",
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
            provider: "github-copilot".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let data: DeviceCodeResponse = response.json().await?;
    Ok(DeviceCodeStart {
        verification_url: data.verification_uri,
        user_code: data.user_code,
        device_code: data.device_code,
        interval_seconds: data.interval.unwrap_or(5),
    })
}

pub async fn poll_copilot_device_code(
    domain: &str,
    device_code: &str,
) -> Result<Option<OAuthTokenResponse>, AppError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/oauth/access_token");

    #[derive(Debug, Deserialize)]
    struct PollResult {
        access_token: Option<String>,
        error: Option<String>,
    }

    let response = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
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
            provider: "github-copilot".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let data: PollResult = response.json().await?;
    if let Some(token) = data.access_token {
        return Ok(Some(OAuthTokenResponse {
            refresh: token.clone(),
            access: token,
            expires_at_ms: 0,
            account_id: None,
        }));
    }

    match data.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some(err) => Err(AppError::Provider(format!(
            "github copilot device oauth failed: {err}"
        ))),
        None => Ok(None),
    }
}

fn parse_device_auth_interval(raw: Option<serde_json::Value>, default_seconds: u64) -> u64 {
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

pub fn start_gitlab_oauth(
    instance_url: &str,
    redirect_uri: &str,
) -> Result<OAuthAuthorizeStart, AppError> {
    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| AppError::Config("GITLAB_CLIENT_ID is not set".to_owned()))?;

    let client = BasicClient::new(
        ClientId::new(client_id),
        None,
        AuthUrl::new(format!("{instance}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid gitlab auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{instance}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid gitlab token url: {e}")))?,
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|e| AppError::Config(format!("invalid redirect uri: {e}")))?,
    );

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read_user".to_owned()))
        .set_pkce_challenge(challenge)
        .url();

    Ok(OAuthAuthorizeStart {
        authorize_url: url.to_string(),
        state: state.secret().to_owned(),
        pkce_verifier: verifier.secret().to_owned(),
    })
}

pub async fn exchange_gitlab_oauth_code(
    instance_url: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse, AppError> {
    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| AppError::Config("GITLAB_CLIENT_ID is not set".to_owned()))?;
    let client_secret = std::env::var("GITLAB_CLIENT_SECRET").ok();

    let client = BasicClient::new(
        ClientId::new(client_id),
        client_secret.map(oauth2::ClientSecret::new),
        AuthUrl::new(format!("{instance}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid gitlab auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{instance}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid gitlab token url: {e}")))?,
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|e| AppError::Config(format!("invalid redirect uri: {e}")))?,
    );

    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AppError::Provider(format!("gitlab oauth token exchange failed: {e}")))?;

    let expires_at_ms = token
        .expires_in()
        .map(|d| chrono::Utc::now().timestamp_millis() + d.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|x| x.secret().to_owned())
            .unwrap_or_default(),
        access: token.access_token().secret().to_owned(),
        expires_at_ms,
        account_id: None,
    })
}

pub async fn refresh_gitlab_token(
    instance_url: &str,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, AppError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AppError::Config(
            "gitlab oauth refresh token is empty".to_owned(),
        ));
    }

    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| AppError::Config("GITLAB_CLIENT_ID is not set".to_owned()))?;
    let client_secret = std::env::var("GITLAB_CLIENT_SECRET").ok();

    let client = BasicClient::new(
        ClientId::new(client_id),
        client_secret.map(oauth2::ClientSecret::new),
        AuthUrl::new(format!("{instance}/oauth/authorize"))
            .map_err(|e| AppError::Config(format!("invalid gitlab auth url: {e}")))?,
        Some(
            TokenUrl::new(format!("{instance}/oauth/token"))
                .map_err(|e| AppError::Config(format!("invalid gitlab token url: {e}")))?,
        ),
    );

    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AppError::Provider(format!("gitlab oauth token refresh failed: {e}")))?;

    let expires_at_ms = token
        .expires_in()
        .map(|d| chrono::Utc::now().timestamp_millis() + d.as_millis() as i64)
        .unwrap_or(0);

    Ok(OAuthTokenResponse {
        refresh: token
            .refresh_token()
            .map(|x| x.secret().to_owned())
            .unwrap_or_else(|| refresh_token.to_owned()),
        access: token.access_token().secret().to_owned(),
        expires_at_ms,
        account_id: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallback {
    pub code: String,
    pub state: String,
}

pub fn wait_for_oauth_callback(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> Result<OAuthCallback, AppError> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| AppError::Config(format!("failed to bind oauth callback port {port}: {e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Config(format!("failed to set oauth callback nonblocking: {e}")))?;

    let started = Instant::now();
    while started.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf)?;
                let request = String::from_utf8_lossy(&buf[..n]);

                let first_line = request.lines().next().unwrap_or_default();
                let path = first_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_owned();

                let url = format!("http://localhost:{port}{path}");
                let parsed = url::Url::parse(url.as_str())
                    .map_err(|e| AppError::Config(format!("invalid oauth callback url: {e}")))?;

                if let Some(err) = parsed.query_pairs().find(|(k, _)| k == "error") {
                    let body = oauth_html_error(err.1.as_ref());
                    write_http_html(&mut stream, 400, body.as_str())?;
                    return Err(AppError::Provider(format!(
                        "oauth callback failed: {}",
                        err.1
                    )));
                }

                let code = parsed
                    .query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.to_string())
                    .ok_or_else(|| AppError::Provider("oauth callback missing code".to_owned()))?;
                let state = parsed
                    .query_pairs()
                    .find(|(k, _)| k == "state")
                    .map(|(_, v)| v.to_string())
                    .ok_or_else(|| AppError::Provider("oauth callback missing state".to_owned()))?;

                if state != expected_state {
                    write_http_html(&mut stream, 400, oauth_html_error("Invalid state").as_str())?;
                    return Err(AppError::Provider(
                        "oauth callback state mismatch (potential csrf)".to_owned(),
                    ));
                }

                write_http_html(&mut stream, 200, oauth_html_success())?;
                return Ok(OAuthCallback { code, state });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(AppError::Io(err)),
        }
    }

    Err(AppError::Provider("oauth callback timeout".to_owned()))
}

fn normalize_domain(url_or_domain: &str) -> String {
    url_or_domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

fn extract_openai_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    json.get("chatgpt_account_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|v| v.get("chatgpt_account_id"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            json.get("organizations")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|x| x.get("id"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
}

fn write_http_html(stream: &mut impl Write, status: u16, html: &str) -> Result<(), AppError> {
    let status_line = if status == 200 {
        "HTTP/1.1 200 OK"
    } else {
        "HTTP/1.1 400 Bad Request"
    };
    let body = html.as_bytes();
    let response = format!(
        "{status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn oauth_html_success() -> &'static str {
    "<!doctype html><html><body><h1>Authorization Successful</h1><p>You can close this window.</p><script>setTimeout(() => window.close(), 1500)</script></body></html>"
}

fn oauth_html_error(error: &str) -> String {
    format!(
        "<!doctype html><html><body><h1>Authorization Failed</h1><p>{}</p></body></html>",
        error
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_device_auth_interval_supports_number_and_string() {
        assert_eq!(parse_device_auth_interval(Some(serde_json::json!(7)), 5), 7);
        assert_eq!(
            parse_device_auth_interval(Some(serde_json::json!("9")), 5),
            9
        );
        assert_eq!(
            parse_device_auth_interval(Some(serde_json::json!("0")), 5),
            1
        );
        assert_eq!(parse_device_auth_interval(None, 5), 5);
    }

    #[test]
    fn normalize_domain_strips_protocol_and_slash() {
        assert_eq!(
            normalize_domain("https://github.example.com/"),
            "github.example.com"
        );
        assert_eq!(normalize_domain("http://gitlab.local"), "gitlab.local");
    }
}
