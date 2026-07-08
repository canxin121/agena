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
        Self::from_remote_sources(store, default_remote_sources()).await
    }

    pub(super) async fn from_remote_sources(
        store: ModelCatalogStore,
        remote_sources: Vec<sources::ModelCatalogRemoteSource>,
    ) -> Result<Self, AppError> {
        let snapshot = match store.read_cached_official().await? {
            Some(cached) => ModelCatalogSnapshot {
                last_refresh_at: DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms),
                last_successful_source: Some(cached.source),
                last_error: None,
                official: cached.document,
            },
            None => ModelCatalogSnapshot::default(),
        };

        Ok(Self {
            store,
            state: Arc::new(RwLock::new(snapshot)),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .user_agent(crate::provider::codex_user_agent())
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

    pub fn needs_startup_refresh(&self) -> bool {
        let snapshot = self.snapshot();
        self.snapshot_needs_startup_refresh(&snapshot)
    }

    pub async fn refresh_from_registry(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let (document, warnings) = self.build_catalog_document(providers, resolution).await?;
        let fetched_at_unix_ms = now_unix_ms();
        self.store
            .write_cached_official(&CachedOfficialCatalog {
                fetched_at_unix_ms,
                source: ModelCatalogSnapshotSourceKind::Generated,
                document: document.clone(),
            })
            .await?;

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

    async fn build_catalog_document(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<(ModelCatalogDocument, Option<String>), AppError> {
        let mut merged_models = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut succeeded = 0_usize;
        let mut has_live_provider_models = false;

        let (remote_documents, remote_warnings) =
            sources::fetch_documents(&self.client, &self.remote_sources).await;
        warnings.extend(remote_warnings);
        let mut remote_documents = remote_documents;
        remote_documents.sort_by(|left, right| {
            right
                .grade
                .sort_priority
                .cmp(&left.grade.sort_priority)
                .then_with(|| right.kind.priority().cmp(&left.kind.priority()))
                .then_with(|| left.name.cmp(&right.name))
        });
        for fetched in remote_documents {
            merge_public_source_catalog_document(&mut merged_models, fetched.document);
            succeeded += 1;
        }

        match self
            .build_live_provider_catalog_document(providers, resolution)
            .await
        {
            Ok((Some(document), warning)) => {
                merge_live_provider_catalog_document(&mut merged_models, document);
                has_live_provider_models = true;
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

        let merged = ModelCatalogDocument {
            models: merged_models,
        };
        let mut document = if has_live_provider_models {
            curate::curate_live_catalog_document(merged)
        } else {
            curate::curate_catalog_document(merged)
        }
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
                        if model.id.as_ref().trim().is_empty() {
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

        let document =
            curate::curate_live_catalog_document(ModelCatalogDocument { models: raw_models })
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
