use std::sync::Arc;

use agena_storage::{UsageRecord, UsageRepository, UsageRepositoryError, UsageSample};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use serde::Deserialize;

const SESSION_TABLE: &str = "agena_sessions";
const LINEAGE_TABLE: &str = "agena_session_lineage";
const MESSAGE_TABLE: &str = "agena_activity_messages";

/// SQLite implementation of usage aggregation input reads.
///
/// The application-facing contract deliberately exposes only provider/model
/// identifiers and token/cost values. This adapter reads those stable JSON
/// fields directly instead of importing core transcript entity types.
pub struct SeaUsageRepository {
    db: Arc<DatabaseConnection>,
}

impl SeaUsageRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl UsageRepository for SeaUsageRepository {
    async fn list(
        &self,
        workspace_id: i64,
        session_ids: &[i64],
        include_subagents: bool,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
    ) -> Result<Vec<UsageRecord>, UsageRepositoryError> {
        let mut session_clauses = vec![
            "s.workspace_id = ?".to_owned(),
            "s.lifecycle_state = 'ready'".to_owned(),
        ];
        let mut session_values = vec![workspace_id.into()];
        if !session_ids.is_empty() {
            session_clauses.push("s.id IN (".to_owned() + &placeholders(session_ids.len()) + ")");
            session_values.extend(session_ids.iter().copied().map(Into::into));
        }
        if !include_subagents {
            session_clauses.push("COALESCE(l.relation_kind, 'root') != 'subagent'".to_owned());
        }
        let sessions = self.db.query_all(statement(
            format!("SELECT s.id, s.title, COALESCE(l.relation_kind, 'root') AS relation_kind FROM {SESSION_TABLE} s LEFT JOIN {LINEAGE_TABLE} l ON l.session_id = s.id WHERE {}", session_clauses.join(" AND ")),
            session_values,
        )).await.map_err(map_error)?;
        if sessions.is_empty() {
            return Ok(Vec::new());
        }

        let mut metadata = std::collections::HashMap::with_capacity(sessions.len());
        for row in sessions {
            let id: i64 = row.try_get("", "id").map_err(map_error)?;
            metadata.insert(
                id,
                (
                    row.try_get("", "title").map_err(map_error)?,
                    row.try_get::<String>("", "relation_kind")
                        .map_err(map_error)?
                        == "subagent",
                ),
            );
        }
        let ids = metadata.keys().copied().collect::<Vec<_>>();
        let mut clauses = vec![
            format!("session_id IN ({})", placeholders(ids.len())),
            // `StoredRole::Assistant` is the stable SQLite integer value.
            "role = 2".to_owned(),
            "usage IS NOT NULL".to_owned(),
        ];
        let mut values = ids.into_iter().map(Into::into).collect::<Vec<Value>>();
        if let Some(from_ms) = from_ms {
            clauses.push("created_at_ms >= ?".to_owned());
            values.push(from_ms.into());
        }
        if let Some(to_ms) = to_ms {
            clauses.push("created_at_ms <= ?".to_owned());
            values.push(to_ms.into());
        }
        let rows = self.db.query_all(statement(
            format!("SELECT session_id, created_at_ms, metadata, usage FROM {MESSAGE_TABLE} WHERE {} ORDER BY created_at_ms ASC, message_id ASC", clauses.join(" AND ")),
            values,
        )).await.map_err(map_error)?;
        rows.into_iter()
            .map(|row| {
                let session_id: i64 = row.try_get("", "session_id").map_err(map_error)?;
                let (session_title, is_subagent) =
                    metadata.get(&session_id).cloned().ok_or_else(|| {
                        UsageRepositoryError::Backend(format!(
                            "usage message references unselected session {session_id}"
                        ))
                    })?;
                let message: PersistedMessageMetadata =
                    serde_json::from_value(row.try_get("", "metadata").map_err(map_error)?)
                        .map_err(map_error)?;
                let usage: serde_json::Value = row.try_get("", "usage").map_err(map_error)?;
                Ok(UsageRecord {
                    session_id,
                    session_title,
                    is_subagent,
                    created_at_ms: row.try_get("", "created_at_ms").map_err(map_error)?,
                    provider_id: nonempty_or_unknown(&message.model_provider_id),
                    model_id: nonempty_or_unknown(&message.model_id),
                    usage: UsageSample { value: usage },
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct PersistedMessageMetadata {
    #[serde(default)]
    model_provider_id: String,
    #[serde(default)]
    model_id: String,
}
fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}
fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn nonempty_or_unknown(value: &str) -> String {
    if value.trim().is_empty() {
        "unknown".to_owned()
    } else {
        value.to_owned()
    }
}
fn map_error(error: impl std::fmt::Display) -> UsageRepositoryError {
    UsageRepositoryError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    async fn repository() -> SeaUsageRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        for sql in [
            format!(
                "CREATE TABLE {SESSION_TABLE} (id INTEGER PRIMARY KEY, workspace_id INTEGER NOT NULL, title TEXT NOT NULL, lifecycle_state TEXT NOT NULL)"
            ),
            format!(
                "CREATE TABLE {LINEAGE_TABLE} (session_id INTEGER PRIMARY KEY, relation_kind TEXT NOT NULL)"
            ),
            format!(
                "CREATE TABLE {MESSAGE_TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL, role INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, metadata JSON NOT NULL, usage JSON NULL)"
            ),
            format!(
                "INSERT INTO {SESSION_TABLE} VALUES (1, 9, 'root', 'ready'), (2, 9, 'child', 'ready'), (3, 8, 'other', 'ready')"
            ),
            format!("INSERT INTO {LINEAGE_TABLE} VALUES (2, 'subagent')"),
            format!(
                "INSERT INTO {MESSAGE_TABLE} VALUES (10, 1, 2, 100, '{{\"model_provider_id\":\"openai\",\"model_id\":\"gpt\"}}', '{{\"input_tokens\":1,\"output_tokens\":2,\"reasoning_tokens\":3,\"cache_write_tokens\":4,\"cache_read_tokens\":5,\"total_cost\":0.25}}'), (11, 2, 2, 200, '{{\"model_provider_id\":\"\",\"model_id\":\"\"}}', '{{\"input_tokens\":6,\"output_tokens\":7,\"reasoning_tokens\":8,\"cache_write_tokens\":9,\"cache_read_tokens\":10,\"total_cost\":0.5}}'), (12, 1, 1, 300, '{{}}', NULL)"
            ),
        ] {
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .expect("fixture");
        }
        SeaUsageRepository::new(Arc::new(db))
    }
    #[tokio::test]
    async fn filters_workspace_subagents_and_time_range_without_core_models() {
        let repository = repository().await;
        let root = repository
            .list(9, &[], false, None, None)
            .await
            .expect("root usage");
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].provider_id, "openai");
        let all = repository
            .list(9, &[1, 2], true, Some(150), None)
            .await
            .expect("all usage");
        assert_eq!(all.len(), 1);
        assert!(all[0].is_subagent);
        assert_eq!(all[0].model_id, "unknown");
    }
}
