use std::{path::Path, sync::Arc};

use agena_storage::{
    WorkspaceListQuery, WorkspaceRecord, WorkspaceRepository, WorkspaceRepositoryError,
};
use async_trait::async_trait;
use chrono::Utc;
use path_clean::PathClean;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};

const TABLE: &str = "agena_workspaces";

/// SQLite-backed workspace repository.
pub struct SeaWorkspaceRepository {
    db: Arc<DatabaseConnection>,
}

impl SeaWorkspaceRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    async fn record(&self, id: i64) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError> {
        self.db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT id, path, created_at_ms, updated_at_ms FROM {TABLE} WHERE id = ?"),
                [id.into()],
            ))
            .await
            .map_err(map_error)?
            .map(record_from_row)
            .transpose()
    }
}

#[async_trait]
impl WorkspaceRepository for SeaWorkspaceRepository {
    async fn create(&self, path: String) -> Result<WorkspaceRecord, WorkspaceRepositoryError> {
        let path = normalized_workspace_path(path.as_str())?;
        let now = Utc::now().timestamp_millis();
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "INSERT INTO {TABLE} (path, created_at_ms, updated_at_ms) VALUES (?, ?, ?)"
                ),
                [path.into(), now.into(), now.into()],
            ))
            .await
            .map_err(map_error)?;
        let id = i64::try_from(result.last_insert_id()).map_err(|_| {
            WorkspaceRepositoryError::Backend("workspace identifier exceeds i64 range".to_owned())
        })?;
        self.record(id).await?.ok_or_else(|| {
            WorkspaceRepositoryError::Backend("created workspace row is missing".to_owned())
        })
    }
    async fn update_path(
        &self,
        id: i64,
        path: String,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError> {
        if self.record(id).await?.is_none() {
            return Ok(None);
        }
        let path = normalized_workspace_path(path.as_str())?;
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("UPDATE {TABLE} SET path = ?, updated_at_ms = ? WHERE id = ?"),
                [path.into(), Utc::now().timestamp_millis().into(), id.into()],
            ))
            .await
            .map_err(map_error)?;
        self.record(id).await
    }
    async fn delete(&self, id: i64) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError> {
        let existing = self.record(id).await?;
        if existing.is_some() {
            self.db
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    format!("DELETE FROM {TABLE} WHERE id = ?"),
                    [id.into()],
                ))
                .await
                .map_err(map_error)?;
        }
        Ok(existing)
    }
    async fn get(&self, id: i64) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError> {
        self.record(id).await
    }
    async fn list(
        &self,
        query: WorkspaceListQuery,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceRepositoryError> {
        let mut clauses = Vec::new();
        let mut values = Vec::<Value>::new();
        if let Some(search) = query.search.filter(|value| !value.is_empty()) {
            clauses.push("path LIKE ?");
            values.push(format!("%{search}%").into());
        }
        if let (Some(updated), Some(id)) = (query.before_updated_at_ms, query.before_id) {
            clauses.push("(updated_at_ms < ? OR (updated_at_ms = ? AND id < ?))");
            values.extend([updated.into(), updated.into(), id.into()]);
        }
        values.push(query.limit.into());
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let rows = self.db.query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            format!("SELECT id, path, created_at_ms, updated_at_ms FROM {TABLE}{where_clause} ORDER BY updated_at_ms DESC, id DESC LIMIT ?"), values)).await.map_err(map_error)?;
        rows.into_iter().map(record_from_row).collect()
    }
    async fn path_by_id(&self, id: i64) -> Result<Option<String>, WorkspaceRepositoryError> {
        Ok(self.record(id).await?.map(|row| row.path))
    }
    async fn lookup_id(&self, path: &str) -> Result<Option<i64>, WorkspaceRepositoryError> {
        let path = normalized_workspace_path(path)?;
        self.db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT id FROM {TABLE} WHERE path = ?"),
                [path.into()],
            ))
            .await
            .map_err(map_error)?
            .map(|row| row.try_get("", "id").map_err(map_error))
            .transpose()
    }
    async fn ensure_id(&self, path: &str) -> Result<i64, WorkspaceRepositoryError> {
        // Atomic insert-or-nothing: concurrent processes racing on the same
        // path can only win one insert, and every loser falls through to the
        // read-back below instead of surfacing a unique-constraint error.
        let path = normalized_workspace_path(path)?;
        let now = Utc::now().timestamp_millis();
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "INSERT INTO {TABLE} (path, created_at_ms, updated_at_ms) VALUES (?, ?, ?) \
                     ON CONFLICT(path) DO NOTHING"
                ),
                [path.clone().into(), now.into(), now.into()],
            ))
            .await
            .map_err(map_error)?;
        self.lookup_id(&path).await?.ok_or_else(|| {
            WorkspaceRepositoryError::Backend(format!(
                "workspace row is missing after ensure_id for {path}"
            ))
        })
    }
}

fn record_from_row(row: sea_orm::QueryResult) -> Result<WorkspaceRecord, WorkspaceRepositoryError> {
    Ok(WorkspaceRecord {
        id: row.try_get("", "id").map_err(map_error)?,
        path: row.try_get("", "path").map_err(map_error)?,
        created_at_ms: row.try_get("", "created_at_ms").map_err(map_error)?,
        updated_at_ms: row.try_get("", "updated_at_ms").map_err(map_error)?,
    })
}
fn normalized_workspace_path(path: &str) -> Result<String, WorkspaceRepositoryError> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err(WorkspaceRepositoryError::InvalidPath(
            "workspace path cannot be empty".to_owned(),
        ));
    }
    let cleaned = Path::new(raw).clean();
    let mut value = cleaned.to_string_lossy().replace('\\', "/");
    while value.ends_with('/') && value.len() > 1 && !is_windows_drive_root(value.as_str()) {
        value.pop();
    }
    if cfg!(windows) {
        value.make_ascii_lowercase();
    }
    Ok(value)
}
fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}
fn map_error(error: impl std::error::Error + 'static) -> WorkspaceRepositoryError {
    let message = agena_failure::diagnostic::format_error_chain(&error);
    if message.contains("workspace path cannot be empty") {
        WorkspaceRepositoryError::InvalidPath(message)
    } else {
        WorkspaceRepositoryError::Backend(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    async fn repository() -> SeaWorkspaceRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("CREATE TABLE {TABLE} (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)"),
        )).await.expect("create workspace fixture");
        SeaWorkspaceRepository::new(Arc::new(db))
    }

    #[tokio::test]
    async fn normalizes_identity_and_supports_crud() {
        let repository = repository().await;
        let created = repository
            .create(" /tmp/work/ ".to_owned())
            .await
            .expect("create");
        assert_eq!(created.path, "/tmp/work");
        assert_eq!(
            repository.lookup_id("/tmp/work//").await.expect("lookup"),
            Some(created.id)
        );
        let updated = repository
            .update_path(created.id, "/tmp/next/".to_owned())
            .await
            .expect("update")
            .expect("existing");
        assert_eq!(updated.path, "/tmp/next");
        assert_eq!(
            repository.path_by_id(created.id).await.expect("path"),
            Some("/tmp/next".to_owned())
        );
        assert_eq!(
            repository
                .delete(created.id)
                .await
                .expect("delete")
                .map(|row| row.id),
            Some(created.id)
        );
        assert!(repository.get(created.id).await.expect("get").is_none());
    }

    #[tokio::test]
    async fn rejects_empty_workspace_path() {
        let repository = repository().await;
        assert!(matches!(
            repository.create("  ".to_owned()).await,
            Err(WorkspaceRepositoryError::InvalidPath(_))
        ));
    }
}
