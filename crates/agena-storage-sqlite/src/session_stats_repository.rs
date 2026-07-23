use std::{collections::HashMap, sync::Arc};

use agena_domain::MESSAGE_CREATED_EVENT_KIND_TAGS;
use agena_storage::{SessionEventStats, SessionStatsRepository, SessionStatsRepositoryError};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};

const SESSION_TABLE: &str = "agena_sessions";
const EVENT_TABLE: &str = "agena_events";

pub struct SeaSessionStatsRepository {
    db: Arc<DatabaseConnection>,
}

impl SeaSessionStatsRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    async fn grouped_counts(
        &self,
        group_column: &str,
        id_column: &str,
        ids: &[i64],
    ) -> Result<HashMap<i64, i64>, SessionStatsRepositoryError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT {group_column} AS id, COUNT(*) AS count FROM {SESSION_TABLE} WHERE {id_column} IN ({placeholders}) AND lifecycle_state = 'ready' GROUP BY {group_column}"),
                ids.iter().copied().map(Into::into).collect::<Vec<Value>>(),
            ))
            .await
            .map_err(map_error)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("", "id").map_err(map_error)?,
                    row.try_get("", "count").map_err(map_error)?,
                ))
            })
            .collect()
    }
}

#[async_trait]
impl SessionStatsRepository for SeaSessionStatsRepository {
    async fn workspace_counts(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, SessionStatsRepositoryError> {
        self.grouped_counts("workspace_id", "workspace_id", workspace_ids)
            .await
    }

    async fn event_stats(
        &self,
        session_ids: &[i64],
    ) -> Result<HashMap<i64, SessionEventStats>, SessionStatsRepositoryError> {
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let session_placeholders = std::iter::repeat_n("?", session_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let kind_placeholders = std::iter::repeat_n("?", MESSAGE_CREATED_EVENT_KIND_TAGS.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut values = session_ids
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<Value>>();
        values.extend(
            MESSAGE_CREATED_EVENT_KIND_TAGS
                .iter()
                .map(|tag| (*tag).into()),
        );
        let rows = self.db.query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("SELECT session_id, COUNT(*) AS message_count, MAX(created_at_ms) AS last_message_at_ms FROM {EVENT_TABLE} WHERE session_id IN ({session_placeholders}) AND kind_tag IN ({kind_placeholders}) GROUP BY session_id"),
            values,
        )).await.map_err(map_error)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("", "session_id").map_err(map_error)?,
                    SessionEventStats {
                        message_count: row.try_get("", "message_count").map_err(map_error)?,
                        last_message_at_ms: row
                            .try_get("", "last_message_at_ms")
                            .map_err(map_error)?,
                    },
                ))
            })
            .collect()
    }

    async fn child_counts(
        &self,
        parent_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, SessionStatsRepositoryError> {
        self.grouped_counts("parent_id", "parent_id", parent_ids)
            .await
    }
}

fn map_error(error: impl std::fmt::Display) -> SessionStatsRepositoryError {
    SessionStatsRepositoryError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    async fn repository() -> SeaSessionStatsRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        for sql in [
            format!(
                "CREATE TABLE {SESSION_TABLE} (id INTEGER PRIMARY KEY, workspace_id INTEGER NOT NULL, parent_id INTEGER NULL, lifecycle_state TEXT NOT NULL)"
            ),
            format!(
                "CREATE TABLE {EVENT_TABLE} (id INTEGER PRIMARY KEY, session_id INTEGER NULL, kind_tag TEXT NOT NULL, created_at_ms INTEGER NOT NULL)"
            ),
            format!(
                "INSERT INTO {SESSION_TABLE} VALUES (1, 10, NULL, 'ready'), (2, 10, 1, 'ready'), (3, 10, 1, 'creating'), (4, 11, NULL, 'ready')"
            ),
            format!(
                "INSERT INTO {EVENT_TABLE} VALUES (1, 1, 'user_message_appended', 100), (2, 1, 'tool_call_completed', 200), (3, 1, 'assistant_message_finished', 300)"
            ),
        ] {
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .expect("create stats fixture");
        }
        SeaSessionStatsRepository::new(Arc::new(db))
    }

    #[tokio::test]
    async fn aggregates_only_ready_sessions_and_message_creation_events() {
        let repository = repository().await;
        assert_eq!(
            repository
                .workspace_counts(&[10, 11])
                .await
                .expect("workspace stats"),
            HashMap::from([(10, 2), (11, 1)])
        );
        assert_eq!(
            repository.child_counts(&[1]).await.expect("child stats"),
            HashMap::from([(1, 1)])
        );
        assert_eq!(
            repository.event_stats(&[1]).await.expect("event stats"),
            HashMap::from([(
                1,
                SessionEventStats {
                    message_count: 2,
                    last_message_at_ms: Some(300)
                },
            )])
        );
    }
}
