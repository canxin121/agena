use crate::ProviderError;
use agena_provider::OAuthAuthorizeStart;

use super::shared::{
    exchange_oauth_code, oauth_authorize_start, oauth_client, oauth_http_client,
    refresh_oauth_token,
};
use agena_provider::OAuthTokenResponse;

pub fn start_gitlab_oauth(
    instance_url: &str,
    redirect_uri: &str,
) -> Result<OAuthAuthorizeStart, ProviderError> {
    let client = gitlab_oauth_client(instance_url, redirect_uri, false)?;

    Ok(oauth_authorize_start(&client, &["read_user"], &[]))
}

pub async fn exchange_gitlab_oauth_code(
    instance_url: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse, ProviderError> {
    let client = gitlab_oauth_client(instance_url, redirect_uri, true)?;

    exchange_oauth_code(
        client,
        code,
        pkce_verifier,
        oauth_http_client(false),
        "gitlab oauth token exchange failed",
        |_| None,
    )
    .await
}

pub async fn refresh_gitlab_token(
    instance_url: &str,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, ProviderError> {
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(ProviderError::Config(
            "gitlab oauth refresh token is empty".to_owned(),
        ));
    }

    let client = gitlab_oauth_client(instance_url, "http://localhost", true)?;
    refresh_oauth_token(
        client,
        refresh_token,
        oauth_http_client(false),
        "gitlab oauth token refresh failed",
        |_| None,
    )
    .await
}

fn gitlab_oauth_client(
    instance_url: &str,
    redirect_uri: &str,
    include_secret: bool,
) -> Result<super::shared::OAuthClient, ProviderError> {
    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| ProviderError::Config("GITLAB_CLIENT_ID is not set".to_owned()))?;
    let client_secret = if include_secret {
        let client_secret = std::env::var("GITLAB_CLIENT_SECRET")
            .map_err(|_| ProviderError::Config("GITLAB_CLIENT_SECRET is not set".to_owned()))?;
        let client_secret = client_secret.trim();
        if client_secret.is_empty() {
            return Err(ProviderError::Config(
                "GITLAB_CLIENT_SECRET cannot be empty".to_owned(),
            ));
        }
        Some(client_secret.to_owned())
    } else {
        None
    };

    oauth_client(
        "gitlab",
        client_id,
        client_secret,
        format!("{instance}/oauth/authorize"),
        format!("{instance}/oauth/token"),
        Some(redirect_uri),
    )
}
