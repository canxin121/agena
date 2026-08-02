use std::{marker::PhantomData, sync::Arc};

use agena_domain::{EventEnvelope, EventFilter, EventMeta, EventScope, KindMatcher};
use agena_storage::{EventStore, EventStoreError, ReverseStoreRange, StoreRange};
use async_trait::async_trait;
use chrono::TimeZone;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};
use uuid::Uuid;

const TABLE: &str = "agena_events";

/// SQLite implementation of the generic durable event-store contract.
pub struct SeaEventStore<K> {
    db: Arc<DatabaseConnection>,
    _kind: PhantomData<fn() -> K>,
}
impl<K> SeaEventStore<K> {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self {
            db,
            _kind: PhantomData,
        }
    }
    pub fn db(&self) -> &Arc<DatabaseConnection> {
        &self.db
    }
}

#[async_trait]
impl<K> EventStore<K> for SeaEventStore<K>
where
    K: KindMatcher + Clone + Send + Sync + 'static + serde::Serialize + serde::de::DeserializeOwned,
{
    async fn append_batch(&self, events: &[EventEnvelope<K>]) -> Result<(), EventStoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let txn = self.db.begin().await.map_err(backend_error)?;
        for event in events {
            let payload = serde_json::to_value(&event.kind).map_err(EventStoreError::Serde)?;
            let result = txn.execute(statement(format!("INSERT INTO {TABLE} (event_uuid, seq_global, seq_session, session_id, workspace_id, kind_tag, envelope_schema, payload_json, causation_uuid, correlation_uuid, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"), [event.meta.id.to_string().into(), event.meta.seq_global.into(), event.meta.seq_session.into(), event.meta.session_id.into(), event.meta.workspace_id.into(), event.kind.tag().to_string().into(), (event.meta.envelope_schema as i32).into(), payload.into(), event.meta.causation_id.map(|id| id.to_string()).into(), event.meta.correlation_id.map(|id| id.to_string()).into(), event.meta.created_at.timestamp_millis().into()])).await;
            if let Err(error) = result {
                let message = error.to_string();
                let _ = txn.rollback().await;
                return if message.contains("idx_agena_events_seq_global")
                    || message.contains("agena_events.seq_global")
                {
                    Err(EventStoreError::DuplicateSeq(event.meta.seq_global))
                } else if message.contains("agena_events.seq_session")
                    || message.contains("uq_agena_events_session_seq")
                {
                    Err(EventStoreError::DuplicateSessionSeq {
                        session_id: event.meta.session_id.unwrap_or_default(),
                        seq_session: event.meta.seq_session.unwrap_or_default(),
                    })
                } else {
                    Err(EventStoreError::Backend(message))
                };
            }
        }
        txn.commit().await.map_err(backend_error)?;
        Ok(())
    }
    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<EventEnvelope<K>>, EventStoreError> {
        let mut clauses = vec!["seq_global > ?".to_owned()];
        let mut values = vec![range.after_seq_global.into()];
        match filter.scope {
            EventScope::Global => {}
            EventScope::Workspace { workspace_id } => {
                clauses.push("workspace_id = ?".to_owned());
                values.push(workspace_id.into());
            }
            EventScope::Session { session_id } => {
                clauses.push("session_id = ?".to_owned());
                values.push(session_id.into());
            }
        }
        if let Some(kinds) = &filter.kinds {
            if kinds.is_empty() {
                return Ok(Vec::new());
            }
            clauses.push(format!("kind_tag IN ({})", placeholders(kinds.len())));
            values.extend(kinds.iter().map(|tag| tag.to_string().into()));
        }
        if let Some(since) = filter.since_seq_global {
            clauses.push("seq_global > ?".to_owned());
            values.push(since.into());
        }
        values.push((range.limit as i64).into());
        self.db.query_all(statement(format!("SELECT event_uuid, seq_global, seq_session, session_id, workspace_id, kind_tag, envelope_schema, payload_json, causation_uuid, correlation_uuid, created_at_ms FROM {TABLE} WHERE {} ORDER BY seq_global ASC LIMIT ?", clauses.join(" AND ")), values)).await.map_err(backend_error)?.into_iter().map(event_from_row).collect()
    }
    async fn range_before(
        &self,
        filter: &EventFilter,
        range: ReverseStoreRange,
    ) -> Result<Vec<EventEnvelope<K>>, EventStoreError> {
        let mut clauses = Vec::<String>::new();
        let mut values = Vec::<Value>::new();
        if let Some(before_seq_global) = range.before_seq_global {
            clauses.push("seq_global < ?".to_owned());
            values.push(before_seq_global.into());
        }
        match filter.scope {
            EventScope::Global => {}
            EventScope::Workspace { workspace_id } => {
                clauses.push("workspace_id = ?".to_owned());
                values.push(workspace_id.into());
            }
            EventScope::Session { session_id } => {
                clauses.push("session_id = ?".to_owned());
                values.push(session_id.into());
            }
        }
        if let Some(kinds) = &filter.kinds {
            if kinds.is_empty() {
                return Ok(Vec::new());
            }
            clauses.push(format!("kind_tag IN ({})", placeholders(kinds.len())));
            values.extend(kinds.iter().map(|tag| tag.to_string().into()));
        }
        if let Some(since) = filter.since_seq_global {
            clauses.push("seq_global > ?".to_owned());
            values.push(since.into());
        }
        values.push((range.limit as i64).into());
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        self.db.query_all(statement(format!("SELECT event_uuid, seq_global, seq_session, session_id, workspace_id, kind_tag, envelope_schema, payload_json, causation_uuid, correlation_uuid, created_at_ms FROM {TABLE} {where_clause} ORDER BY seq_global DESC LIMIT ?"), values)).await.map_err(backend_error)?.into_iter().map(event_from_row).collect()
    }
    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError> {
        scalar_max(
            self.db.as_ref(),
            "SELECT MAX(seq_global) AS value FROM agena_events",
            Vec::new(),
        )
        .await
    }
    async fn session_high_watermark(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, EventStoreError> {
        scalar_max(
            self.db.as_ref(),
            "SELECT MAX(seq_session) AS value FROM agena_events WHERE session_id = ?",
            vec![session_id.into()],
        )
        .await
    }
}
async fn scalar_max(
    db: &DatabaseConnection,
    sql: &str,
    values: Vec<Value>,
) -> Result<Option<i64>, EventStoreError> {
    let row = db
        .query_one(statement(sql.to_owned(), values))
        .await
        .map_err(backend_error)?;
    row.map(|row| {
        row.try_get::<Option<i64>>("", "value")
            .map_err(backend_error)
    })
    .transpose()
    .map(Option::flatten)
}
fn event_from_row<K: serde::de::DeserializeOwned>(
    row: sea_orm::QueryResult,
) -> Result<EventEnvelope<K>, EventStoreError> {
    let parse_uuid = |value: String, field: &str| {
        Uuid::parse_str(&value)
            .map_err(|e| EventStoreError::InvalidRange(format!("{field} not a valid uuid: {e}")))
    };
    let created_at_ms: i64 = row.try_get("", "created_at_ms").map_err(backend_error)?;
    Ok(EventEnvelope {
        meta: EventMeta {
            id: parse_uuid(
                row.try_get("", "event_uuid").map_err(backend_error)?,
                "event_uuid",
            )?,
            seq_global: row.try_get("", "seq_global").map_err(backend_error)?,
            seq_session: row.try_get("", "seq_session").map_err(backend_error)?,
            session_id: row.try_get("", "session_id").map_err(backend_error)?,
            workspace_id: row.try_get("", "workspace_id").map_err(backend_error)?,
            created_at: chrono::Utc
                .timestamp_millis_opt(created_at_ms)
                .single()
                .ok_or_else(|| {
                    EventStoreError::InvalidRange(format!(
                        "created_at_ms out of range: {created_at_ms}"
                    ))
                })?,
            causation_id: row
                .try_get::<Option<String>>("", "causation_uuid")
                .map_err(backend_error)?
                .map(|value| parse_uuid(value, "causation_uuid"))
                .transpose()?,
            correlation_id: row
                .try_get::<Option<String>>("", "correlation_uuid")
                .map_err(backend_error)?
                .map(|value| parse_uuid(value, "correlation_uuid"))
                .transpose()?,
            envelope_schema: row
                .try_get::<i32>("", "envelope_schema")
                .map_err(backend_error)? as u32,
        },
        kind: serde_json::from_value(row.try_get("", "payload_json").map_err(backend_error)?)
            .map_err(EventStoreError::Serde)?,
    })
}
fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}
fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn backend_error(error: impl std::fmt::Display) -> EventStoreError {
    EventStoreError::Backend(error.to_string())
}
