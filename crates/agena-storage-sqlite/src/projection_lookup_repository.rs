use std::sync::Arc;

use agena_storage::{ProjectionLookupRepository, ProjectionLookupRepositoryError};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

const MESSAGE_TABLE: &str = "agena_activity_messages";
const PART_TABLE: &str = "agena_activity_parts";

/// SQLite adapter for the stable projected-message/part ownership lookup.
/// It deliberately uses only table-level SQL so it does not depend on core
/// transcript entities or message payload types.
pub struct SeaProjectionLookupRepository {
    db: Arc<DatabaseConnection>,
}

impl SeaProjectionLookupRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    async fn session_id_for_message_id(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, ProjectionLookupRepositoryError> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT session_id FROM {MESSAGE_TABLE} WHERE message_id = ? LIMIT 1"),
                [message_id.into()],
            ))
            .await
            .map_err(map_error)?;
        row.map(|row| row.try_get("", "session_id").map_err(map_error))
            .transpose()
    }
}

#[async_trait]
impl ProjectionLookupRepository for SeaProjectionLookupRepository {
    async fn session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, ProjectionLookupRepositoryError> {
        self.session_id_for_message_id(message_id).await
    }

    async fn session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, ProjectionLookupRepositoryError> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT message_id FROM {PART_TABLE} WHERE part_id = ? LIMIT 1"),
                [part_id.into()],
            ))
            .await
            .map_err(map_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let message_id: i64 = row.try_get("", "message_id").map_err(map_error)?;
        self.session_id_for_message_id(message_id).await
    }
}

fn map_error(error: impl std::fmt::Display) -> ProjectionLookupRepositoryError {
    ProjectionLookupRepositoryError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    async fn repository() -> SeaProjectionLookupRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        for sql in [
            format!(
                "CREATE TABLE {MESSAGE_TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL)"
            ),
            format!(
                "CREATE TABLE {PART_TABLE} (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL)"
            ),
            format!("INSERT INTO {MESSAGE_TABLE} (message_id, session_id) VALUES (7, 41)"),
            format!("INSERT INTO {PART_TABLE} (part_id, message_id) VALUES (9, 7)"),
        ] {
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .expect("create projection fixture");
        }
        SeaProjectionLookupRepository::new(Arc::new(db))
    }

    #[tokio::test]
    async fn resolves_session_ownership_for_messages_and_parts() {
        let repository = repository().await;
        assert_eq!(
            repository.session_id_for_message(7).await.expect("message"),
            Some(41)
        );
        assert_eq!(
            repository.session_id_for_part(9).await.expect("part"),
            Some(41)
        );
        assert_eq!(
            repository.session_id_for_part(99).await.expect("missing"),
            None
        );
    }
}
