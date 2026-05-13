use std::collections::HashMap;
use std::time::Duration;

use crate::error::AppError;

use super::{
    AuthData, AuthStore, CopilotDeployment, DeviceCodeStart, OAuthAuthorizeStart,
    OAuthTokenResponse, exchange_gitlab_oauth_code, exchange_openai_oauth_code,
    poll_copilot_device_code, poll_openai_headless_device_code, refresh_gitlab_token,
    refresh_openai_token, start_copilot_device_code, start_gitlab_oauth,
    start_openai_browser_oauth, start_openai_headless_device_code, wait_for_oauth_callback,
};

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
            token.refresh,
            token.access,
            token.expires_at_ms,
            token.account_id,
            None,
        )?;
        self.store.set(provider_id, auth.clone())?;
        Ok(auth)
    }

    pub async fn openai_browser_login_auto(
        &self,
        provider_id: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<(String, AuthData), AppError> {
        let redirect_uri = format!("http://localhost:{port}/auth/callback");
        let start = self.start_openai_browser_login(redirect_uri.clone())?;
        let callback = wait_for_oauth_callback(port, start.state.as_str(), timeout)?;
        let auth = self
            .finish_openai_browser_login(
                provider_id,
                callback.code,
                start.pkce_verifier,
                redirect_uri,
            )
            .await?;
        Ok((start.authorize_url, auth))
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
            token.refresh,
            token.access,
            token.expires_at_ms,
            token.account_id,
            None,
        )?;
        self.store.set(provider_id, auth.clone())?;
        Ok(Some(auth))
    }

    pub async fn refresh_openai_login(&self, provider_id: &str) -> Result<AuthData, AppError> {
        let Some(AuthData::OAuth {
            refresh,
            enterprise_url,
            ..
        }) = self.store.get(provider_id)?
        else {
            return Err(AppError::Config(format!(
                "{provider_id} oauth credential not found"
            )));
        };

        let token = refresh_openai_token(refresh.as_str()).await?;
        let auth = oauth_auth_data(
            provider_id,
            token.refresh,
            token.access,
            token.expires_at_ms,
            token.account_id,
            enterprise_url,
        )?;
        self.store.set(provider_id, auth.clone())?;
        Ok(auth)
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
            token.refresh,
            token.access,
            token.expires_at_ms,
            None,
            enterprise_url,
        )?;
        self.store.set(provider_id, auth.clone())?;
        Ok(Some(auth))
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
            token.refresh,
            token.access,
            token.expires_at_ms,
            None,
            None,
        )?;
        let _ = instance;
        self.store.set(provider_id, auth.clone())?;
        Ok(auth)
    }

    pub async fn refresh_gitlab_login(
        &self,
        provider_id: &str,
        instance_url: impl Into<String>,
    ) -> Result<AuthData, AppError> {
        let Some(AuthData::OAuth {
            refresh,
            account_id,
            ..
        }) = self.store.get(provider_id)?
        else {
            return Err(AppError::Config(format!(
                "{provider_id} oauth credential not found"
            )));
        };

        let instance_url = instance_url.into();

        let token = refresh_gitlab_token(instance_url.as_str(), refresh.as_str()).await?;
        let auth = oauth_auth_data(
            provider_id,
            token.refresh,
            token.access,
            token.expires_at_ms,
            account_id,
            None,
        )?;
        self.store.set(provider_id, auth.clone())?;
        Ok(auth)
    }

    pub async fn gitlab_browser_login_auto(
        &self,
        provider_id: &str,
        instance_url: impl Into<String>,
        port: u16,
        timeout: Duration,
    ) -> Result<(String, AuthData), AppError> {
        let instance_url = instance_url.into();
        let redirect_uri = format!("http://localhost:{port}/auth/callback");
        let start = self.start_gitlab_login(instance_url.clone(), redirect_uri.clone())?;
        let callback = wait_for_oauth_callback(port, start.state.as_str(), timeout)?;
        let auth = self
            .finish_gitlab_login(
                provider_id,
                instance_url,
                callback.code,
                start.pkce_verifier,
                redirect_uri,
            )
            .await?;
        Ok((start.authorize_url, auth))
    }
}

fn oauth_auth_data(
    provider_id: &str,
    refresh: String,
    access: String,
    expires_at_ms: i64,
    account_id: Option<String>,
    enterprise_url: Option<String>,
) -> Result<AuthData, AppError> {
    let auth = AuthData::OAuth {
        refresh,
        access,
        expires_at_ms,
        account_id,
        enterprise_url,
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
            refresh, access, ..
        } => {
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

fn normalize_domain(url_or_domain: &str) -> String {
    let trimmed = url_or_domain.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    without_scheme.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        data: Mutex<HashMap<String, AuthData>>,
    }

    impl AuthStore for MemoryStore {
        fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
            Ok(self
                .data
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .clone())
        }

        fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
            Ok(self
                .data
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .get(provider_id)
                .cloned())
        }

        fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
            self.data
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .insert(provider_id.to_owned(), auth);
            Ok(())
        }

        fn remove(&self, provider_id: &str) -> Result<(), AppError> {
            self.data
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .remove(provider_id);
            Ok(())
        }
    }

    #[test]
    fn set_api_key_rejects_empty_value() {
        let manager = AuthManager::new(MemoryStore::default());
        let err = manager
            .set_api_key("openai", "   ")
            .expect_err("empty api key should be rejected");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn set_api_key_trims_and_persists_value() {
        let manager = AuthManager::new(MemoryStore::default());
        manager
            .set_api_key("openai", "  sk-test  ")
            .expect("api key should be stored");

        let stored = manager
            .get("openai")
            .expect("store read should succeed")
            .expect("openai auth should exist");

        match stored {
            AuthData::Api { key } => assert_eq!(key, "sk-test"),
            other => panic!("unexpected auth variant: {other:?}"),
        }
    }

    #[test]
    fn normalize_domain_strips_protocol_and_slash() {
        assert_eq!(
            normalize_domain("https://github.example.com/"),
            "github.example.com"
        );
        assert_eq!(normalize_domain("http://gitlab.local"), "gitlab.local");
    }

    #[test]
    fn set_auth_data_rejects_empty_oauth_refresh() {
        let manager = AuthManager::new(MemoryStore::default());
        let err = manager
            .set_auth_data(
                "openai",
                AuthData::OAuth {
                    refresh: "   ".to_owned(),
                    access: "access".to_owned(),
                    expires_at_ms: 1,
                    account_id: None,
                    enterprise_url: None,
                },
            )
            .expect_err("empty oauth refresh token should be rejected");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn set_auth_data_rejects_empty_oauth_access() {
        let manager = AuthManager::new(MemoryStore::default());
        let err = manager
            .set_auth_data(
                "openai",
                AuthData::OAuth {
                    refresh: "refresh".to_owned(),
                    access: "   ".to_owned(),
                    expires_at_ms: 1,
                    account_id: None,
                    enterprise_url: None,
                },
            )
            .expect_err("empty oauth access token should be rejected");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[tokio::test]
    async fn refresh_gitlab_login_requires_oauth_credential() {
        let manager = AuthManager::new(MemoryStore::default());
        let err = manager
            .refresh_gitlab_login("gitlab", "https://gitlab.com")
            .await
            .expect_err("missing gitlab oauth should fail");
        assert!(matches!(err, AppError::Config(_)));
    }
}
