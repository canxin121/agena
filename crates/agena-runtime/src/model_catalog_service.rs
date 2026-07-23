use std::sync::Arc;

use agena_provider::{
    ModelCatalogProviderRecord, ModelCatalogSnapshot, ModelCatalogSnapshotSourceKind,
    ProviderModelPriorities, ProviderModelSource,
};
use agena_storage::ModelCatalogRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;

use crate::{
    ModelCatalogCompositionError, ModelCatalogPublicSourceResult, SnapshotStore,
    build_live_provider_catalog_document, compose_model_catalog_document,
    model_catalog_cache_record_from_document, model_catalog_snapshot_from_cache_record,
    should_refresh,
};

/// Concrete HTTP/source adapters provide public catalog data through this port.
#[async_trait]
pub trait ModelCatalogPublicSource: Send + Sync {
    async fn fetch(&self) -> ModelCatalogPublicSourceResult;
}

/// Failure while restoring, refreshing, or persisting a runtime catalog.
#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogServiceError {
    #[error("model catalog initialization error: {0}")]
    Initialization(String),
    #[error("model catalog repository error: {0}")]
    Repository(String),
    #[error(transparent)]
    Cache(#[from] crate::ModelCatalogCacheCodecError),
    #[error(transparent)]
    Composition(#[from] ModelCatalogCompositionError),
}

#[derive(Clone)]
pub struct ModelCatalogService {
    store: Arc<dyn ModelCatalogRepository>,
    cache_max_age_secs: u64,
    state: Arc<SnapshotStore<ModelCatalogSnapshot>>,
    public_source: Arc<dyn ModelCatalogPublicSource>,
}

impl ModelCatalogService {
    pub async fn new(
        store: Arc<dyn ModelCatalogRepository>,
        cache_max_age_secs: u64,
        public_source: Arc<dyn ModelCatalogPublicSource>,
    ) -> Result<Self, ModelCatalogServiceError> {
        let snapshot = match store
            .read_cache()
            .await
            .map_err(|error| ModelCatalogServiceError::Repository(error.to_string()))?
        {
            Some(record) => model_catalog_snapshot_from_cache_record(&record)?,
            None => ModelCatalogSnapshot::default(),
        };
        Ok(Self {
            store,
            cache_max_age_secs,
            state: Arc::new(SnapshotStore::new(Arc::new(snapshot))),
            public_source,
        })
    }

    /// Compose the default persisted catalog service used by a Runtime process.
    /// The concrete provider registry remains a caller-supplied source at
    /// refresh time; Runtime owns cache storage, policy, and public catalog
    /// source initialization.
    pub async fn compose_default(
        database: Arc<DatabaseConnection>,
    ) -> Result<Arc<Self>, ModelCatalogServiceError> {
        let store = Arc::new(agena_storage_sqlite::SeaModelCatalogRepository::new(
            database,
        ));
        let public_source = crate::build_default_public_model_catalog_source(
            crate::runtime_codex_user_agent(env!("CARGO_PKG_VERSION")),
        )
        .map_err(|error| ModelCatalogServiceError::Initialization(error.to_string()))?;
        Ok(Arc::new(
            Self::new(
                store,
                crate::ModelCatalogRuntimeConfig::default().cache_max_age_secs,
                public_source,
            )
            .await?,
        ))
    }

    /// Compose the default persisted catalog when the process has a database.
    ///
    /// Snapshot composition is allowed to run without persistence (for
    /// example, lightweight configuration inspection), so the optional
    /// database decision belongs to Runtime rather than every concrete
    /// composition adapter.  Keeping the missing-database diagnostic here
    /// also prevents callers from manufacturing a concrete configuration
    /// error for a Runtime-owned service.
    pub async fn compose_default_optional(
        database: Option<Arc<DatabaseConnection>>,
    ) -> Result<Arc<Self>, ModelCatalogServiceError> {
        let database = database.ok_or_else(|| {
            ModelCatalogServiceError::Initialization(
                "runtime database connection missing".to_owned(),
            )
        })?;
        Self::compose_default(database).await
    }

    pub fn snapshot(&self) -> ModelCatalogSnapshot {
        (*self.state.current()).clone()
    }

    pub fn effective_provider_record(
        &self,
        _adapter_ids: &[String],
    ) -> Option<ModelCatalogProviderRecord> {
        let record = self.snapshot().merged_models();
        (!record.models.is_empty()).then_some(record)
    }

    pub fn needs_startup_refresh(&self) -> bool {
        self.snapshot_needs_startup_refresh(&self.snapshot())
    }

    pub async fn refresh_from_source(
        &self,
        providers: &dyn ProviderModelSource,
        provider_priorities: Option<&ProviderModelPriorities>,
    ) -> Result<ModelCatalogSnapshot, ModelCatalogServiceError> {
        let public = self.public_source.fetch().await;
        let live = build_live_provider_catalog_document(providers, provider_priorities).await;
        let (document, warnings) = compose_model_catalog_document(public, live)?;
        let fetched_at_unix_ms = now_unix_ms();
        let record = model_catalog_cache_record_from_document(
            fetched_at_unix_ms,
            ModelCatalogSnapshotSourceKind::Generated,
            &document,
        )?;
        self.store
            .write_cache(&record)
            .await
            .map_err(|error| ModelCatalogServiceError::Repository(error.to_string()))?;

        let mut snapshot = self.snapshot();
        snapshot.official = document;
        snapshot.last_successful_source = Some(ModelCatalogSnapshotSourceKind::Generated);
        snapshot.last_refresh_at = DateTime::<Utc>::from_timestamp_millis(fetched_at_unix_ms);
        snapshot.last_error = warnings;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub fn record_refresh_failure(&self, error: impl Into<String>) {
        let mut snapshot = self.snapshot();
        snapshot.last_error = Some(error.into());
        self.replace_snapshot(snapshot);
    }

    fn snapshot_needs_startup_refresh(&self, snapshot: &ModelCatalogSnapshot) -> bool {
        should_refresh(
            !snapshot.official.model_ids().is_empty(),
            snapshot.last_refresh_at,
            chrono::Duration::seconds(self.cache_max_age_secs as i64),
        )
    }

    fn replace_snapshot(&self, snapshot: ModelCatalogSnapshot) {
        self.state.swap(Arc::new(snapshot));
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agena_storage::{
        ModelCatalogCacheRecord, ModelCatalogRepository, ModelCatalogRepositoryError,
    };
    use async_trait::async_trait;

    use super::{ModelCatalogPublicSource, ModelCatalogPublicSourceResult, ModelCatalogService};

    #[derive(Default)]
    struct MemoryCatalogRepository {
        record: Mutex<Option<ModelCatalogCacheRecord>>,
    }

    #[async_trait]
    impl ModelCatalogRepository for MemoryCatalogRepository {
        async fn read_cache(
            &self,
        ) -> Result<Option<ModelCatalogCacheRecord>, ModelCatalogRepositoryError> {
            Ok(self.record.lock().expect("cache record lock").clone())
        }

        async fn write_cache(
            &self,
            record: &ModelCatalogCacheRecord,
        ) -> Result<(), ModelCatalogRepositoryError> {
            *self.record.lock().expect("cache record lock") = Some(record.clone());
            Ok(())
        }
    }

    struct EmptyPublicSource;

    #[async_trait]
    impl ModelCatalogPublicSource for EmptyPublicSource {
        async fn fetch(&self) -> ModelCatalogPublicSourceResult {
            ModelCatalogPublicSourceResult {
                models: Default::default(),
                warnings: Vec::new(),
                succeeded: 0,
            }
        }
    }

    #[tokio::test]
    async fn optional_constructor_reports_missing_database_at_runtime_boundary() {
        let error = match ModelCatalogService::compose_default_optional(None).await {
            Ok(_) => panic!("missing persistence must be reported by Runtime"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "model catalog initialization error: runtime database connection missing"
        );
    }

    #[tokio::test]
    async fn constructor_restores_cache_through_the_storage_port() {
        let service = ModelCatalogService::new(
            Arc::new(MemoryCatalogRepository {
                record: Mutex::new(Some(ModelCatalogCacheRecord {
                    fetched_at_unix_ms: 123,
                    source: "cache".to_owned(),
                    document: serde_json::json!({ "models": {} }),
                })),
            }),
            60,
            Arc::new(EmptyPublicSource),
        )
        .await
        .expect("construct catalog service");
        assert_eq!(
            service
                .snapshot()
                .last_refresh_at
                .expect("cache timestamp")
                .timestamp_millis(),
            123
        );
    }

    #[tokio::test]
    async fn refresh_failure_replaces_the_runtime_snapshot() {
        let service = ModelCatalogService::new(
            Arc::new(MemoryCatalogRepository::default()),
            60,
            Arc::new(EmptyPublicSource),
        )
        .await
        .expect("construct catalog service");
        service.record_refresh_failure("source unavailable");
        assert_eq!(
            service.snapshot().last_error.as_deref(),
            Some("source unavailable")
        );
    }
}
