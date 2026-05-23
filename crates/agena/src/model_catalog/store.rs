use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CachedOfficialCatalog {
    pub(super) fetched_at_unix_ms: i64,
    pub(super) source: ModelCatalogEntrySourceKind,
    pub(super) document: ModelCatalogDocument,
}

#[derive(Clone)]
pub struct ModelCatalogStore {
    config: ModelCatalogConfig,
    db: Arc<DatabaseConnection>,
}

impl ModelCatalogStore {
    pub fn new(config: ModelCatalogConfig, db: Arc<DatabaseConnection>) -> Self {
        Self { config, db }
    }

    pub fn config(&self) -> &ModelCatalogConfig {
        &self.config
    }

    fn database(&self) -> &Arc<DatabaseConnection> {
        &self.db
    }

    pub(super) async fn read_cached_official(
        &self,
    ) -> Result<Option<CachedOfficialCatalog>, AppError> {
        self.read_cached_official_from_db().await
    }

    pub(super) async fn write_cached_official(
        &self,
        cached: &CachedOfficialCatalog,
    ) -> Result<(), AppError> {
        self.write_cached_official_to_db(cached).await
    }

    async fn read_document_from_db(&self, kind: &str) -> Result<ModelCatalogDocument, AppError> {
        let db = self.database();
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
        let db = self.database();
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
        let db = self.database();
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
        let db = self.database();
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
