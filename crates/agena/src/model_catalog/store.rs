use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CachedOfficialCatalog {
    pub(super) fetched_at_unix_ms: i64,
    pub(super) source: ModelCatalogEntrySourceKind,
    pub(super) document: ModelCatalogDocument,
}

#[derive(Clone)]
enum ModelCatalogStoreBackend {
    File(ModelCatalogConfig),
    Database {
        config: ModelCatalogConfig,
        db: Arc<DatabaseConnection>,
    },
}

#[derive(Clone)]
pub struct ModelCatalogStore {
    backend: ModelCatalogStoreBackend,
}

impl ModelCatalogStore {
    pub fn new(config: ModelCatalogConfig) -> Self {
        Self {
            backend: ModelCatalogStoreBackend::File(config),
        }
    }

    pub fn new_database(config: ModelCatalogConfig, db: Arc<DatabaseConnection>) -> Self {
        Self {
            backend: ModelCatalogStoreBackend::Database { config, db },
        }
    }

    pub fn config(&self) -> &ModelCatalogConfig {
        match &self.backend {
            ModelCatalogStoreBackend::File(config) => config,
            ModelCatalogStoreBackend::Database { config, .. } => config,
        }
    }

    fn database(&self) -> Option<&Arc<DatabaseConnection>> {
        match &self.backend {
            ModelCatalogStoreBackend::File(_) => None,
            ModelCatalogStoreBackend::Database { db, .. } => Some(db),
        }
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<Option<T>, AppError> {
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(path)?;
        let parsed = serde_json::from_str(&text)
            .map_err(|err| AppError::Config(format!("parse {}: {err}", path.display())))?;
        Ok(Some(parsed))
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(value).map_err(AppError::from)?;
        fs::write(path, format!("{text}\n"))?;
        Ok(())
    }

    pub(super) async fn migrate_legacy_files_if_needed(&self) -> Result<(), AppError> {
        let Some(db) = self.database() else {
            return Ok(());
        };

        let has_any_entries = model_catalog_entry::Entity::find()
            .one(db.as_ref())
            .await?
            .is_some();
        let has_state = model_catalog_state::Entity::find_by_id(CATALOG_STATE_ID)
            .one(db.as_ref())
            .await?
            .is_some();
        if has_any_entries || has_state {
            return Ok(());
        }

        if let Some(custom) = self.read_json::<ModelCatalogDocument>(&self.config().custom_path)? {
            self.write_document_to_db(CATALOG_KIND_CUSTOM, &custom)
                .await?;
        }
        if let Some(cached) = self.read_json::<CachedOfficialCatalog>(&self.config().cache_path)? {
            self.write_cached_official_to_db(&cached).await?;
        }

        Ok(())
    }

    pub async fn read_custom(&self) -> Result<ModelCatalogDocument, AppError> {
        if self.database().is_some() {
            return self.read_document_from_db(CATALOG_KIND_CUSTOM).await;
        }
        Ok(self
            .read_json(&self.config().custom_path)?
            .unwrap_or_default())
    }

    pub async fn write_custom(&self, document: &ModelCatalogDocument) -> Result<(), AppError> {
        if self.database().is_some() {
            return self
                .write_document_to_db(CATALOG_KIND_CUSTOM, document)
                .await;
        }
        self.write_json(&self.config().custom_path, document)
    }

    pub(super) async fn read_cached_official(
        &self,
    ) -> Result<Option<CachedOfficialCatalog>, AppError> {
        if self.database().is_some() {
            return self.read_cached_official_from_db().await;
        }
        self.read_json(&self.config().cache_path)
    }

    pub(super) async fn write_cached_official(
        &self,
        cached: &CachedOfficialCatalog,
    ) -> Result<(), AppError> {
        if self.database().is_some() {
            return self.write_cached_official_to_db(cached).await;
        }
        self.write_json(&self.config().cache_path, cached)
    }

    async fn read_document_from_db(&self, kind: &str) -> Result<ModelCatalogDocument, AppError> {
        let db = self
            .database()
            .expect("database-backed catalog store should have a database");
        let rows = model_catalog_entry::Entity::find()
            .filter(model_catalog_entry::Column::Kind.eq(kind))
            .order_by_asc(model_catalog_entry::Column::ModelId)
            .all(db.as_ref())
            .await?;
        let mut models = BTreeMap::new();
        for row in rows {
            let definition = serde_json::from_value::<CatalogModelDefinition>(row.definition_json)
                .map_err(|err| {
                    AppError::Config(format!(
                        "parse model catalog definition `{}`/{kind}: {err}",
                        row.model_id
                    ))
                })?;
            models.insert(row.model_id, definition);
        }
        Ok(ModelCatalogDocument { models })
    }

    async fn write_document_to_db(
        &self,
        kind: &str,
        document: &ModelCatalogDocument,
    ) -> Result<(), AppError> {
        let db = self
            .database()
            .expect("database-backed catalog store should have a database");
        model_catalog_entry::Entity::delete_many()
            .filter(model_catalog_entry::Column::Kind.eq(kind))
            .exec(db.as_ref())
            .await?;

        let updated_at_ms = now_unix_ms();
        let rows = document
            .models
            .iter()
            .map(|(model_id, definition)| {
                Ok(model_catalog_entry::ActiveModel {
                    kind: Set(kind.to_owned()),
                    model_id: Set(model_id.clone()),
                    definition_json: Set(serde_json::to_value(definition)?),
                    search_text: Set(model_catalog_definition_search_text(model_id, definition)),
                    updated_at_ms: Set(updated_at_ms),
                    ..Default::default()
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        if !rows.is_empty() {
            model_catalog_entry::Entity::insert_many(rows)
                .exec(db.as_ref())
                .await?;
        }
        Ok(())
    }

    pub(super) async fn read_cached_official_from_db(
        &self,
    ) -> Result<Option<CachedOfficialCatalog>, AppError> {
        let db = self
            .database()
            .expect("database-backed catalog store should have a database");
        let Some(state) = model_catalog_state::Entity::find_by_id(CATALOG_STATE_ID)
            .one(db.as_ref())
            .await?
        else {
            return Ok(None);
        };
        let Some(fetched_at_unix_ms) = state.fetched_at_unix_ms else {
            return Ok(None);
        };
        let Some(source) = state
            .source
            .as_deref()
            .map(parse_catalog_source)
            .transpose()?
        else {
            return Ok(None);
        };
        let document = self.read_document_from_db(CATALOG_KIND_OFFICIAL).await?;
        Ok(Some(CachedOfficialCatalog {
            fetched_at_unix_ms,
            source,
            document,
        }))
    }

    pub(super) async fn write_cached_official_to_db(
        &self,
        cached: &CachedOfficialCatalog,
    ) -> Result<(), AppError> {
        let db = self
            .database()
            .expect("database-backed catalog store should have a database");
        self.write_document_to_db(CATALOG_KIND_OFFICIAL, &cached.document)
            .await?;
        model_catalog_state::Entity::delete_by_id(CATALOG_STATE_ID)
            .exec(db.as_ref())
            .await?;
        model_catalog_state::ActiveModel {
            id: Set(CATALOG_STATE_ID),
            fetched_at_unix_ms: Set(Some(cached.fetched_at_unix_ms)),
            source: Set(Some(format_catalog_source(cached.source))),
            last_error: Set(None),
            updated_at_ms: Set(now_unix_ms()),
        }
        .insert(db.as_ref())
        .await?;
        Ok(())
    }
}
