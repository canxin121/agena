use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_storage::instance_state::{InstanceStateError, InstanceStateRepository};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement};

/// Small application documents in the instance database.
/// This adapter has no independent schema initialization or close operation.
pub struct ServerStateRepository {
    database: Arc<DatabaseConnection>,
    reader: Arc<DatabaseConnection>,
    path: PathBuf,
}

impl ServerStateRepository {
    pub async fn new(database: Arc<DatabaseConnection>) -> Result<Self, DbErr> {
        Self::with_reader(database.clone(), database).await
    }

    pub async fn with_reader(
        database: Arc<DatabaseConnection>,
        reader: Arc<DatabaseConnection>,
    ) -> Result<Self, DbErr> {
        let row = reader
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA database_list",
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("SQLite main database is missing".into()))?;
        let file: String = row.try_get("", "file")?;
        let path = if file.is_empty() {
            PathBuf::from(format!(
                ":memory:{}",
                crate::kernel::schema::instance_id(&reader).await?
            ))
        } else {
            PathBuf::from(file)
        };
        Ok(Self {
            database,
            reader,
            path,
        })
    }
}

#[async_trait]
impl InstanceStateRepository for ServerStateRepository {
    fn path(&self) -> &Path {
        &self.path
    }

    async fn get_payload(&self, key: &str) -> Result<Option<String>, InstanceStateError> {
        let key = normalize_key(key)?;
        self.reader
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT value_json FROM agena_server_state WHERE key=?",
                [key.into()],
            ))
            .await
            .map_err(backend_error)?
            .map(|row| row.try_get("", "value_json").map_err(backend_error))
            .transpose()
    }

    async fn set_payload(&self, key: &str, payload: String) -> Result<(), InstanceStateError> {
        let key = normalize_key(key)?;
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(backend_error)?;
        self.database
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO agena_server_state(key,value_json) VALUES (?,?)
                 ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json
                 WHERE agena_server_state.value_json!=excluded.value_json",
                [key.into(), payload.into()],
            ))
            .await
            .map_err(backend_error)?;
        Ok(())
    }
}

fn normalize_key(key: &str) -> Result<&str, InstanceStateError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 200 {
        return Err(InstanceStateError::InvalidKey(
            "expected 1–200 bytes".into(),
        ));
    }
    Ok(key)
}

fn backend_error(error: DbErr) -> InstanceStateError {
    InstanceStateError::Backend(agena_failure::diagnostic::format_error_chain(&error))
}
