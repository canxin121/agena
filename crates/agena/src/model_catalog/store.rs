use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CachedOfficialCatalog {
    pub(super) fetched_at_unix_ms: i64,
    pub(super) source: ModelCatalogSnapshotSourceKind,
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

    async fn clear_cached_official_from_db(&self) -> Result<(), AppError> {
        let db = self.database();
        model_catalog_entry::Entity::delete_many()
            .filter(model_catalog_entry::Column::Kind.eq(CATALOG_KIND_OFFICIAL))
            .exec(db.as_ref())
            .await?;
        model_catalog_state::Entity::delete_by_id(CATALOG_STATE_ID)
            .exec(db.as_ref())
            .await?;
        Ok(())
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
        let Some(source_value) = state.source.as_deref() else {
            return Ok(None);
        };
        let source = match parse_catalog_source(source_value) {
            Ok(source) => source,
            Err(AppError::Config(_)) => {
                self.clear_cached_official_from_db().await?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let document = match self.read_document_from_db(CATALOG_KIND_OFFICIAL).await {
            Ok(document) => document,
            Err(AppError::Config(_)) | Err(AppError::SerdeJson(_)) => {
                self.clear_cached_official_from_db().await?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
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
use super::{
    AppError, Arc, BTreeMap, CATALOG_KIND_OFFICIAL, CATALOG_STATE_ID, CatalogModelDefinition,
    DatabaseConnection, Deserialize, ModelCatalogConfig, ModelCatalogDocument,
    ModelCatalogSnapshotSourceKind, Serialize, Set, format_catalog_source,
    model_catalog_definition_search_text, model_catalog_entry, model_catalog_state, now_unix_ms,
    parse_catalog_source,
};

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, PaginatorTrait};

    async fn test_store() -> ModelCatalogStore {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        crate::db::init_schema(&db).await.expect("schema");
        ModelCatalogStore::new(ModelCatalogConfig::default(), Arc::new(db))
    }

    async fn insert_cache(
        store: &ModelCatalogStore,
        source: &str,
        definition_json: serde_json::Value,
    ) {
        let db = store.database();
        model_catalog_entry::ActiveModel {
            kind: Set(CATALOG_KIND_OFFICIAL.to_owned()),
            model_id: Set("test-model".to_owned()),
            definition_json: Set(definition_json),
            search_text: Set(String::new()),
            updated_at_ms: Set(1),
            ..Default::default()
        }
        .insert(db.as_ref())
        .await
        .expect("catalog entry");
        model_catalog_state::ActiveModel {
            id: Set(CATALOG_STATE_ID),
            fetched_at_unix_ms: Set(Some(1)),
            source: Set(Some(source.to_owned())),
            last_error: Set(None),
            updated_at_ms: Set(1),
        }
        .insert(db.as_ref())
        .await
        .expect("catalog state");
    }

    async fn assert_cache_cleared(store: &ModelCatalogStore) {
        let db = store.database();
        assert_eq!(
            model_catalog_entry::Entity::find()
                .count(db.as_ref())
                .await
                .unwrap(),
            0
        );
        assert!(
            model_catalog_state::Entity::find_by_id(CATALOG_STATE_ID)
                .one(db.as_ref())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn obsolete_cache_format_is_discarded() {
        let store = test_store().await;
        insert_cache(&store, "generated", serde_json::json!({})).await;

        assert!(
            store
                .read_cached_official_from_db()
                .await
                .unwrap()
                .is_none()
        );
        assert_cache_cleared(&store).await;
    }

    #[tokio::test]
    async fn incompatible_cached_definition_is_discarded() {
        let store = test_store().await;
        let source = format!("{}:generated", super::super::CATALOG_CACHE_FORMAT_VERSION);
        insert_cache(
            &store,
            source.as_str(),
            serde_json::json!({
                "thinking_modes": {
                    "high": {
                        "thinking": { "type": "effort", "effort": "high" }
                    }
                }
            }),
        )
        .await;

        assert!(
            store
                .read_cached_official_from_db()
                .await
                .unwrap()
                .is_none()
        );
        assert_cache_cleared(&store).await;
    }
}
