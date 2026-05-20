use std::time::Duration;

use serde::Deserialize;

use crate::error::{AppError, ProviderErrorKind};

use super::super::{OAuthAuthorizeStart, OAuthTokenResponse, OAuthUserInfo};

const PLATFORM_LOGIN_URL: &str = "https://acs.atomgit.com/auth/login";
const PLATFORM_CHECK_URL: &str = "https://acs.atomgit.com/auth/check";
const PLATFORM_TOKEN_URL: &str = "https://acs.atomgit.com/auth/token";
const PLATFORM_REFRESH_URL: &str = "https://acs.atomgit.com/oauth/refresh";
const ATOMGIT_USER_AGENT: &str = crate::provider::ATOMCODE_USER_AGENT;

#[derive(Debug, Deserialize)]
struct PlatformLoginResponse {
    login_url: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct PlatformCheckResponse {
    valid: bool,
}

#[derive(Debug, Deserialize)]
struct PlatformUserInfo {
    id: String,
    username: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlatformTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    user: PlatformUserInfo,
}

#[derive(Debug, Deserialize)]
struct PlatformRefreshResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    user: Option<PlatformUserInfo>,
}

pub async fn start_atomgit_oauth() -> Result<OAuthAuthorizeStart, AppError> {
    let response = atomgit_client()?
        .get(format!("{PLATFORM_LOGIN_URL}?provider=atomgit"))
        .send()
        .await?;

    let response = ensure_success("auth/login", response).await?;
    let data: PlatformLoginResponse = response.json().await?;

    Ok(OAuthAuthorizeStart {
        authorize_url: strip_force_login(data.login_url.as_str()),
        state: data.state,
        pkce_verifier: String::new(),
    })
}

pub async fn poll_atomgit_oauth_state(state: &str) -> Result<bool, AppError> {
    let state = state.trim();
    if state.is_empty() {
        return Err(AppError::Config("atomgit oauth state is empty".to_owned()));
    }

    let response = atomgit_client()?
        .get(format!(
            "{PLATFORM_CHECK_URL}?state={}",
            urlencoding::encode(state)
        ))
        .send()
        .await?;

    if !response.status().is_success() {
        return Ok(false);
    }

    Ok(response
        .json::<PlatformCheckResponse>()
        .await
        .map(|check| check.valid)
        .unwrap_or(false))
}

pub async fn exchange_atomgit_oauth_state(state: &str) -> Result<OAuthTokenResponse, AppError> {
    let state = state.trim();
    if state.is_empty() {
        return Err(AppError::Config("atomgit oauth state is empty".to_owned()));
    }

    let response = atomgit_client()?
        .get(format!(
            "{PLATFORM_TOKEN_URL}?state={}",
            urlencoding::encode(state)
        ))
        .send()
        .await?;

    let response = ensure_success("auth/token", response).await?;
    let data: PlatformTokenResponse = response.json().await?;
    let user = user_info(data.user);

    Ok(OAuthTokenResponse {
        refresh: data.refresh_token.unwrap_or_default(),
        access: data.access_token,
        expires_at_ms: expires_at_ms(data.expires_in),
        account_id: Some(user.id.clone()),
        user: Some(user),
    })
}

pub async fn refresh_atomgit_token(refresh_token: &str) -> Result<OAuthTokenResponse, AppError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AppError::Config(
            "atomgit oauth refresh token is empty".to_owned(),
        ));
    }

    let response = atomgit_client()?
        .post(PLATFORM_REFRESH_URL)
        .json(&serde_json::json!({
            "refresh_token": refresh_token,
            "provider": "atomgit",
        }))
        .send()
        .await?;

    let response = ensure_success("oauth/refresh", response).await?;
    let data: PlatformRefreshResponse = response.json().await?;
    let user = data.user.map(user_info);

    Ok(OAuthTokenResponse {
        refresh: data
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_owned()),
        access: data.access_token,
        expires_at_ms: expires_at_ms(data.expires_in),
        account_id: user.as_ref().map(|user| user.id.clone()),
        user,
    })
}

fn atomgit_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent(ATOMGIT_USER_AGENT)
        .build()
        .map_err(AppError::from)
}

async fn ensure_success(
    endpoint: &str,
    response: reqwest::Response,
) -> Result<reqwest::Response, AppError> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(AppError::HttpStatus {
        provider: "atomgit".to_owned(),
        status,
        body: format!("{endpoint}: {body}"),
        kind: ProviderErrorKind::ApiError,
        retryable: false,
    })
}

fn user_info(user: PlatformUserInfo) -> OAuthUserInfo {
    OAuthUserInfo {
        id: user.id,
        username: user.username,
        name: user.name,
        email: user.email,
        avatar_url: user.avatar_url,
    }
}

fn expires_at_ms(expires_in: Option<i64>) -> i64 {
    expires_in
        .filter(|seconds| *seconds > 0)
        .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds * 1_000)
        .unwrap_or(0)
}

fn strip_force_login(url: &str) -> String {
    url.replace("&force_login=true", "")
        .replace("?force_login=true&", "?")
        .replace("?force_login=true", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_force_login_removes_query_flag() {
        assert_eq!(
            strip_force_login("https://atomgit.com/oauth/authorize?state=s&force_login=true"),
            "https://atomgit.com/oauth/authorize?state=s"
        );
        assert_eq!(
            strip_force_login("https://atomgit.com/oauth/authorize?force_login=true&state=s"),
            "https://atomgit.com/oauth/authorize?state=s"
        );
        assert_eq!(
            strip_force_login("https://atomgit.com/oauth/authorize?state=s&force_login=true&x=1"),
            "https://atomgit.com/oauth/authorize?state=s&x=1"
        );
    }
}
