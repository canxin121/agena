use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use crate::error::AppError;

use super::{
    AuthData, AuthStore, CopilotDeployment, CredentialIssuer, DeviceCodeStart, OAuthAuthorizeStart,
    OAuthCallback, OAuthTokenResponse, OAuthUserInfo, exchange_gitlab_oauth_code,
    exchange_openai_oauth_code, poll_copilot_device_code, poll_openai_headless_device_code,
    refresh_gitlab_token, refresh_openai_token, revoke_openai_token, start_copilot_device_code,
    start_gitlab_oauth, start_openai_browser_oauth, start_openai_headless_device_code,
    wait_for_oauth_callback,
};

struct StoredOAuthCredential {
    refresh: String,
    id_token: Option<String>,
    account_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
    enterprise_url: Option<String>,
    user: Option<OAuthUserInfo>,
}

pub struct AuthManager<S: AuthStore> {
    store: S,
}

impl<S: AuthStore> AuthManager<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        self.store.all()
    }

    pub fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        self.store.get(provider_id)
    }

    pub fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        self.store.remove(provider_id)
    }

    pub async fn logout(&self, provider_id: &str) -> Result<Option<String>, AppError> {
        let stored = self.store.get(provider_id)?;
        let revoke_target = match stored.as_ref() {
            Some(auth) if should_revoke_openai_oauth(provider_id, auth) => match auth {
                AuthData::OAuth {
                    refresh, access, ..
                } => Some((refresh.clone(), access.clone())),
                _ => None,
            },
            None => None,
            _ => None,
        };

        let revoke_warning = match revoke_target {
            Some((refresh, access)) => revoke_openai_token(refresh.as_str(), access.as_str())
                .await
                .err()
                .map(|err| err.to_string()),
            None => None,
        };

        self.store.remove(provider_id)?;
        Ok(revoke_warning)
    }

    pub fn set_auth_data(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        validate_auth_data(provider_id, &auth)?;
        self.store.set(provider_id, auth)
    }

    pub fn set_api_key(&self, provider_id: &str, key: impl Into<String>) -> Result<(), AppError> {
        let key = key.into();
        let key = key.trim();
        if key.is_empty() {
            return Err(AppError::Config(format!(
                "{provider_id} api key cannot be empty"
            )));
        }

        self.store.set(
            provider_id,
            AuthData::Api {
                key: key.to_owned(),
            },
        )
    }

    pub fn set_anthropic_api_key(&self, key: impl Into<String>) -> Result<(), AppError> {
        self.set_api_key("anthropic", key)
    }

    pub fn set_openai_api_key(&self, key: impl Into<String>) -> Result<(), AppError> {
        self.set_api_key("openai", key)
    }

    pub fn start_openai_browser_login(
        &self,
        redirect_uri: impl Into<String>,
    ) -> Result<OAuthAuthorizeStart, AppError> {
        start_openai_browser_oauth(redirect_uri.into().as_str())
    }

    pub async fn finish_openai_browser_login(
        &self,
        provider_id: &str,
        code: impl Into<String>,
        pkce_verifier: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Result<AuthData, AppError> {
        let code = code.into();
        let pkce_verifier = pkce_verifier.into();
        let redirect_uri = redirect_uri.into();
        let token = exchange_openai_oauth_code(
            code.as_str(),
            pkce_verifier.as_str(),
            redirect_uri.as_str(),
        )
        .await?;
        let auth = oauth_auth_data(
            provider_id,
            CredentialIssuer::OpenaiChatgpt,
            token.refresh,
            token.access,
            token.id_token,
            token.expires_at_ms,
            token.account_id,
            token.chatgpt_account_is_fedramp,
            None,
        )?;
        self.persist_auth(provider_id, auth)
    }

    pub async fn openai_browser_login_auto(
        &self,
        provider_id: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<(String, AuthData), AppError> {
        let redirect_uri = browser_login_redirect_uri(port);
        let start = self.start_openai_browser_login(redirect_uri.clone())?;
        self.complete_browser_login_auto(
            port,
            timeout,
            redirect_uri,
            start,
            |callback, pkce_verifier, redirect_uri| {
                self.finish_openai_browser_login(
                    provider_id,
                    callback.code,
                    pkce_verifier,
                    redirect_uri,
                )
            },
        )
        .await
    }

    pub async fn start_openai_headless_login(&self) -> Result<DeviceCodeStart, AppError> {
        start_openai_headless_device_code().await
    }

    pub async fn poll_openai_headless_login(
        &self,
        provider_id: &str,
        device_code: impl Into<String>,
        user_code: impl Into<String>,
    ) -> Result<Option<AuthData>, AppError> {
        let device_code = device_code.into();
        let user_code = user_code.into();
        let token =
            poll_openai_headless_device_code(device_code.as_str(), user_code.as_str()).await?;
        let Some(token) = token else {
            return Ok(None);
        };

        let auth = oauth_auth_data(
            provider_id,
            CredentialIssuer::OpenaiChatgpt,
            token.refresh,
            token.access,
            token.id_token,
            token.expires_at_ms,
            token.account_id,
            token.chatgpt_account_is_fedramp,
            None,
        )?;
        self.persist_auth(provider_id, auth).map(Some)
    }

    pub async fn refresh_openai_login(&self, provider_id: &str) -> Result<AuthData, AppError> {
        let stored = self.stored_oauth_credential(provider_id)?;
        let token = refresh_openai_token(stored.refresh.as_str()).await?;
        let auth = oauth_auth_data_with_user(
            provider_id,
            CredentialIssuer::OpenaiChatgpt,
            token.refresh,
            token.access,
            token.id_token.or(stored.id_token),
            token.expires_at_ms,
            token.account_id,
            token.chatgpt_account_is_fedramp || stored.chatgpt_account_is_fedramp,
            stored.enterprise_url,
            stored.user,
        )?;
        self.persist_auth(provider_id, auth)
    }

    pub async fn start_copilot_login(
        &self,
        deployment: CopilotDeployment,
    ) -> Result<DeviceCodeStart, AppError> {
        let domain = match deployment {
            CopilotDeployment::GitHubCom => "github.com".to_owned(),
            CopilotDeployment::Enterprise { domain } => domain,
        };
        start_copilot_device_code(domain.as_str()).await
    }

    pub async fn poll_copilot_login(
        &self,
        provider_id: &str,
        device_code: impl Into<String>,
        deployment: CopilotDeployment,
    ) -> Result<Option<AuthData>, AppError> {
        let device_code = device_code.into();
        let (domain, enterprise_url) = match deployment {
            CopilotDeployment::GitHubCom => ("github.com".to_owned(), None),
            CopilotDeployment::Enterprise { domain } => {
                let normalized = normalize_domain(domain.as_str());
                (normalized.clone(), Some(normalized))
            }
        };

        let token = poll_copilot_device_code(domain.as_str(), device_code.as_str()).await?;
        let Some(token) = token else {
            return Ok(None);
        };

        let auth = oauth_auth_data(
            provider_id,
            CredentialIssuer::GithubCopilot,
            token.refresh,
            token.access,
            token.id_token,
            token.expires_at_ms,
            None,
            token.chatgpt_account_is_fedramp,
            enterprise_url,
        )?;
        self.persist_auth(provider_id, auth).map(Some)
    }

    pub fn start_gitlab_login(
        &self,
        instance_url: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Result<OAuthAuthorizeStart, AppError> {
        start_gitlab_oauth(instance_url.into().as_str(), redirect_uri.into().as_str())
    }

    pub async fn finish_gitlab_login(
        &self,
        provider_id: &str,
        instance_url: impl Into<String>,
        code: impl Into<String>,
        pkce_verifier: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Result<AuthData, AppError> {
        let instance = instance_url.into();
        let code = code.into();
        let pkce_verifier = pkce_verifier.into();
        let redirect_uri = redirect_uri.into();
        let token: OAuthTokenResponse = exchange_gitlab_oauth_code(
            instance.as_str(),
            code.as_str(),
            pkce_verifier.as_str(),
            redirect_uri.as_str(),
        )
        .await?;

        let auth = oauth_auth_data(
            provider_id,
            CredentialIssuer::Gitlab,
            token.refresh,
            token.access,
            token.id_token,
            token.expires_at_ms,
            None,
            token.chatgpt_account_is_fedramp,
            None,
        )?;
        self.persist_auth(provider_id, auth)
    }

    pub async fn refresh_gitlab_login(
        &self,
        provider_id: &str,
        instance_url: impl Into<String>,
    ) -> Result<AuthData, AppError> {
        let stored = self.stored_oauth_credential(provider_id)?;
        let instance_url = instance_url.into();
        let token = refresh_gitlab_token(instance_url.as_str(), stored.refresh.as_str()).await?;
        let auth = oauth_auth_data_with_user(
            provider_id,
            CredentialIssuer::Gitlab,
            token.refresh,
            token.access,
            token.id_token.or(stored.id_token),
            token.expires_at_ms,
            stored.account_id,
            token.chatgpt_account_is_fedramp || stored.chatgpt_account_is_fedramp,
            None,
            stored.user,
        )?;
        self.persist_auth(provider_id, auth)
    }

    pub async fn gitlab_browser_login_auto(
        &self,
        provider_id: &str,
        instance_url: impl Into<String>,
        port: u16,
        timeout: Duration,
    ) -> Result<(String, AuthData), AppError> {
        let instance_url = instance_url.into();
        let redirect_uri = browser_login_redirect_uri(port);
        let start = self.start_gitlab_login(instance_url.clone(), redirect_uri.clone())?;
        self.complete_browser_login_auto(
            port,
            timeout,
            redirect_uri,
            start,
            |callback, pkce_verifier, redirect_uri| {
                self.finish_gitlab_login(
                    provider_id,
                    instance_url,
                    callback.code,
                    pkce_verifier,
                    redirect_uri,
                )
            },
        )
        .await
    }

    fn persist_auth(&self, provider_id: &str, auth: AuthData) -> Result<AuthData, AppError> {
        self.store.set(provider_id, auth.clone())?;
        Ok(auth)
    }

    fn stored_oauth_credential(
        &self,
        provider_id: &str,
    ) -> Result<StoredOAuthCredential, AppError> {
        let Some(AuthData::OAuth {
            refresh,
            id_token,
            account_id,
            chatgpt_account_is_fedramp,
            enterprise_url,
            user,
            ..
        }) = self.store.get(provider_id)?
        else {
            return Err(missing_oauth_credential_error(provider_id));
        };

        Ok(StoredOAuthCredential {
            refresh,
            id_token,
            account_id,
            chatgpt_account_is_fedramp,
            enterprise_url,
            user,
        })
    }

    async fn complete_browser_login_auto<F, Fut>(
        &self,
        port: u16,
        timeout: Duration,
        redirect_uri: String,
        start: OAuthAuthorizeStart,
        finish: F,
    ) -> Result<(String, AuthData), AppError>
    where
        F: FnOnce(OAuthCallback, String, String) -> Fut,
        Fut: Future<Output = Result<AuthData, AppError>>,
    {
        let authorize_url = start.authorize_url.clone();
        let callback = wait_for_oauth_callback(port, start.state.as_str(), timeout)?;
        let auth = finish(callback, start.pkce_verifier, redirect_uri).await?;
        Ok((authorize_url, auth))
    }
}

fn should_revoke_openai_oauth(provider_id: &str, auth: &AuthData) -> bool {
    matches!(
        auth,
        AuthData::OAuth {
            issuer: Some(CredentialIssuer::OpenaiChatgpt),
            ..
        }
    ) || matches!(auth, AuthData::OAuth { issuer: None, .. } if provider_id == "openai")
}

fn oauth_auth_data(
    provider_id: &str,
    issuer: CredentialIssuer,
    refresh: String,
    access: String,
    id_token: Option<String>,
    expires_at_ms: i64,
    account_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
    enterprise_url: Option<String>,
) -> Result<AuthData, AppError> {
    oauth_auth_data_with_user(
        provider_id,
        issuer,
        refresh,
        access,
        id_token,
        expires_at_ms,
        account_id,
        chatgpt_account_is_fedramp,
        enterprise_url,
        None,
    )
}

fn oauth_auth_data_with_user(
    provider_id: &str,
    issuer: CredentialIssuer,
    refresh: String,
    access: String,
    id_token: Option<String>,
    expires_at_ms: i64,
    account_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
    enterprise_url: Option<String>,
    user: Option<OAuthUserInfo>,
) -> Result<AuthData, AppError> {
    let auth = AuthData::OAuth {
        issuer: Some(issuer),
        refresh,
        access,
        id_token,
        expires_at_ms,
        account_id,
        chatgpt_account_is_fedramp,
        enterprise_url,
        user,
    };
    validate_auth_data(provider_id, &auth)?;
    Ok(auth)
}

fn validate_auth_data(provider_id: &str, auth: &AuthData) -> Result<(), AppError> {
    match auth {
        AuthData::Api { key } => {
            if key.trim().is_empty() {
                return Err(AppError::Config(format!(
                    "{provider_id} api key cannot be empty"
                )));
            }
        }
        AuthData::OAuth {
            issuer,
            refresh,
            access,
            user,
            ..
        } => {
            if issuer.is_none() {
                return Err(AppError::Config(format!(
                    "{provider_id} oauth credential must include an issuer"
                )));
            }
            if refresh.trim().is_empty() {
                return Err(AppError::Config(format!(
                    "{provider_id} oauth refresh token cannot be empty"
                )));
            }
            if access.trim().is_empty() {
                return Err(AppError::Config(format!(
                    "{provider_id} oauth access token cannot be empty"
                )));
            }
            if let Some(user) = user {
                if user.id.trim().is_empty() {
                    return Err(AppError::Config(format!(
                        "{provider_id} oauth user id cannot be empty"
                    )));
                }
                if user.username.trim().is_empty() {
                    return Err(AppError::Config(format!(
                        "{provider_id} oauth username cannot be empty"
                    )));
                }
            }
        }
        AuthData::WellKnown { key, token } => {
            let _ = token;
            if key.trim().is_empty() {
                return Err(AppError::Config(format!(
                    "{provider_id} well-known key cannot be empty"
                )));
            }
        }
    }
    Ok(())
}

fn browser_login_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

fn missing_oauth_credential_error(provider_id: &str) -> AppError {
    AppError::Config(format!("{provider_id} oauth credential not found"))
}

fn normalize_domain(url_or_domain: &str) -> String {
    let trimmed = url_or_domain.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    without_scheme.trim_end_matches('/').to_owned()
}
