use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    ProviderError,
    config_support::ProviderConfigCredentialStore,
    provider::{
        auth::{AuthStore, refresh_gitlab_token},
        utils,
    },
};
use agena_provider::{AuthData, AuthRefreshStrategy, AuthSecretSelector, SapAiCoreServiceKey};
use agena_provider_google_auth::{GoogleAdcError, access_token as google_adc_access_token};

const EAGER_REFRESH_BUFFER_MS: i64 = 5 * 60 * 1_000;
static CREDENTIAL_IDENTITY_FAILURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
/// A managed provider credential.
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
    AuthData {
        provider_id: String,
        auth: Arc<Mutex<AuthData>>,
        selector: AuthSecretSelector,
        refresh: AuthRefreshStrategy,
        config_path: Option<PathBuf>,
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

    pub fn auth_data_shared(
        label: impl Into<String>,
        provider_id: impl Into<String>,
        auth: Arc<Mutex<AuthData>>,
        selector: AuthSecretSelector,
        refresh: AuthRefreshStrategy,
    ) -> Self {
        Self::new(
            label.into(),
            CredentialSource::AuthData {
                provider_id: provider_id.into(),
                auth,
                selector,
                refresh,
                config_path: None,
            },
        )
    }

    pub fn auth_data_shared_with_store(
        label: impl Into<String>,
        provider_id: impl Into<String>,
        auth: Arc<Mutex<AuthData>>,
        selector: AuthSecretSelector,
        refresh: AuthRefreshStrategy,
        config_path: impl Into<PathBuf>,
    ) -> Self {
        Self::new(
            label.into(),
            CredentialSource::AuthData {
                provider_id: provider_id.into(),
                auth,
                selector,
                refresh,
                config_path: Some(config_path.into()),
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

    pub async fn resolve(&self) -> Result<String, ProviderError> {
        Ok(self.resolve_cached(false).await?.secret)
    }

    pub async fn force_refresh(&self) -> Result<String, ProviderError> {
        Ok(self.resolve_cached(true).await?.secret)
    }

    pub fn prompt_cache_scope(&self) -> String {
        format!(
            "label={};source={}",
            self.inner.label,
            self.inner.source.prompt_cache_scope()
        )
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

    async fn resolve_cached(&self, force_refresh: bool) -> Result<CachedCredential, ProviderError> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        {
            let cached = self.inner.cached.lock().await;
            if !force_refresh
                && let Some(entry) = cached.as_ref().filter(|entry| entry.is_fresh(now_ms))
            {
                return Ok(entry.clone());
            }
        }

        let mut cached = self.inner.cached.lock().await;
        if !force_refresh
            && let Some(entry) = cached.as_ref().filter(|entry| entry.is_fresh(now_ms))
        {
            return Ok(entry.clone());
        }

        let resolved = self.inner.source.resolve(force_refresh).await?;
        if resolved.secret.trim().is_empty() {
            return Err(ProviderError::Config(format!(
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
    fn prompt_cache_scope(&self) -> String {
        match self {
            Self::Static(secret) => format!(
                "static:sha256={}",
                prompt_cache_secret_fingerprint(secret.as_str())
            ),
            Self::Env {
                provider_id,
                field,
                env_key,
            } => {
                let mut scope = format!("env:{provider_id}:{field}:{env_key}");
                if let Ok(secret) = std::env::var(env_key) {
                    let secret = secret.trim();
                    if !secret.is_empty() {
                        scope.push_str(":sha256=");
                        scope.push_str(prompt_cache_secret_fingerprint(secret).as_str());
                    }
                }
                scope
            }
            Self::AuthData {
                provider_id,
                auth,
                selector,
                refresh,
                config_path: _,
            } => {
                let mut scope = format!(
                    "auth_data:{provider_id}:{}:{}",
                    auth_secret_selector_key(*selector),
                    auth_refresh_strategy_key(refresh)
                );
                if let Some(identity) = auth
                    .try_lock()
                    .ok()
                    .as_deref()
                    .and_then(auth_data_prompt_cache_identity)
                {
                    scope.push(':');
                    scope.push_str(identity.as_str());
                }
                scope
            }
            Self::GoogleAdc { provider_id } => google_adc_prompt_cache_scope(provider_id.as_str()),
            Self::SapAiCore {
                provider_id,
                service_key,
                ..
            } => format!(
                "sap_ai_core:{provider_id}:{}:clientid={}",
                service_key.url.trim_end_matches('/'),
                service_key.clientid.trim()
            ),
        }
    }

    async fn resolve(&self, force_refresh: bool) -> Result<CachedCredential, ProviderError> {
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
                    .and_then(normalize_optional_text)
                    .ok_or_else(|| {
                        ProviderError::Config(format!(
                            "{provider_id} is missing required environment variable `{env_key}` for `{field}`"
                        ))
                    })?;
                Ok(CachedCredential {
                    secret: value,
                    expires_at_ms: None,
                })
            }
            Self::AuthData {
                provider_id,
                auth,
                selector,
                refresh,
                config_path,
            } => {
                resolve_inline_auth_credential(
                    auth.as_ref(),
                    provider_id.as_str(),
                    *selector,
                    refresh,
                    config_path.as_deref(),
                    force_refresh,
                )
                .await
            }
            Self::GoogleAdc { provider_id } => {
                let secret = google_adc_access_token()
                    .await
                    .map_err(|error| match error {
                        GoogleAdcError::Provider(error) => ProviderError::Config(format!(
                            "{provider_id} requires Google ADC credentials: {error}"
                        )),
                        GoogleAdcError::Token(error) => ProviderError::Provider(format!(
                            "{provider_id} failed to obtain Google ADC access token: {error}"
                        )),
                    })?;
                Ok(CachedCredential {
                    secret,
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

fn prompt_cache_secret_fingerprint(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

fn google_adc_prompt_cache_scope(provider_id: &str) -> String {
    let mut scope = format!("google_adc:{provider_id}");
    if let Some(identity) = google_adc_prompt_cache_identity() {
        scope.push(':');
        scope.push_str(identity.as_str());
    }
    scope
}

fn google_adc_prompt_cache_identity() -> Option<String> {
    if let Some(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .and_then(normalize_optional_text)
    {
        let mut parts = vec![
            "source=application_credentials_env".to_owned(),
            format!("path={path}"),
        ];
        if let Some((payload_fingerprint, fields)) =
            prompt_cache_json_identity(std::path::Path::new(path.as_str()))
        {
            parts.push(format!("payload_sha256={payload_fingerprint}"));
            parts.extend(fields);
        }
        return Some(parts.join(";"));
    }

    if let Some(path) = google_adc_default_credentials_path() {
        let path_text = path.to_string_lossy().trim().to_owned();
        let mut parts = vec![
            "source=config_default_credentials".to_owned(),
            format!("path={path_text}"),
        ];
        if let Some((payload_fingerprint, fields)) = prompt_cache_json_identity(path.as_path()) {
            parts.push(format!("payload_sha256={payload_fingerprint}"));
            parts.extend(fields);
        }
        return Some(parts.join(";"));
    }

    let mut parts = Vec::new();
    for (env_key, field_key) in [
        ("GOOGLE_CLOUD_PROJECT", "google_cloud_project"),
        ("GCLOUD_PROJECT", "gcloud_project"),
        ("GCP_PROJECT", "gcp_project"),
        ("GOOGLE_PROJECT_ID", "google_project_id"),
        ("CLOUDSDK_CORE_PROJECT", "cloudsdk_core_project"),
    ] {
        if let Some(value) = std::env::var(env_key)
            .ok()
            .and_then(normalize_optional_text)
        {
            parts.push(format!("{field_key}={value}"));
        }
    }

    (!parts.is_empty()).then(|| {
        let mut identity = vec!["source=ambient".to_owned()];
        identity.extend(parts);
        identity.join(";")
    })
}

fn google_adc_default_credentials_path() -> Option<std::path::PathBuf> {
    #[cfg(target_family = "unix")]
    {
        let home = std::env::var("HOME")
            .ok()
            .and_then(normalize_optional_text)?;
        let mut path = std::path::PathBuf::from(home);
        path.push(".config/gcloud/application_default_credentials.json");
        prompt_cache_existing_credential_path(path)
    }

    #[cfg(target_family = "windows")]
    {
        let app_data = std::env::var("APPDATA")
            .ok()
            .and_then(normalize_optional_text)?;
        let mut path = std::path::PathBuf::from(app_data);
        path.push("gcloud/application_default_credentials.json");
        prompt_cache_existing_credential_path(path)
    }
}

fn prompt_cache_existing_credential_path(path: std::path::PathBuf) -> Option<std::path::PathBuf> {
    match path.try_exists() {
        Ok(true) => Some(path),
        Ok(false) => None,
        Err(error) => {
            tracing::error!(
                path = %path.display(),
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "inspect provider credential path for prompt-cache identity",
                    &error,
                ),
                "provider credential path could not be inspected"
            );
            // Keep the path in the identity flow so the subsequent read emits
            // a one-use failure fingerprint instead of collapsing to a shared
            // cache shape.
            Some(path)
        }
    }
}

fn prompt_cache_json_identity(path: &std::path::Path) -> Option<(String, Vec<String>)> {
    let payload = match std::fs::read_to_string(path) {
        Ok(payload) => payload,
        Err(error) => {
            let sequence = CREDENTIAL_IDENTITY_FAILURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                path = %path.display(),
                sequence,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "read provider credential JSON for prompt-cache identity",
                    &error,
                ),
                "provider credential identity is using a one-use fingerprint"
            );
            return Some((
                format!("credential-read-failure-{sequence}"),
                vec!["credential_json_unreadable=true".to_owned()],
            ));
        }
    };
    let fingerprint = utils::request_shape_fingerprint(&payload);
    let json: serde_json::Value = match serde_json::from_str(payload.as_str()) {
        Ok(json) => json,
        Err(error) => {
            let sequence = CREDENTIAL_IDENTITY_FAILURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                path = %path.display(),
                sequence,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode provider credential JSON for prompt-cache identity",
                    &error,
                ),
                "provider credential identity is using a one-use fingerprint"
            );
            return Some((
                format!("credential-decode-failure-{sequence}"),
                vec!["credential_json_invalid=true".to_owned()],
            ));
        }
    };

    let mut fields = Vec::new();
    if let Some(project_id) = json
        .get("project_id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_text(value.to_owned()))
    {
        fields.push(format!("project_id={project_id}"));
    }
    if let Some(quota_project_id) = json
        .get("quota_project_id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_text(value.to_owned()))
    {
        fields.push(format!("quota_project_id={quota_project_id}"));
    }
    if let Some(client_email) = json
        .get("client_email")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_text(value.to_owned()))
    {
        fields.push(format!("client_email={client_email}"));
    }
    if let Some(client_id) = json
        .get("client_id")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_text(value.to_owned()))
    {
        fields.push(format!(
            "client_id_sha256={}",
            utils::request_shape_fingerprint(&client_id)
        ));
    }

    Some((fingerprint, fields))
}

fn auth_secret_selector_key(selector: AuthSecretSelector) -> &'static str {
    match selector {
        AuthSecretSelector::AccessOrApiKey => "access_or_api_key",
        AuthSecretSelector::RefreshOrAccess => "refresh_or_access",
    }
}

fn auth_refresh_strategy_key(strategy: &AuthRefreshStrategy) -> String {
    match strategy {
        AuthRefreshStrategy::None => "none".to_owned(),
        AuthRefreshStrategy::ReloadFromStore => "reload_from_store".to_owned(),
        AuthRefreshStrategy::OpenAiOAuth => "openai_oauth".to_owned(),
        AuthRefreshStrategy::GitlabOAuth { instance_url } => {
            format!("gitlab_oauth:{}", instance_url.trim_end_matches('/'))
        }
    }
}

fn auth_data_prompt_cache_identity(auth: &AuthData) -> Option<String> {
    match auth {
        AuthData::Api { key } => Some(format!(
            "api;key_sha256={}",
            prompt_cache_secret_fingerprint(key.as_str())
        )),
        AuthData::WellKnown { key, .. } => Some(format!(
            "well_known;key_sha256={}",
            prompt_cache_secret_fingerprint(key.as_str())
        )),
        AuthData::OAuth {
            account_id,
            enterprise_url,
            user,
            ..
        } => {
            let mut parts = vec!["oauth".to_owned()];
            if let Some(account_id) = account_id
                .as_ref()
                .and_then(|value| normalize_optional_text(value.clone()))
            {
                parts.push(format!("account_id={account_id}"));
            }
            if let Some(enterprise_url) = enterprise_url
                .as_ref()
                .and_then(|value| normalize_optional_text(value.clone()))
            {
                parts.push(format!(
                    "enterprise_url={}",
                    enterprise_url.trim_end_matches('/')
                ));
            }
            if let Some(username) = user
                .as_ref()
                .and_then(|user| normalize_optional_text(user.username.clone()))
            {
                parts.push(format!("username={username}"));
            }
            Some(parts.join(";"))
        }
    }
}

async fn resolve_inline_auth_credential(
    auth: &Mutex<AuthData>,
    provider_id: &str,
    selector: AuthSecretSelector,
    refresh: &AuthRefreshStrategy,
    config_path: Option<&Path>,
    force_refresh: bool,
) -> Result<CachedCredential, ProviderError> {
    let mut current = auth.lock().await.clone();
    if let Some(config_path) = config_path
        && let Some(stored) = load_auth_data_from_store(config_path, provider_id)?
    {
        if stored != current {
            *auth.lock().await = stored.clone();
        }
        current = stored;
    }
    let selected = select_auth_secret(&current, selector, provider_id);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let should_refresh = match refresh {
        AuthRefreshStrategy::GitlabOAuth { .. } | AuthRefreshStrategy::OpenAiOAuth => {
            oauth_refresh_token(&current).is_some()
                && match selected.as_ref() {
                    Ok(selected) => force_refresh || !selected.is_fresh(now_ms),
                    Err(..) => true,
                }
        }
        AuthRefreshStrategy::None | AuthRefreshStrategy::ReloadFromStore => false,
    };

    if !should_refresh {
        return selected;
    }

    match refresh {
        AuthRefreshStrategy::OpenAiOAuth => {
            let AuthData::OAuth {
                issuer,
                refresh: refresh_token,
                id_token,
                account_id,
                chatgpt_account_is_fedramp,
                enterprise_url,
                user,
                ..
            } = current
            else {
                return selected;
            };

            let refreshed =
                crate::provider::auth::refresh_openai_token(refresh_token.as_str()).await?;
            let updated = AuthData::OAuth {
                issuer,
                refresh: refreshed.refresh,
                access: refreshed.access,
                id_token: refreshed.id_token.or(id_token),
                expires_at_ms: refreshed.expires_at_ms,
                account_id: refreshed.account_id.or(account_id),
                chatgpt_account_is_fedramp: refreshed.chatgpt_account_is_fedramp
                    || chatgpt_account_is_fedramp,
                enterprise_url,
                user,
            };
            write_auth_data(auth, provider_id, config_path, updated.clone()).await?;
            select_auth_secret(&updated, selector, provider_id)
        }
        AuthRefreshStrategy::GitlabOAuth { instance_url } => {
            let AuthData::OAuth {
                issuer,
                refresh: refresh_token,
                account_id,
                enterprise_url,
                user,
                ..
            } = current
            else {
                return selected;
            };

            let refreshed =
                refresh_gitlab_token(instance_url.as_str(), refresh_token.as_str()).await?;
            let updated = AuthData::OAuth {
                issuer,
                refresh: refreshed.refresh,
                access: refreshed.access,
                id_token: refreshed.id_token,
                expires_at_ms: refreshed.expires_at_ms,
                account_id,
                chatgpt_account_is_fedramp: refreshed.chatgpt_account_is_fedramp,
                enterprise_url,
                user,
            };
            write_auth_data(auth, provider_id, config_path, updated.clone()).await?;
            select_auth_secret(&updated, selector, provider_id)
        }
        AuthRefreshStrategy::None | AuthRefreshStrategy::ReloadFromStore => selected,
    }
}

fn load_auth_data_from_store(
    config_path: &Path,
    provider_id: &str,
) -> Result<Option<AuthData>, ProviderError> {
    ProviderConfigCredentialStore::new(config_path.to_path_buf()).get(provider_id)
}

fn persist_auth_data_to_store(
    config_path: &Path,
    provider_id: &str,
    auth: &AuthData,
) -> Result<(), ProviderError> {
    ProviderConfigCredentialStore::new(config_path.to_path_buf()).set(provider_id, auth.clone())
}

async fn write_auth_data(
    auth: &Mutex<AuthData>,
    provider_id: &str,
    config_path: Option<&Path>,
    updated: AuthData,
) -> Result<(), ProviderError> {
    if let Some(config_path) = config_path {
        persist_auth_data_to_store(config_path, provider_id, &updated)?;
    }
    *auth.lock().await = updated;
    Ok(())
}

fn oauth_refresh_token(auth: &AuthData) -> Option<&str> {
    match auth {
        AuthData::OAuth { refresh, .. } => {
            let refresh = refresh.trim();
            (!refresh.is_empty()).then_some(refresh)
        }
        AuthData::Api { .. } | AuthData::WellKnown { .. } => None,
    }
}

fn select_auth_secret(
    auth: &AuthData,
    selector: AuthSecretSelector,
    provider_id: &str,
) -> Result<CachedCredential, ProviderError> {
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

                Err(ProviderError::Config(format!(
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
) -> Result<CachedCredential, ProviderError> {
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
        .map_err(ProviderError::from)?;

    let status = response.status();
    let body = utils::response_text_bounded(
        response,
        utils::MAX_PROVIDER_ERROR_RESPONSE_BYTES,
        "SAP AI Core token response",
    )
    .await?;
    if !status.is_success() {
        return Err(ProviderError::Config(format!(
            "{provider_id} SAP AI Core token exchange failed with status {status}: {body}"
        )));
    }

    let token = serde_json::from_str::<SapAiCoreTokenResponse>(&body)?;
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
    status == reqwest::StatusCode::UNAUTHORIZED
}

fn normalize_optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn normalize_expires_at_ms(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}
