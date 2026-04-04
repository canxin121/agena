use std::{fmt, sync::Arc};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{
    error::AppError,
    provider::auth::{AuthData, AuthStore, refresh_gitlab_token},
};

const EAGER_REFRESH_BUFFER_MS: i64 = 5 * 60 * 1_000;
const GOOGLE_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSecretSelector {
    AccessOrApiKey,
    RefreshOrAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRefreshStrategy {
    None,
    ReloadFromStore,
    GitlabOAuth { instance_url: String },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SapAiCoreServiceKey {
    pub clientid: String,
    pub clientsecret: String,
    pub url: String,
    pub serviceurls: SapAiCoreServiceUrls,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SapAiCoreServiceUrls {
    #[serde(rename = "AI_API_URL")]
    pub ai_api_url: String,
}

#[derive(Clone)]
pub struct ManagedCredential {
    inner: Arc<ManagedCredentialInner>,
}

struct ManagedCredentialInner {
    label: String,
    source: CredentialSource,
    cached: Mutex<Option<CachedCredential>>,
}

#[derive(Debug, Clone)]
struct CachedCredential {
    secret: String,
    expires_at_ms: Option<i64>,
}

enum CredentialSource {
    Static(String),
    Env {
        provider_id: String,
        field: &'static str,
        env_key: String,
    },
    AuthStore {
        auth_store: Arc<dyn AuthStore>,
        provider_id: String,
        selector: AuthSecretSelector,
        refresh: AuthRefreshStrategy,
    },
    GoogleAdc {
        provider_id: String,
    },
    SapAiCore {
        client: reqwest::Client,
        provider_id: String,
        service_key: SapAiCoreServiceKey,
    },
}

#[derive(Debug, Deserialize)]
struct SapAiCoreTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl ManagedCredential {
    pub fn static_value(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label.into(), CredentialSource::Static(value.into()))
    }

    pub fn environment(
        label: impl Into<String>,
        provider_id: impl Into<String>,
        field: &'static str,
        env_key: impl Into<String>,
    ) -> Self {
        Self::new(
            label.into(),
            CredentialSource::Env {
                provider_id: provider_id.into(),
                field,
                env_key: env_key.into(),
            },
        )
    }

    pub fn auth_store(
        label: impl Into<String>,
        auth_store: Arc<dyn AuthStore>,
        provider_id: impl Into<String>,
        selector: AuthSecretSelector,
        refresh: AuthRefreshStrategy,
    ) -> Self {
        Self::new(
            label.into(),
            CredentialSource::AuthStore {
                auth_store,
                provider_id: provider_id.into(),
                selector,
                refresh,
            },
        )
    }

    pub fn google_adc(label: impl Into<String>, provider_id: impl Into<String>) -> Self {
        Self::new(
            label.into(),
            CredentialSource::GoogleAdc {
                provider_id: provider_id.into(),
            },
        )
    }

    pub fn sap_ai_core(
        label: impl Into<String>,
        client: reqwest::Client,
        provider_id: impl Into<String>,
        service_key: SapAiCoreServiceKey,
    ) -> Self {
        Self::new(
            label.into(),
            CredentialSource::SapAiCore {
                client,
                provider_id: provider_id.into(),
                service_key,
            },
        )
    }

    pub async fn resolve(&self) -> Result<String, AppError> {
        Ok(self.resolve_cached(false).await?.secret)
    }

    pub async fn force_refresh(&self) -> Result<String, AppError> {
        Ok(self.resolve_cached(true).await?.secret)
    }

    fn new(label: String, source: CredentialSource) -> Self {
        Self {
            inner: Arc::new(ManagedCredentialInner {
                label,
                source,
                cached: Mutex::new(None),
            }),
        }
    }

    async fn resolve_cached(&self, force_refresh: bool) -> Result<CachedCredential, AppError> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        {
            let cached = self.inner.cached.lock().await;
            if !force_refresh {
                if let Some(entry) = cached.as_ref().filter(|entry| entry.is_fresh(now_ms)) {
                    return Ok(entry.clone());
                }
            }
        }

        let mut cached = self.inner.cached.lock().await;
        if !force_refresh {
            if let Some(entry) = cached.as_ref().filter(|entry| entry.is_fresh(now_ms)) {
                return Ok(entry.clone());
            }
        }

        let resolved = self.inner.source.resolve(force_refresh).await?;
        if resolved.secret.trim().is_empty() {
            return Err(AppError::Config(format!(
                "{} resolved to an empty credential",
                self.inner.label
            )));
        }

        if self.inner.source.should_cache(&resolved) {
            *cached = Some(resolved.clone());
        } else {
            *cached = None;
        }

        Ok(resolved)
    }
}

impl fmt::Debug for ManagedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedCredential")
            .field("label", &self.inner.label)
            .finish_non_exhaustive()
    }
}

impl CachedCredential {
    fn is_fresh(&self, now_ms: i64) -> bool {
        if self.secret.trim().is_empty() {
            return false;
        }

        match self.expires_at_ms {
            Some(expires_at_ms) => expires_at_ms > now_ms + EAGER_REFRESH_BUFFER_MS,
            None => true,
        }
    }
}

impl CredentialSource {
    async fn resolve(&self, force_refresh: bool) -> Result<CachedCredential, AppError> {
        match self {
            Self::Static(secret) => Ok(CachedCredential {
                secret: secret.clone(),
                expires_at_ms: None,
            }),
            Self::Env {
                provider_id,
                field,
                env_key,
            } => {
                let value = std::env::var(env_key)
                    .ok()
                    .and_then(|value| normalize_optional_text(value))
                    .ok_or_else(|| {
                        AppError::Config(format!(
                            "{provider_id} is missing required environment variable `{env_key}` for `{field}`"
                        ))
                    })?;
                Ok(CachedCredential {
                    secret: value,
                    expires_at_ms: None,
                })
            }
            Self::AuthStore {
                auth_store,
                provider_id,
                selector,
                refresh,
            } => {
                resolve_auth_store_credential(
                    auth_store.as_ref(),
                    provider_id.as_str(),
                    *selector,
                    refresh,
                    force_refresh,
                )
                .await
            }
            Self::GoogleAdc { provider_id } => {
                let provider = gcp_auth::provider().await.map_err(|err| {
                    AppError::Config(format!(
                        "{provider_id} requires Google ADC credentials: {err}"
                    ))
                })?;

                let token = provider
                    .token(&[GOOGLE_CLOUD_PLATFORM_SCOPE])
                    .await
                    .map_err(|err| {
                        AppError::Provider(format!(
                            "{provider_id} failed to obtain Google ADC access token: {err}"
                        ))
                    })?;

                Ok(CachedCredential {
                    secret: token.as_str().to_owned(),
                    expires_at_ms: None,
                })
            }
            Self::SapAiCore {
                client,
                provider_id,
                service_key,
            } => resolve_sap_ai_core_credential(client, provider_id.as_str(), service_key).await,
        }
    }

    fn should_cache(&self, credential: &CachedCredential) -> bool {
        if credential.expires_at_ms.is_some() {
            return true;
        }

        matches!(self, Self::Static(_) | Self::SapAiCore { .. })
    }
}

async fn resolve_auth_store_credential(
    auth_store: &dyn AuthStore,
    provider_id: &str,
    selector: AuthSecretSelector,
    refresh: &AuthRefreshStrategy,
    force_refresh: bool,
) -> Result<CachedCredential, AppError> {
    let auth = auth_store.get(provider_id)?.ok_or_else(|| {
        AppError::Config(format!(
            "auth credential `{provider_id}` was not found in the auth store"
        ))
    })?;

    let selected = select_auth_secret(&auth, selector, provider_id)?;
    let should_refresh = matches!(refresh, AuthRefreshStrategy::GitlabOAuth { .. })
        && (force_refresh || !selected.is_fresh(chrono::Utc::now().timestamp_millis()));

    if !should_refresh {
        return Ok(selected);
    }

    match refresh {
        AuthRefreshStrategy::GitlabOAuth { instance_url } => {
            let AuthData::OAuth {
                refresh: refresh_token,
                account_id,
                enterprise_url,
                ..
            } = auth
            else {
                return Ok(selected);
            };

            let refreshed =
                refresh_gitlab_token(instance_url.as_str(), refresh_token.as_str()).await?;
            let updated = AuthData::OAuth {
                refresh: refreshed.refresh,
                access: refreshed.access,
                expires_at_ms: refreshed.expires_at_ms,
                account_id,
                enterprise_url,
            };
            auth_store.set(provider_id, updated.clone())?;
            select_auth_secret(&updated, selector, provider_id)
        }
        AuthRefreshStrategy::None | AuthRefreshStrategy::ReloadFromStore => Ok(selected),
    }
}

fn select_auth_secret(
    auth: &AuthData,
    selector: AuthSecretSelector,
    provider_id: &str,
) -> Result<CachedCredential, AppError> {
    match selector {
        AuthSecretSelector::AccessOrApiKey => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => Ok(CachedCredential {
                secret: key.clone(),
                expires_at_ms: None,
            }),
            AuthData::OAuth {
                access,
                expires_at_ms,
                ..
            } => Ok(CachedCredential {
                secret: access.clone(),
                expires_at_ms: normalize_expires_at_ms(*expires_at_ms),
            }),
        },
        AuthSecretSelector::RefreshOrAccess => match auth {
            AuthData::Api { key } | AuthData::WellKnown { key, .. } => Ok(CachedCredential {
                secret: key.clone(),
                expires_at_ms: None,
            }),
            AuthData::OAuth {
                refresh,
                access,
                expires_at_ms,
                ..
            } => {
                if let Some(refresh) = normalize_optional_text(refresh.clone()) {
                    return Ok(CachedCredential {
                        secret: refresh,
                        expires_at_ms: None,
                    });
                }

                if let Some(access) = normalize_optional_text(access.clone()) {
                    return Ok(CachedCredential {
                        secret: access,
                        expires_at_ms: normalize_expires_at_ms(*expires_at_ms),
                    });
                }

                Err(AppError::Config(format!(
                    "{provider_id} oauth credential does not contain a usable refresh/access token"
                )))
            }
        },
    }
}

async fn resolve_sap_ai_core_credential(
    client: &reqwest::Client,
    provider_id: &str,
    service_key: &SapAiCoreServiceKey,
) -> Result<CachedCredential, AppError> {
    let token_url = format!("{}/oauth/token", service_key.url.trim_end_matches('/'));
    let response = client
        .post(token_url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body({
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("grant_type", "client_credentials");
            serializer.append_pair("client_id", service_key.clientid.as_str());
            serializer.append_pair("client_secret", service_key.clientsecret.as_str());
            serializer.finish()
        })
        .send()
        .await
        .map_err(AppError::from)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Config(format!(
            "{provider_id} SAP AI Core token exchange failed with status {status}: {body}"
        )));
    }

    let token = response.json::<SapAiCoreTokenResponse>().await?;
    Ok(CachedCredential {
        secret: token.access_token,
        expires_at_ms: token
            .expires_in
            .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds as i64 * 1_000),
    })
}

pub fn parse_sap_ai_core_service_key(raw: &str) -> Result<SapAiCoreServiceKey, serde_json::Error> {
    serde_json::from_str(raw)
}

pub fn should_retry_credential(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    )
}

fn normalize_optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn normalize_expires_at_ms(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<HashMap<String, AuthData>>,
    }

    impl AuthStore for MemoryStore {
        fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
            Ok(self
                .values
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .clone())
        }

        fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
            Ok(self
                .values
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .get(provider_id)
                .cloned())
        }

        fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
            self.values
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .insert(provider_id.to_owned(), auth);
            Ok(())
        }

        fn remove(&self, provider_id: &str) -> Result<(), AppError> {
            self.values
                .lock()
                .map_err(|_| AppError::Internal("memory auth store lock poisoned".to_owned()))?
                .remove(provider_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn env_credentials_read_latest_value_on_force_refresh() {
        let key = format!("AGENA_TEST_CREDENTIAL_{}", std::process::id());
        unsafe { std::env::set_var(key.as_str(), "first") };

        let credential =
            ManagedCredential::environment("test env", "openai", "api_key", key.as_str());
        assert_eq!(credential.resolve().await.expect("env credential"), "first");

        unsafe { std::env::set_var(key.as_str(), "second") };
        assert_eq!(
            credential
                .force_refresh()
                .await
                .expect("forced env refresh should re-read"),
            "second"
        );
    }

    #[tokio::test]
    async fn auth_store_reload_reads_latest_value() {
        let store = Arc::new(MemoryStore::default());
        store
            .set(
                "github-copilot",
                AuthData::OAuth {
                    refresh: "refresh-1".to_owned(),
                    access: "access-1".to_owned(),
                    expires_at_ms: 0,
                    account_id: None,
                    enterprise_url: None,
                },
            )
            .expect("initial auth");

        let credential = ManagedCredential::auth_store(
            "copilot",
            store.clone(),
            "github-copilot",
            AuthSecretSelector::RefreshOrAccess,
            AuthRefreshStrategy::ReloadFromStore,
        );
        assert_eq!(
            credential.resolve().await.expect("copilot credential"),
            "refresh-1"
        );

        store
            .set(
                "github-copilot",
                AuthData::OAuth {
                    refresh: "refresh-2".to_owned(),
                    access: "access-2".to_owned(),
                    expires_at_ms: 0,
                    account_id: None,
                    enterprise_url: None,
                },
            )
            .expect("updated auth");

        assert_eq!(
            credential
                .force_refresh()
                .await
                .expect("reload should pick up updated auth"),
            "refresh-2"
        );
    }

    #[test]
    fn parse_sap_service_key_supports_ai_api_url() {
        let parsed = parse_sap_ai_core_service_key(
            r#"{
                "clientid": "client",
                "clientsecret": "secret",
                "url": "https://auth.example.com",
                "serviceurls": {
                    "AI_API_URL": "https://api.example.com/v2"
                }
            }"#,
        )
        .expect("service key should parse");

        assert_eq!(parsed.clientid, "client");
        assert_eq!(parsed.serviceurls.ai_api_url, "https://api.example.com/v2");
    }
}
