use std::{
    fs,
    future::Future,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    model::{Model, ModelCapabilities},
    model_catalog::{
        DEFAULT_GITHUB_FALLBACK_URL, ModelCatalogDocument, bundled_catalog_document,
        catalog_definition_to_provider_definition,
    },
};

use super::utils;

const CACHE_DIR_ENV: &str = "AGENA_PROVIDER_MODELS_CACHE_DIR";
const CACHE_TTL_ENV: &str = "AGENA_PROVIDER_MODELS_CACHE_TTL_SECS";
const CATALOG_FALLBACK_URL_ENV: &str = "AGENA_PROVIDER_MODELS_CATALOG_FALLBACK_URL";
const DEFAULT_CACHE_TTL_SECS: u64 = 15 * 60;
const DEFAULT_CATALOG_FALLBACK_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteModelCatalogSource {
    pub provider_id: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_scope: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub catalog_provider_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub catalog_visible_model_prefix: String,
}

impl RemoteModelCatalogSource {
    pub(crate) fn new(
        provider_id: impl Into<String>,
        endpoint: impl Into<String>,
        auth_scope: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into().trim().to_owned(),
            endpoint: endpoint.into().trim().to_owned(),
            auth_scope: auth_scope.into().trim().to_owned(),
            catalog_provider_id: String::new(),
            catalog_visible_model_prefix: String::new(),
        }
    }

    pub(crate) fn with_catalog_provider_id(
        mut self,
        catalog_provider_id: impl Into<String>,
    ) -> Self {
        self.catalog_provider_id = catalog_provider_id.into().trim().to_owned();
        self
    }

    pub(crate) fn with_catalog_visible_model_prefix(
        mut self,
        catalog_visible_model_prefix: impl Into<String>,
    ) -> Self {
        let prefix = catalog_visible_model_prefix
            .into()
            .trim()
            .trim_end_matches('/')
            .to_owned();
        self.catalog_visible_model_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteModelCatalogCache {
    root: Option<PathBuf>,
    ttl: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteModelCatalogCacheEntry {
    fetched_at_ms: i64,
    source: RemoteModelCatalogSource,
    models: Vec<Model>,
}

impl Default for RemoteModelCatalogCache {
    fn default() -> Self {
        Self {
            root: default_cache_root(),
            ttl: default_cache_ttl(),
        }
    }
}

impl RemoteModelCatalogCache {
    #[cfg(test)]
    pub(crate) fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
            ttl: default_cache_ttl(),
        }
    }

    #[cfg(test)]
    fn with_root_and_ttl(root: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            root: Some(root.into()),
            ttl,
        }
    }

    pub(crate) async fn get_or_fetch<F, Fut>(
        &self,
        source: &RemoteModelCatalogSource,
        fetcher: F,
    ) -> Result<Vec<Model>, AppError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<Model>, AppError>>,
    {
        let cached = match self.read_entry(source) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(
                    provider_id = %source.provider_id,
                    endpoint = %source.endpoint,
                    error = %error,
                    "failed to read provider model catalog cache"
                );
                None
            }
        };

        if let Some(entry) = cached.as_ref()
            && self.is_fresh(entry)
        {
            return Ok(entry.models.clone());
        }

        match fetcher().await {
            Ok(models) => {
                if let Err(error) = self.write_entry(source, models.as_slice()) {
                    tracing::warn!(
                        provider_id = %source.provider_id,
                        endpoint = %source.endpoint,
                        error = %error,
                        "failed to write provider model catalog cache"
                    );
                }
                Ok(models)
            }
            Err(error) => {
                if let Some(entry) = cached {
                    tracing::warn!(
                        provider_id = %source.provider_id,
                        endpoint = %source.endpoint,
                        age_ms = now_ms().saturating_sub(entry.fetched_at_ms),
                        fetch_error = %error,
                        "provider model catalog refresh failed; falling back to stale cache"
                    );
                    return Ok(entry.models);
                }
                if let Some(models) = self.catalog_fallback_models(source).await? {
                    if let Err(write_error) = self.write_entry(source, models.as_slice()) {
                        tracing::warn!(
                            provider_id = %source.provider_id,
                            endpoint = %source.endpoint,
                            error = %write_error,
                            "failed to write provider model catalog fallback cache"
                        );
                    }
                    return Ok(models);
                }
                Err(error)
            }
        }
    }

    async fn catalog_fallback_models(
        &self,
        source: &RemoteModelCatalogSource,
    ) -> Result<Option<Vec<Model>>, AppError> {
        let catalog_provider_id = source.catalog_provider_id.trim();
        if catalog_provider_id.is_empty() {
            return Ok(None);
        }

        match fetch_catalog_document(catalog_fallback_url().as_str()).await {
            Ok(document) => {
                if let Some(models) = catalog_models_from_document(source, &document) {
                    return Ok(Some(models));
                }
            }
            Err(error) => {
                tracing::warn!(
                    provider_id = %source.provider_id,
                    catalog_provider_id,
                    error = %error,
                    "failed to fetch provider model catalog fallback; trying bundled catalog"
                );
            }
        }

        match bundled_catalog_document() {
            Ok(document) => Ok(catalog_models_from_document(source, &document)),
            Err(error) => {
                tracing::warn!(
                    provider_id = %source.provider_id,
                    catalog_provider_id,
                    error = %error,
                    "failed to load bundled provider model catalog fallback"
                );
                Ok(None)
            }
        }
    }

    fn is_fresh(&self, entry: &RemoteModelCatalogCacheEntry) -> bool {
        let age_ms = now_ms().saturating_sub(entry.fetched_at_ms);
        age_ms <= self.ttl.as_millis().min(i64::MAX as u128) as i64
    }

    fn read_entry(
        &self,
        source: &RemoteModelCatalogSource,
    ) -> Result<Option<RemoteModelCatalogCacheEntry>, AppError> {
        let Some(path) = self.path_for(source) else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        let payload = fs::read_to_string(path)?;
        let entry = serde_json::from_str::<RemoteModelCatalogCacheEntry>(&payload)?;
        Ok(Some(entry))
    }

    fn write_entry(
        &self,
        source: &RemoteModelCatalogSource,
        models: &[Model],
    ) -> Result<(), AppError> {
        let Some(path) = self.path_for(source) else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let entry = RemoteModelCatalogCacheEntry {
            fetched_at_ms: now_ms(),
            source: source.clone(),
            models: models.to_vec(),
        };
        fs::write(path, serde_json::to_vec_pretty(&entry)?)?;
        Ok(())
    }

    fn path_for(&self, source: &RemoteModelCatalogSource) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let fingerprint = utils::request_shape_fingerprint(source);
        Some(root.join(format!(
            "{}-{fingerprint}.json",
            sanitize_provider_id(source.provider_id.as_str())
        )))
    }
}

fn default_cache_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(CACHE_DIR_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join(".agena").join("provider-models"))
}

fn default_cache_ttl() -> Duration {
    let secs = std::env::var(CACHE_TTL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CACHE_TTL_SECS);
    Duration::from_secs(secs)
}

fn catalog_fallback_url() -> String {
    std::env::var(CATALOG_FALLBACK_URL_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_FALLBACK_URL.to_owned())
}

fn sanitize_provider_id(provider_id: &str) -> String {
    provider_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

async fn fetch_catalog_document(url: &str) -> Result<ModelCatalogDocument, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_CATALOG_FALLBACK_TIMEOUT_SECS))
        .build()
        .map_err(|err| AppError::Provider(format!("build catalog fallback client: {err}")))?;
    let response =
        client.get(url).send().await.map_err(|err| {
            AppError::Provider(format!("fetch catalog fallback from {url}: {err}"))
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| AppError::Provider(format!("read catalog fallback from {url}: {err}")))?;
    if !status.is_success() {
        return Err(AppError::Provider(format!(
            "fetch catalog fallback from {url}: http {status}: {body}"
        )));
    }
    serde_json::from_str::<ModelCatalogDocument>(&body)
        .map_err(|err| AppError::Config(format!("parse catalog fallback from {url}: {err}")))
}

fn catalog_models_from_document(
    source: &RemoteModelCatalogSource,
    document: &ModelCatalogDocument,
) -> Option<Vec<Model>> {
    let catalog = document.model_record();
    if catalog.models.is_empty() {
        return None;
    }
    Some(
        catalog
            .models
            .iter()
            .map(|(model_id, definition)| {
                let mut model = Model::new(
                    source.provider_id.as_str(),
                    adapter_model_id_from_catalog(
                        model_id.as_str(),
                        source.catalog_visible_model_prefix.as_str(),
                    ),
                );
                if let Some(display_name) = definition.display_name.clone() {
                    model = model.with_display_name(display_name);
                }
                if let Some(family) = definition.family {
                    model = model.with_family(family);
                }
                let metadata_fallback = model.metadata.clone();
                catalog_definition_to_provider_definition(definition).apply_to_model(
                    model,
                    &ModelCapabilities::default(),
                    &metadata_fallback,
                )
            })
            .collect(),
    )
}

fn adapter_model_id_from_catalog<'a>(
    visible_model_id: &'a str,
    visible_model_prefix: &str,
) -> &'a str {
    let visible_model_id = visible_model_id.trim();
    let visible_model_prefix = visible_model_prefix.trim();
    if visible_model_prefix.is_empty() {
        return visible_model_id;
    }

    visible_model_id
        .strip_prefix(visible_model_prefix)
        .unwrap_or(visible_model_id)
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        sync::Arc,
        sync::atomic::{AtomicUsize, Ordering},
        sync::{LazyLock, Mutex},
    };

    use mockito::Server;
    use tempfile::tempdir;

    use super::*;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
        &LOCK
    }

    struct EnvVarGuard {
        key: String,
        original: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: impl AsRef<OsStr>) -> Self {
            let key_string = key.to_owned();
            let original = std::env::var_os(key);
            // SAFETY: tests serialize env mutation through `env_lock()`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key_string,
                original,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(original) = self.original.take() {
                // SAFETY: tests serialize env mutation through `env_lock()`.
                unsafe {
                    std::env::set_var(&self.key, original);
                }
            } else {
                // SAFETY: tests serialize env mutation through `env_lock()`.
                unsafe {
                    std::env::remove_var(&self.key);
                }
            }
        }
    }

    fn test_source() -> RemoteModelCatalogSource {
        RemoteModelCatalogSource::new("openai", "https://api.openai.com/v1/models", "scope-a")
    }

    fn test_models() -> Vec<Model> {
        vec![Model::new("openai", "gpt-5").with_display_name("GPT-5")]
    }

    #[tokio::test]
    async fn returns_fresh_cached_models_without_refetching() {
        let dir = tempdir().expect("tempdir should create");
        let cache = RemoteModelCatalogCache::with_root(dir.path());
        let source = test_source();
        let fetches = Arc::new(AtomicUsize::new(0));

        let first = cache
            .get_or_fetch(&source, {
                let fetches = Arc::clone(&fetches);
                || async move {
                    fetches.fetch_add(1, Ordering::SeqCst);
                    Ok(test_models())
                }
            })
            .await
            .expect("initial fetch should succeed");
        let second = cache
            .get_or_fetch(&source, || async {
                panic!("fresh cache should avoid refetch");
                #[allow(unreachable_code)]
                Ok(Vec::new())
            })
            .await
            .expect("cached fetch should succeed");

        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn falls_back_to_stale_cache_when_refresh_fails() {
        let dir = tempdir().expect("tempdir should create");
        let cache = RemoteModelCatalogCache::with_root_and_ttl(dir.path(), Duration::from_secs(1));
        let source = test_source();
        let path = cache.path_for(&source).expect("cache path should resolve");
        fs::create_dir_all(path.parent().expect("cache path should have parent"))
            .expect("cache parent should create");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&RemoteModelCatalogCacheEntry {
                fetched_at_ms: now_ms() - 5_000,
                source: source.clone(),
                models: test_models(),
            })
            .expect("entry should serialize"),
        )
        .expect("cache file should write");

        let models = cache
            .get_or_fetch(&source, || async {
                Err(AppError::Provider("upstream unavailable".to_owned()))
            })
            .await
            .expect("stale cache fallback should succeed");

        assert_eq!(models, test_models());
    }

    #[tokio::test]
    async fn falls_back_to_catalog_when_fetch_fails_without_cache() {
        let _env_lock = env_lock().lock().expect("env lock should succeed");
        let dir = tempdir().expect("tempdir should create");
        let _cache_dir = EnvVarGuard::set(CACHE_DIR_ENV, dir.path().as_os_str());
        let mut server = Server::new_async().await;
        let fallback_url = format!("{}/catalog.json", server.url());
        let _fallback_url = EnvVarGuard::set(CATALOG_FALLBACK_URL_ENV, fallback_url.as_str());
        let _mock = server
            .mock("GET", "/catalog.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "providers": {
                        "openai": {
                            "default_model": "openai/gpt-5",
                            "models": {
                                "openai/gpt-5": {
                                    "display_name": "GPT-5",
                                    "family": "gpt",
                                    "context_window_tokens": 400000,
                                    "max_output_tokens": 128000
                                }
                            }
                        }
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let cache = RemoteModelCatalogCache::default();
        let source = RemoteModelCatalogSource::new(
            "shared-openai",
            "https://gateway.example/v1/models",
            "scope-a",
        )
        .with_catalog_provider_id("openai")
        .with_catalog_visible_model_prefix("openai");

        let models = cache
            .get_or_fetch(&source, || async {
                Err(AppError::Provider("boom".to_owned()))
            })
            .await
            .expect("catalog fallback should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider_id.as_str(), "shared-openai");
        assert_eq!(models[0].id.as_str(), "gpt-5");
        assert_eq!(models[0].display_name.as_deref(), Some("GPT-5"));
        assert_eq!(
            models[0].metadata.limits.context_window_tokens,
            Some(400000)
        );
        assert_eq!(models[0].metadata.limits.max_output_tokens, Some(128000));

        let cached = cache
            .get_or_fetch(&source, || async {
                Err(AppError::Provider(
                    "should not refetch after fallback cache".to_owned(),
                ))
            })
            .await
            .expect("fallback-seeded cache should be reused");
        assert_eq!(cached, models);
    }

    #[tokio::test]
    async fn falls_back_to_catalog_when_fetch_fails_without_cache_for_bedrock_visible_ids() {
        let _env_lock = env_lock().lock().expect("env lock should succeed");
        let dir = tempdir().expect("tempdir should create");
        let _cache_dir = EnvVarGuard::set(CACHE_DIR_ENV, dir.path().as_os_str());
        let mut server = Server::new_async().await;
        let fallback_url = format!("{}/catalog.json", server.url());
        let _fallback_url = EnvVarGuard::set(CATALOG_FALLBACK_URL_ENV, fallback_url.as_str());
        let _mock = server
            .mock("GET", "/catalog.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "providers": {
                        "bedrock": {
                            "default_model": "amazon_bedrock/amazon.nova-pro-v1:0",
                            "models": {
                                "amazon_bedrock/amazon.nova-pro-v1:0": {
                                    "display_name": "Amazon Nova Pro",
                                    "family": "nova"
                                },
                                "amazon_bedrock/anthropic.claude-sonnet-4-5": {
                                    "display_name": "Claude Sonnet 4.5",
                                    "family": "claude"
                                }
                            }
                        }
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let cache = RemoteModelCatalogCache::default();
        let source = RemoteModelCatalogSource::new(
            "amazon-bedrock",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/models",
            "scope-bedrock",
        )
        .with_catalog_provider_id("bedrock")
        .with_catalog_visible_model_prefix("amazon_bedrock");

        let models = cache
            .get_or_fetch(&source, || async {
                Err(AppError::Provider("boom".to_owned()))
            })
            .await
            .expect("bedrock catalog fallback should succeed");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].provider_id.as_str(), "amazon-bedrock");
        assert_eq!(models[0].id.as_str(), "amazon.nova-pro-v1:0");
        assert_eq!(models[0].display_name.as_deref(), Some("Amazon Nova Pro"));
        assert_eq!(
            models[0].metadata.family,
            Some(crate::model::ModelFamily::Nova)
        );
        assert_eq!(models[1].id.as_str(), "anthropic.claude-sonnet-4-5");
        assert_eq!(models[1].display_name.as_deref(), Some("Claude Sonnet 4.5"));
        assert_eq!(
            models[1].metadata.family,
            Some(crate::model::ModelFamily::Claude)
        );
    }
}
