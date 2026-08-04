use std::sync::Arc;

use agena_domain::{
    MESSAGE_CREATED_EVENT_KIND_TAGS, SessionLifecycleState, SessionRelationKind, SubtaskStatus,
};
use agena_storage::{
    SessionMutationRepository, SessionMutationRepositoryError, SessionSummaryListQuery,
    SessionSummaryRecord, SessionSummaryRepository, SessionSummaryRepositoryError,
    SessionTreeRecord,
};
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};

const SESSIONS: &str = "agena_sessions";
const LINEAGE: &str = "agena_session_lineage";
const EVENTS: &str = "agena_events";
const SESSION_COLUMNS: &str = "s.id, s.parent_id, s.depth, s.root_id, s.workspace_id, s.title, s.version, s.lifecycle_state, s.created_at_ms, s.updated_at_ms, s.runtime_state_json, l.relation_kind, l.source_cutoff_seq_global, l.source_message_id, l.task_id, l.subtask_status";

/// SQLite implementation of ordinary session summary reads and mutations.
/// Branch/history construction remains core-owned because it shares a larger
/// persistence transaction; this adapter owns only the stable summary shape.
pub struct SeaSessionSummaryRepository {
    db: Arc<DatabaseConnection>,
}
impl SeaSessionSummaryRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
    async fn get_record<C: ConnectionTrait>(
        &self,
        db: &C,
        id: i64,
    ) -> Result<Option<SessionSummaryRecord>, SessionSummaryRepositoryError> {
        db.query_one(statement(format!("SELECT {SESSION_COLUMNS} FROM {SESSIONS} s LEFT JOIN {LINEAGE} l ON l.session_id = s.id WHERE s.id = ?"), [id.into()])).await.map_err(map_summary_error)?.map(|row| record_from_row(&row)).transpose()
    }
}

#[async_trait]
impl SessionSummaryRepository for SeaSessionSummaryRepository {
    async fn get(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummaryRecord>, SessionSummaryRepositoryError> {
        self.get_record(self.db.as_ref(), session_id).await
    }
    async fn list(
        &self,
        query: SessionSummaryListQuery,
    ) -> Result<Vec<SessionSummaryRecord>, SessionSummaryRepositoryError> {
        let mut clauses = vec!["s.lifecycle_state = 'ready'".to_owned()];
        let mut values = Vec::<Value>::new();
        if !query.include_subagents {
            clauses.push("COALESCE(l.relation_kind, 'root') != 'subagent'".to_owned());
        }
        if let Some(id) = query.workspace_id {
            clauses.push("s.workspace_id = ?".to_owned());
            values.push(id.into());
        }
        if query.roots_only {
            clauses.push("s.parent_id IS NULL".to_owned());
        }
        if let Some(id) = query.parent_id {
            clauses.push("s.parent_id = ?".to_owned());
            values.push(id.into());
        }
        if let Some(search) = query.search.filter(|value| !value.is_empty()) {
            clauses.push("s.title LIKE ?".to_owned());
            values.push(format!("%{search}%").into());
        }
        if let (Some(updated), Some(id)) = (query.before_updated_at_ms, query.before_id) {
            clauses.push("(s.updated_at_ms < ? OR (s.updated_at_ms = ? AND s.id < ?))".to_owned());
            values.extend([updated.into(), updated.into(), id.into()]);
        }
        let mut limit = String::new();
        if query.limit != u64::MAX {
            limit = " LIMIT ?".to_owned();
            values.push(query.limit.into());
        }
        if query.offset > 0 {
            limit.push_str(if query.limit == u64::MAX {
                " LIMIT -1 OFFSET ?"
            } else {
                " OFFSET ?"
            });
            values.push(query.offset.into());
        }
        self.db.query_all(statement(format!("SELECT {SESSION_COLUMNS} FROM {SESSIONS} s LEFT JOIN {LINEAGE} l ON l.session_id = s.id WHERE {} ORDER BY s.updated_at_ms DESC, s.id DESC{limit}", clauses.join(" AND ")), values)).await.map_err(map_summary_error)?.into_iter().map(|row| record_from_row(&row)).collect()
    }
    async fn get_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, SessionSummaryRepositoryError> {
        self.db.query_one(statement(format!("SELECT {SESSION_COLUMNS} FROM {SESSIONS} s JOIN {LINEAGE} l ON l.session_id = s.id WHERE s.parent_id = ? AND l.relation_kind = 'subagent' AND l.task_id = ?"), [parent_session_id.into(), task_id.to_owned().into()])).await.map_err(map_summary_error)?.map(|row| record_from_row(&row)).transpose()
    }
    async fn list_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionTreeRecord>, SessionSummaryRepositoryError> {
        let rows = self.db.query_all(statement(format!("SELECT {SESSION_COLUMNS}, COUNT(DISTINCT e.id) AS message_count, MAX(e.created_at_ms) AS last_message_at_ms, COUNT(DISTINCT child.id) AS child_session_count FROM {SESSIONS} s LEFT JOIN {LINEAGE} l ON l.session_id = s.id LEFT JOIN {EVENTS} e ON e.session_id = s.id AND e.kind_tag IN ({}) LEFT JOIN {SESSIONS} child ON child.parent_id = s.id WHERE s.root_id = ? GROUP BY s.id ORDER BY s.depth ASC, s.id ASC", placeholders(MESSAGE_CREATED_EVENT_KIND_TAGS.len())), message_kind_values(root_id))).await.map_err(map_summary_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(SessionTreeRecord {
                    summary: record_from_row(&row)?,
                    message_count: row
                        .try_get("", "message_count")
                        .map_err(map_summary_error)?,
                    child_session_count: row
                        .try_get("", "child_session_count")
                        .map_err(map_summary_error)?,
                    last_message_at_ms: row
                        .try_get("", "last_message_at_ms")
                        .map_err(map_summary_error)?,
                })
            })
            .collect()
    }
}

#[async_trait]
impl SessionMutationRepository for SeaSessionSummaryRepository {
    async fn create(
        &self,
        workspace_id: i64,
        parent_id: Option<i64>,
        title: String,
    ) -> Result<SessionSummaryRecord, SessionMutationRepositoryError> {
        let txn = self.db.begin().await.map_err(map_mutation_error)?;
        // Acquire the write lock before the parent SELECT so a concurrent
        // writer in another process cannot make the read→write upgrade fail
        // with SQLITE_BUSY (the busy timeout only applies at transaction start).
        crate::acquire_write_lock(&txn)
            .await
            .map_err(map_mutation_error)?;
        let now = Utc::now().timestamp_millis();
        let (depth, root_id) = if let Some(parent_id) = parent_id {
            let parent = txn.query_one(statement(format!("SELECT depth, root_id, workspace_id, lifecycle_state FROM {SESSIONS} WHERE id = ?"), [parent_id.into()])).await.map_err(map_mutation_error)?.ok_or_else(|| SessionMutationRepositoryError::Backend(format!("parent session not found: {parent_id}")))?;
            let parent_workspace: i64 = parent
                .try_get("", "workspace_id")
                .map_err(map_mutation_error)?;
            let lifecycle: String = parent
                .try_get("", "lifecycle_state")
                .map_err(map_mutation_error)?;
            if parent_workspace != workspace_id || lifecycle != "ready" {
                return Err(SessionMutationRepositoryError::Backend(
                    "parent session is not a ready session in the requested workspace".to_owned(),
                ));
            }
            (
                parent
                    .try_get::<i64>("", "depth")
                    .map_err(map_mutation_error)?
                    + 1,
                parent.try_get("", "root_id").map_err(map_mutation_error)?,
            )
        } else {
            (0, 0)
        };
        let result = txn.execute(statement(format!("INSERT INTO {SESSIONS} (parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, creation_failure_json, runtime_state_json, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, 1, 'ready', NULL, '{{}}', ?, ?)"), [parent_id.into(), depth.into(), root_id.into(), workspace_id.into(), title.into(), now.into(), now.into()])).await.map_err(map_mutation_error)?;
        let id = i64::try_from(result.last_insert_id()).map_err(|_| {
            SessionMutationRepositoryError::Backend(
                "session identifier exceeds i64 range".to_owned(),
            )
        })?;
        if parent_id.is_none() {
            txn.execute(statement(
                format!("UPDATE {SESSIONS} SET root_id = id WHERE id = ?"),
                [id.into()],
            ))
            .await
            .map_err(map_mutation_error)?;
        } else {
            txn.execute(statement(format!("INSERT INTO {LINEAGE} (session_id, relation_kind, source_cutoff_seq_global, source_message_id, task_id, subtask_status, subtask_started_at_ms, subtask_finished_at_ms, subtask_failure_json, created_at_ms) VALUES (?, 'child', NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?)"), [id.into(), now.into()])).await.map_err(map_mutation_error)?;
        }
        let record = self
            .get_record(&txn, id)
            .await
            .map_err(|error| SessionMutationRepositoryError::Backend(error.to_string()))?
            .ok_or_else(|| {
                SessionMutationRepositoryError::Backend("created session row is missing".to_owned())
            })?;
        txn.commit().await.map_err(map_mutation_error)?;
        Ok(record)
    }
    async fn rename(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<Option<SessionSummaryRecord>, SessionMutationRepositoryError> {
        let changed = self.db.execute(statement(format!("UPDATE {SESSIONS} SET title = ?, version = version + 1, updated_at_ms = ? WHERE id = ?"), [title.into(), Utc::now().timestamp_millis().into(), session_id.into()])).await.map_err(map_mutation_error)?;
        if changed.rows_affected() == 0 {
            Ok(None)
        } else {
            self.get_record(self.db.as_ref(), session_id)
                .await
                .map_err(|e| SessionMutationRepositoryError::Backend(e.to_string()))
        }
    }
    async fn delete(&self, session_id: i64) -> Result<u64, SessionMutationRepositoryError> {
        self.db
            .execute(statement(
                format!("DELETE FROM {SESSIONS} WHERE id = ?"),
                [session_id.into()],
            ))
            .await
            .map(|result| result.rows_affected())
            .map_err(map_mutation_error)
    }
}

fn record_from_row(
    row: &sea_orm::QueryResult,
) -> Result<SessionSummaryRecord, SessionSummaryRepositoryError> {
    let id: i64 = row.try_get("", "id").map_err(map_summary_error)?;
    let parent_id: Option<i64> = row.try_get("", "parent_id").map_err(map_summary_error)?;
    let relation_text: Option<String> = row
        .try_get("", "relation_kind")
        .map_err(map_summary_error)?;
    let relation_kind = match (parent_id, relation_text.as_deref()) {
        (None, None) => SessionRelationKind::Root,
        (Some(_), Some(value)) => {
            SessionRelationKind::parse(value).ok_or_else(|| invalid_summary(id, "relation kind"))?
        }
        _ => return Err(invalid_summary(id, "lineage shape")),
    };
    let lifecycle_text: String = row
        .try_get("", "lifecycle_state")
        .map_err(map_summary_error)?;
    let lifecycle_state = SessionLifecycleState::parse(&lifecycle_text)
        .ok_or_else(|| invalid_summary(id, "lifecycle state"))?;
    let status: Option<String> = row
        .try_get("", "subtask_status")
        .map_err(map_summary_error)?;
    let runtime: Option<serde_json::Value> = row
        .try_get("", "runtime_state_json")
        .map_err(map_summary_error)?;
    let subtask_access = if relation_kind.is_subagent() {
        let value = runtime
            .as_ref()
            .and_then(|value| value.pointer("/execution/access"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::String("inherit".to_owned()));
        Some(
            serde_json::from_value(value)
                .map_err(|_| invalid_summary(id, "subtask execution access"))?,
        )
    } else {
        None
    };
    Ok(SessionSummaryRecord {
        id,
        parent_id,
        depth: row.try_get("", "depth").map_err(map_summary_error)?,
        root_id: row.try_get("", "root_id").map_err(map_summary_error)?,
        workspace_id: row.try_get("", "workspace_id").map_err(map_summary_error)?,
        title: row.try_get("", "title").map_err(map_summary_error)?,
        version: row.try_get("", "version").map_err(map_summary_error)?,
        relation_kind,
        lifecycle_state,
        source_cutoff_seq_global: row
            .try_get("", "source_cutoff_seq_global")
            .map_err(map_summary_error)?,
        source_message_id: row
            .try_get("", "source_message_id")
            .map_err(map_summary_error)?,
        task_id: row.try_get("", "task_id").map_err(map_summary_error)?,
        subtask_access,
        subtask_status: match status.as_deref() {
            Some(value) => Some(
                SubtaskStatus::parse(value).ok_or_else(|| invalid_summary(id, "subtask status"))?,
            ),
            None => None,
        },
        created_at_ms: row
            .try_get("", "created_at_ms")
            .map_err(map_summary_error)?,
        updated_at_ms: row
            .try_get("", "updated_at_ms")
            .map_err(map_summary_error)?,
    })
}
fn invalid_summary(id: i64, field: &str) -> SessionSummaryRepositoryError {
    SessionSummaryRepositoryError::Backend(format!("session {id} has invalid {field}"))
}
fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}
fn message_kind_values(root_id: i64) -> Vec<Value> {
    MESSAGE_CREATED_EVENT_KIND_TAGS
        .iter()
        .map(|tag| (*tag).to_owned().into())
        .chain(std::iter::once(root_id.into()))
        .collect()
}
fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn map_summary_error(error: impl std::fmt::Display) -> SessionSummaryRepositoryError {
    SessionSummaryRepositoryError::Backend(error.to_string())
}
fn map_mutation_error(error: impl std::fmt::Display) -> SessionMutationRepositoryError {
    SessionMutationRepositoryError::Backend(error.to_string())
}
