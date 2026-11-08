//! Durable business tasks on the existing instance database. Schema creation
//! belongs exclusively to the database owner, not this repository.
use std::sync::Arc;

use agena_superagent::{
    ChatMessage, ChatReply, Error, Result, Task, TaskQuery, TaskRepository, WorkNotice,
};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, QueryResult, Statement, TransactionTrait,
    Value,
};

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_superagent_chat_messages (session_id INTEGER NOT NULL CHECK(session_id>0), message_id TEXT NOT NULL, message_text TEXT NOT NULL CHECK(length(CAST(message_text AS BLOB)) BETWEEN 1 AND 32768), reply_json TEXT CHECK(reply_json IS NULL OR (json_valid(reply_json) AND length(CAST(reply_json AS BLOB))<=4194304)), PRIMARY KEY(session_id,message_id)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_superagent_tasks (id TEXT PRIMARY KEY CHECK(length(id)=36), source_session_id INTEGER NOT NULL CHECK(source_session_id>0), workspace_id INTEGER NOT NULL CHECK(workspace_id>0), request_key TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('queued','starting','running','waiting','review','completed','interrupted','failed','cancelling','cancelled')), revision INTEGER NOT NULL CHECK(revision>0), task_json TEXT NOT NULL CHECK(json_valid(task_json) AND length(CAST(task_json AS BLOB))<=131072), created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, UNIQUE(source_session_id,request_key))",
    "CREATE INDEX IF NOT EXISTS idx_agena_superagent_pending ON agena_superagent_tasks(state,created_at_ms,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_superagent_source ON agena_superagent_tasks(source_session_id,created_at_ms DESC,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_superagent_workspace ON agena_superagent_tasks(workspace_id,created_at_ms DESC,id)",
];

/// Keep this bundle separate so a complete previous Task baseline can upgrade
/// atomically without accepting arbitrary partially installed schemas.
pub(crate) const CONVERSATION_DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_superagent_outbox (notice_key TEXT PRIMARY KEY, task_json TEXT NOT NULL CHECK(json_valid(task_json)), prepared_text TEXT, delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0,1)), failures INTEGER NOT NULL DEFAULT 0, next_attempt_ms INTEGER NOT NULL DEFAULT 0, created_at_ms INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_agena_superagent_outbox_pending ON agena_superagent_outbox(delivered,next_attempt_ms,created_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_superagent_worker ON agena_superagent_tasks(json_extract(task_json,'$.worker_session_id'))",
];

pub struct SqliteTaskRepository {
    database: Arc<DatabaseConnection>,
}

impl SqliteTaskRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self { database }
    }

    async fn rows(&self, sql: &str, values: Vec<Value>) -> Result<Vec<Task>> {
        self.database
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await
            .map_err(storage)?
            .into_iter()
            .map(decode)
            .collect()
    }
}

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

fn decode(row: QueryResult) -> Result<Task> {
    let payload: String = row.try_get("", "task_json").map_err(storage)?;
    serde_json::from_str(&payload).map_err(storage)
}

fn encode(task: &Task) -> Result<String> {
    let payload = serde_json::to_string(task).map_err(storage)?;
    if payload.len() > 128 * 1024 {
        return Err(Error::Invalid("task record exceeds its byte budget"));
    }
    Ok(payload)
}

#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    async fn for_worker(&self, session_id: i64) -> Result<Option<Task>> {
        Ok(self.rows("SELECT task_json FROM agena_superagent_tasks WHERE json_extract(task_json,'$.worker_session_id')=? LIMIT 1", vec![session_id.into()]).await?.pop())
    }

    async fn active_workers(&self, source_session_id: i64) -> Result<Vec<i64>> {
        let tasks = self.rows("SELECT task_json FROM agena_superagent_tasks WHERE source_session_id=? AND state IN ('starting','running','waiting','cancelling') AND json_extract(task_json,'$.conversation') IS NOT NULL ORDER BY created_at_ms LIMIT 128", vec![source_session_id.into()]).await?;
        Ok(tasks
            .into_iter()
            .filter_map(|task| task.worker_session_id)
            .collect())
    }

    async fn pending_notices(&self, now_ms: i64, limit: usize) -> Result<Vec<WorkNotice>> {
        let rows = self.database.query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT notice_key,task_json,prepared_text,failures FROM agena_superagent_outbox WHERE delivered=0 AND next_attempt_ms<=? ORDER BY created_at_ms,notice_key LIMIT ?",
            vec![now_ms.into(), (limit.clamp(1,64) as i64).into()])).await.map_err(storage)?;
        rows.into_iter()
            .map(|row| {
                let task: String = row.try_get("", "task_json").map_err(storage)?;
                Ok(WorkNotice {
                    key: row.try_get("", "notice_key").map_err(storage)?,
                    task: serde_json::from_str(&task).map_err(storage)?,
                    prepared_text: row.try_get("", "prepared_text").map_err(storage)?,
                    failures: row
                        .try_get::<i64>("", "failures")
                        .map_err(storage)?
                        .clamp(0, 1000) as u32,
                })
            })
            .collect()
    }

    async fn prepare_notice(&self, key: &str, text: &str) -> Result<String> {
        if text.len() > 48 * 1024 {
            return Err(Error::Invalid("work notice exceeds its byte budget"));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_superagent_outbox SET prepared_text=? WHERE notice_key=? AND prepared_text IS NULL", vec![text.into(), key.into()])).await.map_err(storage)?;
        let row = self
            .database
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT prepared_text FROM agena_superagent_outbox WHERE notice_key=?",
                vec![key.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(Error::NotFound)?;
        row.try_get("", "prepared_text").map_err(storage)
    }

    async fn acknowledge_notice(&self, key: &str) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_superagent_outbox SET delivered=1 WHERE notice_key=?",
                vec![key.into()],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn defer_notice(&self, key: &str, next_ms: i64) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "UPDATE agena_superagent_outbox SET failures=failures+1,next_attempt_ms=? WHERE notice_key=? AND delivered=0", vec![next_ms.into(), key.into()])).await.map_err(storage)?;
        Ok(())
    }

    async fn reserve_chat(&self, message: &ChatMessage) -> Result<Option<ChatReply>> {
        message.validate()?;
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_superagent_chat_messages (session_id,message_id,message_text) VALUES (?,?,?) ON CONFLICT(session_id,message_id) DO NOTHING",
            vec![message.session_id.into(), message.message_id.clone().into(), message.text.clone().into()],
        )).await.map_err(storage)?;
        let row = self.database.query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT message_text,reply_json FROM agena_superagent_chat_messages WHERE session_id=? AND message_id=?",
            vec![message.session_id.into(), message.message_id.clone().into()],
        )).await.map_err(storage)?.ok_or(Error::NotFound)?;
        let text: String = row.try_get("", "message_text").map_err(storage)?;
        if text != message.text {
            return Err(Error::Conflict(
                "chat message ID was reused with different content",
            ));
        }
        let reply: Option<String> = row.try_get("", "reply_json").map_err(storage)?;
        reply
            .map(|text| serde_json::from_str(&text).map_err(storage))
            .transpose()
    }

    async fn finish_chat(&self, message: &ChatMessage, reply: ChatReply) -> Result<ChatReply> {
        let payload = serde_json::to_string(&reply).map_err(storage)?;
        if payload.len() > 4 * 1024 * 1024 {
            return Err(Error::Invalid("chat receipt exceeds its byte budget"));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE agena_superagent_chat_messages SET reply_json=? WHERE session_id=? AND message_id=? AND message_text=? AND reply_json IS NULL",
            vec![payload.into(), message.session_id.into(), message.message_id.clone().into(), message.text.clone().into()],
        )).await.map_err(storage)?;
        let row = self.database.query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT reply_json FROM agena_superagent_chat_messages WHERE session_id=? AND message_id=? AND message_text=?",
            vec![message.session_id.into(), message.message_id.clone().into(), message.text.clone().into()],
        )).await.map_err(storage)?.ok_or(Error::Conflict("chat reservation changed"))?;
        let text: String = row.try_get("", "reply_json").map_err(storage)?;
        serde_json::from_str(&text).map_err(storage)
    }

    async fn insert(&self, task: Task) -> Result<Task> {
        let payload = encode(&task)?;
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        self.database.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_superagent_tasks (id,source_session_id,workspace_id,request_key,state,revision,task_json,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(source_session_id,request_key) DO NOTHING",
            vec![task.id.to_string().into(), task.source_session_id.into(), task.workspace_id.into(), task.request_key.clone().into(), task.state.as_str().into(), task.revision.into(), payload.into(), task.created_at_ms.into(), task.updated_at_ms.into()],
        )).await.map_err(storage)?;
        let saved = self.rows(
            "SELECT task_json FROM agena_superagent_tasks WHERE source_session_id=? AND request_key=?",
            vec![task.source_session_id.into(), task.request_key.clone().into()],
        ).await?.pop().ok_or(Error::NotFound)?;
        if saved.goal != task.goal || saved.workspace_id != task.workspace_id {
            return Err(Error::Conflict(
                "request key was reused with different task input",
            ));
        }
        Ok(saved)
    }

    async fn get(&self, id: uuid::Uuid) -> Result<Task> {
        self.rows(
            "SELECT task_json FROM agena_superagent_tasks WHERE id=?",
            vec![id.to_string().into()],
        )
        .await?
        .pop()
        .ok_or(Error::NotFound)
    }

    async fn list(&self, query: TaskQuery) -> Result<Vec<Task>> {
        let limit = query.limit.unwrap_or(50);
        if !(1..=100).contains(&limit) || query.offset > 10000 {
            return Err(Error::Invalid(
                "limit must be 1–100 and offset at most 10000",
            ));
        }
        let mut sql = "SELECT task_json FROM agena_superagent_tasks WHERE 1=1".to_owned();
        let mut values = Vec::new();
        if let Some(session) = query.source_session_id {
            sql.push_str(" AND source_session_id=?");
            values.push(session.into());
        }
        if let Some(workspace) = query.workspace_id {
            sql.push_str(" AND workspace_id=?");
            values.push(workspace.into());
        }
        if let Some(state) = query.state {
            sql.push_str(" AND state=?");
            values.push(state.as_str().into());
        }
        sql.push_str(" ORDER BY created_at_ms DESC,id DESC LIMIT ? OFFSET ?");
        values.extend([
            Value::from(limit as i64),
            Value::from(i64::from(query.offset)),
        ]);
        self.rows(&sql, values).await
    }

    async fn pending(&self, limit: usize) -> Result<Vec<Task>> {
        self.rows(
            "SELECT task_json FROM agena_superagent_tasks WHERE state IN ('queued','starting','running','waiting','cancelling') ORDER BY CASE WHEN state='queued' THEN 1 ELSE 0 END,created_at_ms,id LIMIT ?",
            vec![(limit.clamp(1, 128) as i64).into()],
        ).await
    }

    async fn replace(&self, task: Task, expected_revision: i64) -> Result<Task> {
        if expected_revision.checked_add(1) != Some(task.revision) {
            return Err(Error::Conflict("task revision must advance by one"));
        }
        let payload = encode(&task)?;
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(storage)?;
        let transaction = self.database.begin().await.map_err(storage)?;
        let result = transaction.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE agena_superagent_tasks SET state=?,revision=?,task_json=?,updated_at_ms=? WHERE id=? AND revision=? AND source_session_id=? AND workspace_id=? AND request_key=? AND json_extract(task_json,'$.goal')=?",
            vec![task.state.as_str().into(), task.revision.into(), payload.into(), task.updated_at_ms.into(), task.id.to_string().into(), expected_revision.into(), task.source_session_id.into(), task.workspace_id.into(), task.request_key.clone().into(), task.goal.clone().into()],
        )).await.map_err(storage)?;
        if result.rows_affected() != 1 {
            return Err(Error::Conflict("task changed; refresh before retrying"));
        }
        if let Some(key) = task.work_notice_key() {
            transaction.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
                "INSERT INTO agena_superagent_outbox (notice_key,task_json,created_at_ms) VALUES (?,?,?) ON CONFLICT(notice_key) DO NOTHING",
                vec![key.into(), encode(&task)?.into(), task.updated_at_ms.into()])).await.map_err(storage)?;
        }
        transaction.commit().await.map_err(storage)?;
        Ok(task)
    }
}
