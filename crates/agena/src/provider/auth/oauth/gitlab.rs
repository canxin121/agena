use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse, TokenUrl, basic::BasicClient,
};

use crate::error::AppError;

use super::super::{OAuthAuthorizeStart, OAuthTokenResponse};

pub fn start_gitlab_oauth(
    instance_url: &str,
    redirect_uri: &str,
) -> Result<OAuthAuthorizeStart, AppError> {
    let client = gitlab_oauth_client(instance_url, redirect_uri, false)?;

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
    let client = gitlab_oauth_client(instance_url, redirect_uri, true)?;

    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|error| {
            AppError::Provider(format!("gitlab oauth token exchange failed: {error}"))
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

    let client = gitlab_oauth_client(instance_url, "http://localhost", true)?;
    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_owned()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|error| {
            AppError::Provider(format!("gitlab oauth token refresh failed: {error}"))
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
        account_id: None,
    })
}

fn gitlab_oauth_client(
    instance_url: &str,
    redirect_uri: &str,
    include_secret: bool,
) -> Result<BasicClient, AppError> {
    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| AppError::Config("GITLAB_CLIENT_ID is not set".to_owned()))?;
    let client_secret = include_secret
        .then(|| std::env::var("GITLAB_CLIENT_SECRET").ok())
        .flatten();

    Ok(BasicClient::new(
        ClientId::new(client_id),
        client_secret.map(oauth2::ClientSecret::new),
        AuthUrl::new(format!("{instance}/oauth/authorize"))
            .map_err(|error| AppError::Config(format!("invalid gitlab auth url: {error}")))?,
        Some(
            TokenUrl::new(format!("{instance}/oauth/token"))
                .map_err(|error| AppError::Config(format!("invalid gitlab token url: {error}")))?,
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|error| AppError::Config(format!("invalid redirect uri: {error}")))?,
    ))
}
