use std::{
    fs,
    future::Future,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{error::AppError, model::Model};

use super::utils;

const CACHE_DIR_ENV: &str = "AGENA_PROVIDER_MODELS_CACHE_DIR";
const CACHE_TTL_ENV: &str = "AGENA_PROVIDER_MODELS_CACHE_TTL_SECS";
const DEFAULT_CACHE_TTL_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteModelCatalogSource {
    pub provider_id: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_scope: String,
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
        }
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
                Err(error)
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

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use tempfile::tempdir;

    use super::*;

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
    async fn returns_fetch_error_when_refresh_fails_without_cache() {
        let dir = tempdir().expect("tempdir should create");
        let cache = RemoteModelCatalogCache::with_root(dir.path());
        let source = RemoteModelCatalogSource::new(
            "shared-openai",
            "https://gateway.example/v1/models",
            "scope-a",
        );

        let error = cache
            .get_or_fetch(&source, || async {
                Err(AppError::Provider("boom".to_owned()))
            })
            .await
            .expect_err("refresh without cache should surface fetch error");

        assert!(matches!(error, AppError::Provider(message) if message == "boom"));
    }
}
