use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agena_provider::{
    CatalogModelDefinition, ModelCatalogDocument, ModelCatalogSnapshotSourceKind,
};
use agena_storage::{ModelCatalogCacheRecord, ModelCatalogRepository, ModelCatalogRepositoryError};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};

static CATALOG_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

const CATALOG_KIND_OFFICIAL: &str = "official";
const CATALOG_STATE_ID: i32 = 1;
const ENTRY_TABLE: &str = "agena_model_catalog_entries";
const STATE_TABLE: &str = "agena_model_catalog_state";

fn backend_error(error: impl std::error::Error + 'static) -> ModelCatalogRepositoryError {
    ModelCatalogRepositoryError::Backend(agena_failure::diagnostic::format_error_chain(&error))
}

fn now_unix_ms() -> Result<i64, std::time::SystemTimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

fn model_catalog_definition_search_text(
    model_id: &str,
    definition: &CatalogModelDefinition,
    definition_json: &str,
) -> String {
    [
        model_id,
        definition.display_name.as_deref().unwrap_or_default(),
        definition.origin.as_deref().unwrap_or_default(),
        definition.description.as_deref().unwrap_or_default(),
        definition_json,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedOfficialCatalog {
    fetched_at_unix_ms: i64,
    source: ModelCatalogSnapshotSourceKind,
    document: ModelCatalogDocument,
}

/// SeaORM-backed model-catalog cache adapter.
///
/// The table schema is part of the current Agena SQLite
/// database. This adapter intentionally uses SQL rather than importing core
/// entities, so it can be composed without a dependency on `agena-core`.
#[derive(Clone)]
/// SQLite-backed model catalog repository.
pub struct SeaModelCatalogRepository {
    db: Arc<DatabaseConnection>,
}

impl SeaModelCatalogRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    async fn decode_document(
        rows: Vec<sea_orm::QueryResult>,
    ) -> Result<ModelCatalogDocument, ModelCatalogRepositoryError> {
        CATALOG_CODECS
            .run(move || {
                let mut models = BTreeMap::new();
                for row in rows {
                    let model_id: String = row.try_get("", "model_id").map_err(backend_error)?;
                    let raw: String = row.try_get("", "definition_json").map_err(backend_error)?;
                    let definition_json = serde_json::from_str(&raw).map_err(backend_error)?;
                    let definition = CatalogModelDefinition::from_persisted_json(definition_json)
                        .map_err(|error| {
                        ModelCatalogRepositoryError::Backend(format!(
                            "parse model catalog definition `{model_id}`/official: {error}"
                        ))
                    })?;
                    models.insert(model_id, definition);
                }
                Ok(ModelCatalogDocument { models })
            })
            .await
            .map_err(backend_error)?
    }

    async fn clear_cached_official_from_db(
        &self,
        expected_fetched_at: i64,
    ) -> Result<(), ModelCatalogRepositoryError> {
        let (txn, _permit) = crate::transaction::begin_with_write_lock(&self.db)
            .await
            .map_err(backend_error)?;
        let result = async {
            let row = txn
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    format!("SELECT fetched_at_unix_ms FROM {STATE_TABLE} WHERE id = ?"),
                    [CATALOG_STATE_ID.into()],
                ))
                .await
                .map_err(backend_error)?;
            let current: Option<i64> = row
                .map(|row| row.try_get("", "fetched_at_unix_ms"))
                .transpose()
                .map_err(backend_error)?
                .flatten();
            if current != Some(expected_fetched_at) {
                return Ok(());
            }

            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("DELETE FROM {ENTRY_TABLE} WHERE kind = ?"),
                [CATALOG_KIND_OFFICIAL.into()],
            ))
            .await
            .map_err(backend_error)?;
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("DELETE FROM {STATE_TABLE} WHERE id = ?"),
                [CATALOG_STATE_ID.into()],
            ))
            .await
            .map_err(backend_error)?;
            Ok::<(), ModelCatalogRepositoryError>(())
        }
        .await;
        match result {
            Ok(()) => txn.commit().await.map_err(backend_error),
            Err(error) => match txn.rollback().await {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(ModelCatalogRepositoryError::Backend(format!(
                    "{}; additionally, {}",
                    agena_failure::diagnostic::format_error_chain(&error),
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to roll back clearing the cached model catalog",
                        &rollback_error,
                    )
                ))),
            },
        }
    }

    fn prepare_document(
        document: ModelCatalogDocument,
    ) -> Result<Vec<Statement>, ModelCatalogRepositoryError> {
        let updated_at_ms = now_unix_ms().map_err(backend_error)?;
        let mut statements = Vec::new();
        let mut values = Vec::<sea_orm::Value>::new();
        let mut count = 0;
        for (model_id, definition) in document.models {
            let json = definition.to_persisted_json()?.to_string();
            let search = model_catalog_definition_search_text(&model_id, &definition, &json);
            values.extend([
                CATALOG_KIND_OFFICIAL.into(),
                model_id.into(),
                json.into(),
                search.into(),
                updated_at_ms.into(),
            ]);
            count += 1;
            if count == 100 {
                statements.push(Self::insert_statement(count, std::mem::take(&mut values)));
                count = 0;
            }
        }
        if count > 0 {
            statements.push(Self::insert_statement(count, values));
        }
        Ok(statements)
    }

    fn insert_statement(count: usize, values: Vec<sea_orm::Value>) -> Statement {
        Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {ENTRY_TABLE} (kind, model_id, definition_json, search_text, updated_at_ms) VALUES {}",
                vec!["(?, ?, ?, ?, ?)"; count].join(", ")
            ),
            values,
        )
    }

    async fn read_cached_official_from_db(
        &self,
    ) -> Result<Option<CachedOfficialCatalog>, ModelCatalogRepositoryError> {
        let txn = self.db.begin().await.map_err(backend_error)?;
        let Some(state) = txn
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT fetched_at_unix_ms, source FROM {STATE_TABLE} WHERE id = ? LIMIT 1"
                ),
                [CATALOG_STATE_ID.into()],
            ))
            .await
            .map_err(backend_error)?
        else {
            return Ok(None);
        };
        let rows = txn.query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("SELECT model_id, CAST(definition_json AS TEXT) AS definition_json FROM {ENTRY_TABLE} WHERE kind = ? ORDER BY model_id ASC"),
            [CATALOG_KIND_OFFICIAL.into()],
        )).await.map_err(backend_error)?;
        txn.commit().await.map_err(backend_error)?;
        let fetched_at_unix_ms: Option<i64> = state
            .try_get("", "fetched_at_unix_ms")
            .map_err(backend_error)?;
        let source_value: Option<String> = state.try_get("", "source").map_err(backend_error)?;
        let (Some(fetched_at_unix_ms), Some(source_value)) = (fetched_at_unix_ms, source_value)
        else {
            return Ok(None);
        };
        let source = match ModelCatalogSnapshotSourceKind::from_persisted(source_value.as_str()) {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(
                    source = %source_value,
                    error = %error,
                    "cached model catalog has an invalid source and will be cleared"
                );
                self.clear_cached_official_from_db(fetched_at_unix_ms)
                    .await?;
                return Ok(None);
            }
        };
        let document = match Self::decode_document(rows).await {
            Ok(document) => document,
            Err(ModelCatalogRepositoryError::Backend(error))
                if error.starts_with("parse model catalog definition") =>
            {
                self.clear_cached_official_from_db(fetched_at_unix_ms)
                    .await?;
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

    async fn write_cached_official_to_db(
        &self,
        cached: CachedOfficialCatalog,
    ) -> Result<(), ModelCatalogRepositoryError> {
        let fetched_at_unix_ms = cached.fetched_at_unix_ms;
        let source = cached.source;
        let statements = CATALOG_CODECS
            .run(move || Self::prepare_document(cached.document))
            .await
            .map_err(backend_error)??;
        let (txn, _permit) = crate::transaction::begin_with_write_lock(&self.db)
            .await
            .map_err(backend_error)?;
        let result = async {
            // Freshness gate: if another process already wrote a catalog
            // fetched at or after ours, skip the whole rewrite. SQLite's
            // single-writer transaction serializes concurrent writers, so the
            // check-and-write is atomic: the later process sees the earlier
            // commit and bails.
            let existing = txn
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    format!("SELECT fetched_at_unix_ms FROM {STATE_TABLE} WHERE id = ?"),
                    [CATALOG_STATE_ID.into()],
                ))
                .await
                .map_err(backend_error)?;
            let existing: Option<i64> = existing
                .map(|row| row.try_get("", "fetched_at_unix_ms").map_err(backend_error))
                .transpose()?;
            if existing.is_some_and(|value| value >= fetched_at_unix_ms) {
                return Ok(());
            }
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite, format!("DELETE FROM {ENTRY_TABLE} WHERE kind = ?"),
                [CATALOG_KIND_OFFICIAL.into()],
            )).await.map_err(backend_error)?;
            for statement in statements {
                txn.execute(statement).await.map_err(backend_error)?;
            }
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("DELETE FROM {STATE_TABLE} WHERE id = ?"),
                [CATALOG_STATE_ID.into()],
            ))
            .await
            .map_err(backend_error)?;
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "INSERT INTO {STATE_TABLE} (id, fetched_at_unix_ms, source, last_error, updated_at_ms) VALUES (?, ?, ?, NULL, ?)"
                ),
                [
                    CATALOG_STATE_ID.into(),
                    fetched_at_unix_ms.into(),
                    source.as_persisted().to_owned().into(),
                    now_unix_ms().map_err(backend_error)?.into(),
                ],
            ))
            .await
            .map_err(backend_error)?;
            Ok::<(), ModelCatalogRepositoryError>(())
        }
        .await;
        match result {
            Ok(()) => txn.commit().await.map_err(backend_error),
            Err(error) => match txn.rollback().await {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(ModelCatalogRepositoryError::Backend(format!(
                    "{}; additionally, {}",
                    agena_failure::diagnostic::format_error_chain(&error),
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to roll back writing the cached model catalog",
                        &rollback_error,
                    )
                ))),
            },
        }
    }
}

#[async_trait]
impl ModelCatalogRepository for SeaModelCatalogRepository {
    async fn read_cache(
        &self,
    ) -> Result<Option<ModelCatalogCacheRecord>, ModelCatalogRepositoryError> {
        let Some(cached) = self.read_cached_official_from_db().await? else {
            return Ok(None);
        };
        Ok(Some(ModelCatalogCacheRecord {
            fetched_at_unix_ms: cached.fetched_at_unix_ms,
            source: cached.source.as_persisted().to_owned(),
            document: CATALOG_CODECS
                .run(move || serde_json::to_value(cached.document))
                .await
                .map_err(backend_error)??,
        }))
    }

    async fn write_cache(
        &self,
        record: &ModelCatalogCacheRecord,
    ) -> Result<(), ModelCatalogRepositoryError> {
        let source = ModelCatalogSnapshotSourceKind::from_persisted(record.source.as_str())
            .map_err(ModelCatalogRepositoryError::Backend)?;
        let raw = record.document.clone();
        let document = CATALOG_CODECS
            .run(move || serde_json::from_value(raw))
            .await
            .map_err(backend_error)??;
        self.write_cached_official_to_db(CachedOfficialCatalog {
            fetched_at_unix_ms: record.fetched_at_unix_ms,
            source,
            document,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, Statement};

    async fn test_store() -> SeaModelCatalogRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        for sql in [
            // The write-lock fence (transaction.rs) inserts into agena_sequences,
            // so the fixture must provide that table alongside the catalog tables.
            "CREATE TABLE agena_sequences (seq_name TEXT PRIMARY KEY, next_val INTEGER NOT NULL)"
                .to_owned(),
            format!(
                "CREATE TABLE {ENTRY_TABLE} (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, model_id TEXT NOT NULL, definition_json JSON NOT NULL, search_text TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)"
            ),
            format!(
                "CREATE TABLE {STATE_TABLE} (id INTEGER PRIMARY KEY, fetched_at_unix_ms INTEGER NULL, source TEXT NULL, last_error TEXT NULL, updated_at_ms INTEGER NOT NULL)"
            ),
        ] {
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .expect("create catalog fixture table");
        }
        SeaModelCatalogRepository::new(Arc::new(db))
    }

    #[tokio::test]
    async fn writing_cache_replaces_entries_and_snapshot_state_in_one_operation() {
        let store = test_store().await;
        store
            .write_cache(&ModelCatalogCacheRecord {
                fetched_at_unix_ms: 1,
                source: "generated".to_owned(),
                document: serde_json::json!({
                    "models": {"old-model": {"display_name": "Old model"}}
                }),
            })
            .await
            .expect("seed catalog cache");
        store
            .write_cache(&ModelCatalogCacheRecord {
                fetched_at_unix_ms: 42,
                source: "cache".to_owned(),
                document: serde_json::json!({"models": {}}),
            })
            .await
            .expect("write catalog cache");
        let cached = store
            .read_cache()
            .await
            .expect("read catalog cache")
            .expect("cache exists");
        assert_eq!(cached.fetched_at_unix_ms, 42);
        assert_eq!(cached.source, "cache");
        assert_eq!(cached.document, serde_json::json!({}));
        let row = store
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("SELECT COUNT(*) AS count FROM {ENTRY_TABLE} WHERE kind = 'official'"),
            ))
            .await
            .expect("count replaced entries")
            .expect("count row");
        let count: i64 = row.try_get("", "count").expect("entry count");
        assert_eq!(count, 0, "replacement must remove stale entry rows");
    }

    #[tokio::test]
    async fn invalid_cache_source_is_discarded() {
        let store = test_store().await;
        store
            .db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("INSERT INTO {STATE_TABLE} (id, fetched_at_unix_ms, source, last_error, updated_at_ms) VALUES (1, 1, 'invalid', NULL, 1)"),
            ))
            .await
            .expect("insert invalid state");
        assert!(
            store
                .read_cache()
                .await
                .expect("read invalid cache")
                .is_none()
        );
        let state = store
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("SELECT id FROM {STATE_TABLE} WHERE id = 1"),
            ))
            .await
            .expect("query state");
        assert!(state.is_none());
    }
}
