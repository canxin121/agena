use super::store::CachedOfficialCatalog;
use super::*;

#[derive(Clone)]
pub struct ModelCatalogService {
    store: ModelCatalogStore,
    state: Arc<RwLock<ModelCatalogSnapshot>>,
    client: reqwest::Client,
    remote_sources: Vec<sources::ModelCatalogRemoteSource>,
}

impl ModelCatalogService {
    pub async fn new(store: ModelCatalogStore) -> Result<Self, AppError> {
        Self::with_remote_sources(store, default_remote_sources()).await
    }

    pub(super) async fn with_remote_sources(
        store: ModelCatalogStore,
        remote_sources: Vec<sources::ModelCatalogRemoteSource>,
    ) -> Result<Self, AppError> {
        store.migrate_legacy_files_if_needed().await?;
        let custom = store.read_custom().await?;
        let mut snapshot = ModelCatalogSnapshot {
            custom,
            ..ModelCatalogSnapshot::default()
        };

        if let Some(cached) = store.read_cached_official().await? {
            snapshot.last_successful_source = Some(cached.source);
            snapshot.last_refresh_at =
                DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms);
            snapshot.official = cached.document;
        }

        Ok(Self {
            store,
            state: Arc::new(RwLock::new(snapshot)),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .user_agent(format!("agena/{} model-catalog", env!("CARGO_PKG_VERSION")))
                .build()?,
            remote_sources,
        })
    }

    pub fn snapshot(&self) -> ModelCatalogSnapshot {
        self.state
            .read()
            .expect("catalog state lock poisoned")
            .clone()
    }

    pub fn effective_provider_record(
        &self,
        _adapter_ids: &[String],
    ) -> Option<ModelCatalogProviderRecord> {
        let record = self.snapshot().merged_models();
        (!record.models.is_empty()).then_some(record)
    }

    pub async fn refresh_if_stale_on_startup(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let snapshot = self.snapshot();
        if self.snapshot_needs_startup_refresh(&snapshot) {
            self.refresh_from_registry(providers, resolution).await
        } else {
            Ok(snapshot)
        }
    }

    pub async fn refresh_from_registry(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let (document, warnings) = match self.build_catalog_document(providers, resolution).await {
            Ok(result) => result,
            Err(refresh_error) => {
                if let Some(cached) = self.store.read_cached_official().await? {
                    let cache_is_fresh = self.cache_is_fresh(&cached);
                    let mut snapshot = self.snapshot();
                    snapshot.official = cached.document;
                    snapshot.last_successful_source = Some(ModelCatalogEntrySourceKind::Cache);
                    snapshot.last_refresh_at =
                        DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms);
                    snapshot.last_error = Some(format!(
                        "catalog refresh failed: {refresh_error}; using {}cache",
                        if cache_is_fresh { "" } else { "stale " }
                    ));
                    self.replace_snapshot(snapshot.clone());
                    return Ok(snapshot);
                }
                return Err(refresh_error);
            }
        };
        let fetched_at_unix_ms = now_unix_ms();
        self.store
            .write_cached_official(&CachedOfficialCatalog {
                fetched_at_unix_ms,
                source: ModelCatalogEntrySourceKind::Generated,
                document: document.clone(),
            })
            .await?;

        let mut snapshot = self.snapshot();
        snapshot.official = document;
        snapshot.last_successful_source = Some(ModelCatalogEntrySourceKind::Generated);
        snapshot.last_refresh_at = DateTime::<Utc>::from_timestamp_millis(fetched_at_unix_ms);
        snapshot.last_error = warnings;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub async fn upsert_custom_entry(
        &self,
        model_id: impl Into<String>,
        definition: CatalogModelDefinition,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let model_id = model_id.into();
        let mut snapshot = self.snapshot();
        snapshot.custom.models.insert(model_id, definition);
        self.store.write_custom(&snapshot.custom).await?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub async fn remove_custom_entry(
        &self,
        model_id: &str,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let mut snapshot = self.snapshot();
        snapshot.custom.models.remove(model_id);
        self.store.write_custom(&snapshot.custom).await?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    async fn build_catalog_document(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<(ModelCatalogDocument, Option<String>), AppError> {
        let mut merged = ModelCatalogDocument::default();
        let mut warnings = Vec::new();
        let mut succeeded = 0_usize;

        let (remote_documents, remote_warnings) =
            sources::fetch_documents(&self.client, &self.remote_sources).await;
        warnings.extend(remote_warnings);
        let mut remote_documents = remote_documents;
        remote_documents.sort_by(|left, right| {
            right
                .kind
                .priority()
                .cmp(&left.kind.priority())
                .then_with(|| left.name.cmp(&right.name))
        });
        for fetched in remote_documents {
            merge_public_source_catalog_document(&mut merged, fetched.document);
            succeeded += 1;
        }

        match self
            .build_live_provider_catalog_document(providers, resolution)
            .await
        {
            Ok((Some(document), warning)) => {
                merge_catalog_document(&mut merged, document);
                succeeded += 1;
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Ok((None, warning)) => {
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Err(error) => {
                warnings.push(format!("live provider model list: {error}"));
                if succeeded == 0 {
                    return Err(error);
                }
            }
        }

        if succeeded == 0 {
            let detail = if warnings.is_empty() {
                "no public catalog sources or live provider model lists succeeded".to_owned()
            } else {
                warnings.join("; ")
            };
            return Err(AppError::Config(format!(
                "model catalog generation failed: {detail}"
            )));
        }

        let mut document = curate::curate_catalog_document(merged)
            .map_err(|err| AppError::Config(format!("curate generated model catalog: {err}")))?;
        sources::enrich_catalog_document_thinking_modes(&mut document);
        let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
        Ok((document, warning))
    }

    async fn build_live_provider_catalog_document(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<(Option<ModelCatalogDocument>, Option<String>), AppError> {
        let mut provider_ids = providers.provider_ids();
        provider_ids.sort_by(|left, right| {
            provider_priority(right.as_str(), resolution)
                .cmp(&provider_priority(left.as_str(), resolution))
                .then_with(|| left.cmp(right))
        });

        if provider_ids.is_empty() {
            return Ok((None, None));
        }

        let mut raw_models = BTreeMap::<String, CatalogModelDefinition>::new();
        let mut errors = Vec::new();
        let mut succeeded = 0_usize;

        for provider_id in provider_ids {
            match providers.list_models(provider_id.as_str()).await {
                Ok(models) => {
                    succeeded += 1;
                    for model in models {
                        if model.id.as_str().trim().is_empty() {
                            continue;
                        }
                        let definition = catalog_definition_from_model(&model);
                        raw_models
                            .entry(model.id.to_string())
                            .and_modify(|current| merge_catalog_definition(current, &definition))
                            .or_insert(definition);
                    }
                }
                Err(error) => errors.push(format!("{provider_id}: {error}")),
            }
        }

        if succeeded == 0 {
            let detail = if errors.is_empty() {
                return Ok((None, None));
            } else {
                errors.join("; ")
            };
            return Err(AppError::Config(format!(
                "live provider model list failed: {detail}"
            )));
        }

        let document = curate::curate_catalog_document(ModelCatalogDocument { models: raw_models })
            .map_err(|err| {
                AppError::Config(format!("curate live provider model catalog: {err}"))
            })?;
        let warning = (!errors.is_empty()).then(|| {
            format!(
                "live provider model lists generated catalog from {succeeded} provider(s); skipped {} provider(s): {}",
                errors.len(),
                errors.join("; ")
            )
        });
        Ok((Some(document), warning))
    }

    fn cache_is_fresh(&self, cached: &CachedOfficialCatalog) -> bool {
        let fetched = UNIX_EPOCH + Duration::from_millis(cached.fetched_at_unix_ms.max(0) as u64);
        SystemTime::now()
            .duration_since(fetched)
            .map(|age| age.as_secs() <= self.store.config().cache_max_age_secs)
            .unwrap_or(false)
    }

    fn snapshot_needs_startup_refresh(&self, snapshot: &ModelCatalogSnapshot) -> bool {
        if snapshot.official.model_ids().is_empty() {
            return true;
        }

        let Some(last_refresh_at) = snapshot.last_refresh_at else {
            return true;
        };

        match SystemTime::now().duration_since(last_refresh_at.into()) {
            Ok(age) => age.as_secs() > self.store.config().cache_max_age_secs,
            Err(_) => false,
        }
    }

    fn replace_snapshot(&self, snapshot: ModelCatalogSnapshot) {
        let mut guard = self.state.write().expect("catalog state lock poisoned");
        *guard = snapshot;
    }
}
