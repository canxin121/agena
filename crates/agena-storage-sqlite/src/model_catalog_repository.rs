use std::{collections::BTreeMap, sync::Arc};

use agena_provider::{
    CatalogModelDefinition, ModelCatalogDocument, ModelCatalogSnapshotSourceKind,
};
use agena_storage::{ModelCatalogCacheRecord, ModelCatalogRepository, ModelCatalogRepositoryError};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use sha2::{Digest, Sha256};

static CATALOG_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

const ENTRY_TABLE: &str = "agena_model_catalog_entries";

fn backend_error(error: impl std::error::Error + 'static) -> ModelCatalogRepositoryError {
    ModelCatalogRepositoryError::Backend(agena_failure::diagnostic::format_error_chain(&error))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedOfficialCatalog {
    fetched_at_unix_ms: i64,
    source: ModelCatalogSnapshotSourceKind,
    document: ModelCatalogDocument,
}

struct PreparedCatalog {
    model_ids: String,
    upserts: Vec<Statement>,
    digest: [u8; 32],
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
    reader: Arc<DatabaseConnection>,
}

impl SeaModelCatalogRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self::with_reader(db.clone(), db)
    }

    pub fn with_reader(db: Arc<DatabaseConnection>, reader: Arc<DatabaseConnection>) -> Self {
        Self { db, reader }
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
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "SELECT catalog_fetched_at_ms FROM agena_instance WHERE id=1",
                ))
                .await
                .map_err(backend_error)?;
            let current: Option<i64> = row
                .map(|row| row.try_get("", "catalog_fetched_at_ms"))
                .transpose()
                .map_err(backend_error)?
                .flatten();
            if current != Some(expected_fetched_at) {
                return Ok(());
            }

            txn.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("DELETE FROM {ENTRY_TABLE}"),
            ))
            .await
            .map_err(backend_error)?;
            txn.execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "UPDATE agena_instance SET catalog_fetched_at_ms=NULL,catalog_source=NULL,catalog_digest=NULL WHERE id=1",
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
    ) -> Result<PreparedCatalog, ModelCatalogRepositoryError> {
        let model_ids = serde_json::to_string(&document.models.keys().collect::<Vec<_>>())
            .map_err(backend_error)?;
        let mut statements = Vec::new();
        let mut values = Vec::<sea_orm::Value>::new();
        let mut count = 0;
        let mut digest = Sha256::new();
        for (model_id, definition) in document.models {
            let json = definition.to_persisted_json()?.to_string();
            for field in [&model_id, &json] {
                digest.update((field.len() as u64).to_le_bytes());
                digest.update(field.as_bytes());
            }
            values.extend([model_id.into(), json.into()]);
            count += 1;
            if count == 100 {
                statements.push(Self::upsert_statement(count, std::mem::take(&mut values)));
                count = 0;
            }
        }
        if count > 0 {
            statements.push(Self::upsert_statement(count, values));
        }
        Ok(PreparedCatalog {
            model_ids,
            upserts: statements,
            digest: digest.finalize().into(),
        })
    }

    fn upsert_statement(count: usize, values: Vec<sea_orm::Value>) -> Statement {
        Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO {ENTRY_TABLE} (model_id, definition_json) VALUES {}
                 ON CONFLICT(model_id) DO UPDATE SET definition_json=excluded.definition_json
                 WHERE definition_json IS NOT excluded.definition_json",
                vec!["(?, ?)"; count].join(", ")
            ),
            values,
        )
    }

    async fn read_cached_official_from_db(
        &self,
    ) -> Result<Option<CachedOfficialCatalog>, ModelCatalogRepositoryError> {
        let txn = self.reader.begin().await.map_err(backend_error)?;
        let Some(state) = txn
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT catalog_fetched_at_ms,catalog_source FROM agena_instance WHERE id=1",
            ))
            .await
            .map_err(backend_error)?
        else {
            return Ok(None);
        };
        let Some(fetched_at_unix_ms) = state
            .try_get::<Option<i64>>("", "catalog_fetched_at_ms")
            .map_err(backend_error)?
        else {
            return Ok(None);
        };
        let rows = txn
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("SELECT model_id, definition_json FROM {ENTRY_TABLE} ORDER BY model_id"),
            ))
            .await
            .map_err(backend_error)?;
        txn.commit().await.map_err(backend_error)?;
        let source_value: String = state.try_get("", "catalog_source").map_err(backend_error)?;
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
        let PreparedCatalog {
            model_ids,
            upserts,
            digest,
        } = CATALOG_CODECS
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
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "SELECT catalog_fetched_at_ms,catalog_digest FROM agena_instance WHERE id=1",
                ))
                .await
                .map_err(backend_error)?
                .ok_or_else(|| ModelCatalogRepositoryError::Backend("instance identity is missing".into()))?;
            let previous_fetched_at: Option<i64> = existing
                .try_get("", "catalog_fetched_at_ms")
                .map_err(backend_error)?;
            if previous_fetched_at.is_some_and(|time| time >= fetched_at_unix_ms) {
                return Ok(());
            }
            let previous_digest: Option<Vec<u8>> = existing
                .try_get("", "catalog_digest")
                .map_err(backend_error)?;
            if previous_digest.as_deref() != Some(digest.as_slice()) {
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    format!("DELETE FROM {ENTRY_TABLE} WHERE model_id NOT IN (SELECT value FROM json_each(?))"),
                    [model_ids.into()],
                )).await.map_err(backend_error)?;
                for statement in upserts {
                    txn.execute(statement).await.map_err(backend_error)?;
                }
            }
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_instance SET catalog_fetched_at_ms=?,catalog_source=?,catalog_digest=? WHERE id=1",
                [fetched_at_unix_ms.into(), source.as_persisted().into(), digest.to_vec().into()],
            )).await.map_err(backend_error)?;
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
    use serde_json::json;

    #[tokio::test]
    async fn catalog_replacement_and_freshness_share_one_snapshot() {
        let database = crate::KernelDatabase::open("sqlite::memory:")
            .await
            .unwrap();
        let repository = SeaModelCatalogRepository::new(database.writer.clone());
        let instance_id = crate::kernel::schema::instance_id(database.writer.as_ref())
            .await
            .unwrap();
        let snapshot = |time, document| ModelCatalogCacheRecord {
            fetched_at_unix_ms: time,
            source: "generated".into(),
            document,
        };
        let first = snapshot(
            10,
            json!({"models":{
                "example/unchanged":{"display_name":"Stable"},
                "example/changed":{"display_name":"Old"},
                "example/removed":{"display_name":"Removed"}
            }}),
        );
        let second = snapshot(
            20,
            json!({"models":{
                "example/unchanged":{"display_name":"Stable"},
                "example/changed":{"display_name":"New"},
                "example/added":{"display_name":"Added"}
            }}),
        );
        let changes = || async {
            sea_orm::sqlx::query_scalar::<_, i64>("SELECT total_changes()")
                .fetch_one(database.writer.get_sqlite_connection_pool())
                .await
                .unwrap()
        };
        assert!(repository.read_cache().await.unwrap().is_none());
        repository.write_cache(&first).await.unwrap();
        let before = changes().await;
        repository.write_cache(&second).await.unwrap();
        // Three catalog rows change, plus the writer lock and catalog state.
        assert_eq!(changes().await - before, 5);
        repository.write_cache(&first).await.unwrap();
        let stored = repository.read_cache().await.unwrap().unwrap();
        assert_eq!(stored.fetched_at_unix_ms, second.fetched_at_unix_ms);
        assert_eq!(stored.source, second.source);
        assert_eq!(stored.document, second.document);
        let before = changes().await;
        repository
            .write_cache(&snapshot(30, second.document))
            .await
            .unwrap();
        // The writer lock touches the commit row, then only catalog state updates.
        assert_eq!(changes().await - before, 2);
        assert_eq!(
            repository
                .read_cache()
                .await
                .unwrap()
                .unwrap()
                .fetched_at_unix_ms,
            30
        );
        repository.clear_cached_official_from_db(20).await.unwrap();
        assert_eq!(
            repository
                .read_cache()
                .await
                .unwrap()
                .unwrap()
                .fetched_at_unix_ms,
            30
        );

        let pool = database.writer.get_sqlite_connection_pool();
        assert!(
            sea_orm::sqlx::query("UPDATE agena_instance SET catalog_digest=NULL WHERE id=1")
                .execute(pool)
                .await
                .is_err()
        );
        sea_orm::sqlx::query("UPDATE agena_instance SET catalog_source='invalid' WHERE id=1")
            .execute(pool)
            .await
            .unwrap();
        assert!(repository.read_cache().await.unwrap().is_none());
        assert_eq!(
            crate::kernel::schema::instance_id(database.writer.as_ref())
                .await
                .unwrap(),
            instance_id
        );
        assert_eq!(
            sea_orm::sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM agena_model_catalog_entries"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        repository.write_cache(&first).await.unwrap();
        assert_eq!(
            repository
                .read_cache()
                .await
                .unwrap()
                .unwrap()
                .fetched_at_unix_ms,
            first.fetched_at_unix_ms
        );
        database.close().await.unwrap();
    }
}
