//! Durable integration grants, transactional inbox checkpoints and outbound
//! receipts. A provider crash never silently authorizes a repeated side effect.
use agena_assistant::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};
use std::sync::Arc;
use uuid::Uuid;
mod channel_access;
pub(crate) use channel_access::DECLARATIONS as CHANNEL_DECLARATIONS;

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_integration_bindings (id TEXT PRIMARY KEY, workspace_id INTEGER NULL CHECK(workspace_id IS NULL OR workspace_id>0), owner_bot_id TEXT NULL CHECK(owner_bot_id IS NULL OR length(owner_bot_id)=36), source_session_id INTEGER NOT NULL CHECK(source_session_id>0), request_key TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), state TEXT NOT NULL, next_poll_ms INTEGER NOT NULL, binding_json TEXT NOT NULL CHECK(json_valid(binding_json)), UNIQUE(source_session_id,request_key), CHECK((workspace_id IS NULL) <> (owner_bot_id IS NULL)))",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_bindings_workspace ON agena_integration_bindings(workspace_id,state,next_poll_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_bindings_owner ON agena_integration_bindings(owner_bot_id,state,next_poll_ms) WHERE owner_bot_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_due ON agena_integration_bindings(next_poll_ms,id) WHERE state='enabled'",
    "CREATE TABLE IF NOT EXISTS agena_integration_inbox (binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), event_id TEXT NOT NULL, state TEXT NOT NULL, received_at_ms INTEGER NOT NULL, event_json TEXT NOT NULL CHECK(json_valid(event_json) AND length(CAST(event_json AS BLOB))<=65536), PRIMARY KEY(binding_id,event_id)) WITHOUT ROWID",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_inbox_pending ON agena_integration_inbox(binding_id,state,received_at_ms)",
    "CREATE TABLE IF NOT EXISTS agena_integration_outbox (id TEXT PRIMARY KEY, binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), state TEXT NOT NULL, attempts INTEGER NOT NULL, next_attempt_ms INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, delivery_json TEXT NOT NULL CHECK(json_valid(delivery_json) AND length(CAST(delivery_json AS BLOB))<=65536))",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_outbox_ready ON agena_integration_outbox(binding_id,state,next_attempt_ms)",
];
pub struct SqliteIntegrationRepository {
    database: Arc<DatabaseConnection>,
}
impl SqliteIntegrationRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self { database }
    }
}
fn error(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}
fn statement(sql: &str, args: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, args)
}
fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(error)
}
fn decode<T: serde::de::DeserializeOwned>(row: sea_orm::QueryResult, column: &str) -> Result<T> {
    serde_json::from_str(&row.try_get::<String>("", column).map_err(error)?).map_err(error)
}
async fn read_binding(db: &impl ConnectionTrait, id: Uuid) -> Result<IntegrationBinding> {
    decode(
        db.query_one(statement(
            "SELECT binding_json FROM agena_integration_bindings WHERE id=?",
            vec![id.to_string().into()],
        ))
        .await
        .map_err(error)?
        .ok_or(Error::NotFound)?,
        "binding_json",
    )
}
async fn put_binding(
    db: &impl ConnectionTrait,
    b: &IntegrationBinding,
    expected: i64,
) -> Result<()> {
    let result=db.execute(statement("UPDATE agena_integration_bindings SET revision=?,state=?,next_poll_ms=?,binding_json=? WHERE id=? AND revision=?",vec![b.revision.into(),b.state.as_str().into(),b.next_poll_ms.into(),json(b)?.into(),b.id.to_string().into(),expected.into()])).await.map_err(error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Conflict(
            "integration changed; refresh and reconfirm",
        ));
    }
    Ok(())
}
async fn put_event(db: &impl ConnectionTrait, e: &IntegrationEvent) -> Result<()> {
    db.execute(statement(
        "UPDATE agena_integration_inbox SET state=?,event_json=? WHERE binding_id=? AND event_id=?",
        vec![
            e.state.clone().into(),
            json(e)?.into(),
            e.binding_id.to_string().into(),
            e.event.id.clone().into(),
        ],
    ))
    .await
    .map_err(error)?;
    Ok(())
}
async fn put_delivery(db: &impl ConnectionTrait, d: &IntegrationDelivery) -> Result<()> {
    db.execute(statement("UPDATE agena_integration_outbox SET state=?,attempts=?,next_attempt_ms=?,delivery_json=? WHERE id=? AND binding_id=?",vec![d.state.clone().into(),i64::from(d.attempts).into(),d.next_attempt_ms.into(),json(d)?.into(),d.id.clone().into(),d.binding_id.to_string().into()])).await.map_err(error)?;
    Ok(())
}

#[async_trait]
impl IntegrationRepository for SqliteIntegrationRepository {
    async fn channel_execution_actor(&self, id: Uuid, execution: &str) -> Result<Option<String>> {
        let row=self.database.query_one(statement("SELECT json_extract(event_json,'$.event.actor') AS actor FROM agena_integration_inbox WHERE binding_id=? AND json_extract(event_json,'$.execution_id')=? LIMIT 1",vec![id.to_string().into(),execution.into()])).await.map_err(error)?;
        row.map(|r| r.try_get("", "actor").map_err(error))
            .transpose()
    }

    async fn update_channel_policy(
        &self,
        id: Uuid,
        r: ChannelPolicyUpdate,
        actor: &str,
        now: i64,
    ) -> Result<IntegrationBinding> {
        self.channel_policy_write(id, r, actor, now).await
    }
    async fn channel_audit(&self, id: Uuid) -> Result<Vec<ChannelAccessAudit>> {
        self.channel_audit_read(id).await
    }
    async fn channel_sessions(&self, id: Uuid) -> Result<Vec<ChannelSessionRecord>> {
        self.channel_sessions_read(id).await
    }
    async fn save_channel_session(
        &self,
        record: ChannelSessionRecord,
    ) -> Result<ChannelSessionRecord> {
        self.channel_session_write(record).await
    }
    async fn channel_session(&self, id: Uuid, key: &str) -> Result<Option<ChannelSessionRecord>> {
        self.channel_session_read(id, key).await
    }
    async fn forget_channel_session(&self, id: Uuid, key: &str) -> Result<()> {
        self.channel_session_forget(id, key).await
    }

    async fn create_binding(&self, b: IntegrationBinding) -> Result<IntegrationBinding> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        if let Some(row)=tx.query_one(statement("SELECT binding_json FROM agena_integration_bindings WHERE source_session_id=? AND request_key=?",vec![b.source_session_id.into(),b.request_key.clone().into()])).await.map_err(error)?{
            let previous:IntegrationBinding=decode(row,"binding_json")?;
            if json(&previous.spec)?!=json(&b.spec)?{return Err(Error::Conflict("integration request key was reused with different settings"));}return Ok(previous);
        }
        let count = tx
            .query_one(statement(
                "SELECT COUNT(*) AS n FROM agena_integration_bindings WHERE workspace_id IS ? AND owner_bot_id IS ?",
                vec![
                    b.workspace_id.into(),
                    b.owner_bot_id.map(|id| id.to_string()).into(),
                ],
            ))
            .await
            .map_err(error)?
            .ok_or(Error::NotFound)?
            .try_get::<i64>("", "n")
            .map_err(error)?;
        if count >= 64 {
            return Err(Error::Conflict("workspace integration limit reached (64)"));
        }
        tx.execute(statement("INSERT INTO agena_integration_bindings(id,workspace_id,owner_bot_id,source_session_id,request_key,revision,state,next_poll_ms,binding_json) VALUES(?,?,?,?,?,?,?,?,?)",vec![b.id.to_string().into(),b.workspace_id.into(),b.owner_bot_id.map(|id| id.to_string()).into(),b.source_session_id.into(),b.request_key.clone().into(),b.revision.into(),b.state.as_str().into(),b.next_poll_ms.into(),json(&b)?.into()])).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(b)
    }
    async fn binding(&self, id: Uuid) -> Result<IntegrationBinding> {
        read_binding(self.database.as_ref(), id).await
    }
    async fn bindings(&self, scope: Option<IntegrationScope>) -> Result<Vec<IntegrationBinding>> {
        let (sql, args) = match scope {
            Some(IntegrationScope {
                workspace_id: Some(workspace),
                owner_bot_id: None,
            }) => (
                "SELECT binding_json FROM agena_integration_bindings WHERE workspace_id=? ORDER BY next_poll_ms,id LIMIT 64",
                vec![workspace.into()],
            ),
            Some(IntegrationScope {
                workspace_id: None,
                owner_bot_id: Some(bot),
            }) => (
                "SELECT binding_json FROM agena_integration_bindings WHERE owner_bot_id=? ORDER BY next_poll_ms,id LIMIT 64",
                vec![bot.to_string().into()],
            ),
            _ => (
                "SELECT binding_json FROM agena_integration_bindings WHERE state='enabled' OR EXISTS(SELECT 1 FROM agena_integration_inbox i WHERE i.binding_id=agena_integration_bindings.id AND i.state IN ('processing','executing') AND (json_extract(i.event_json,'$.execution_id') IS NOT NULL OR EXISTS(SELECT 1 FROM agena_deliveries d WHERE d.business_key='integration:'||i.binding_id||':'||i.event_id))) ORDER BY next_poll_ms,id LIMIT 256",
                vec![],
            ),
        };
        self.database
            .query_all(statement(sql, args))
            .await
            .map_err(error)?
            .into_iter()
            .map(|r| decode(r, "binding_json"))
            .collect()
    }
    async fn save_binding(
        &self,
        b: IntegrationBinding,
        expected: i64,
    ) -> Result<IntegrationBinding> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        put_binding(self.database.as_ref(), &b, expected).await?;
        Ok(b)
    }
    async fn accept_batch(
        &self,
        id: Uuid,
        expected: i64,
        batch: AdapterBatch,
        now: i64,
    ) -> Result<()> {
        if batch.events.len() > 32 || json(&batch.cursor)?.len() > 32768 {
            return Err(Error::Invalid(
                "provider batch exceeds 32 events or 32 KiB cursor",
            ));
        }
        for e in &batch.events {
            e.validate()?;
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let mut b = read_binding(&tx, id).await?;
        if b.revision != expected || b.state != BindingState::Enabled || now >= b.spec.expires_at_ms
        {
            return Err(Error::Conflict(
                "integration authority changed during poll; cursor was not committed",
            ));
        }
        // A signed platform removal is a security control, not a model/user
        // message. Apply all removals before admitting messages from the same
        // batch, even when chat quotas are exhausted. Revocation and cursor
        // commit together, so a replay cannot restore or double-apply access.
        channel_access::ingest_platform_revocations(&tx, &mut b, &batch.events, now).await?;
        let mut count=tx.query_one(statement("SELECT COUNT(*) AS n FROM agena_integration_inbox WHERE binding_id=? AND state IN ('pending','processing') AND json_extract(event_json,'$.epoch')=?",vec![id.to_string().into(),b.epoch.into()])).await.map_err(error)?.ok_or(Error::NotFound)?.try_get::<i64>("","n").map_err(error)?;
        for event in batch.events {
            // First accepted payload owns this identity. A provider reporting
            // edited issue text under an old 'opened' ID cannot replace it or
            // trigger more work; checkpoint progress is still allowed.
            let previous=tx.query_one(statement("SELECT event_json FROM agena_integration_inbox WHERE binding_id=? AND event_id=?",vec![id.to_string().into(),event.id.clone().into()])).await.map_err(error)?;
            if previous.is_some() {
                continue;
            }
            let note = b.rejection(&event, now).map(str::to_owned);
            let state = if note.is_some() { "ignored" } else { "pending" }.to_owned();
            if state == "pending" {
                count += 1;
                if count > 128 {
                    return Err(Error::Conflict(
                        "integration inbox is full; cursor remains unchanged until work drains",
                    ));
                }
            }
            let row = IntegrationEvent {
                binding_id: id,
                epoch: b.epoch,
                event,
                state,
                received_at_ms: now,
                session_id: None,
                execution_generation: None,
                execution_id: None,
                note,
            };
            tx.execute(statement("INSERT INTO agena_integration_inbox(binding_id,event_id,state,received_at_ms,event_json) VALUES(?,?,?,?,?)",vec![id.to_string().into(),row.event.id.clone().into(),row.state.clone().into(),now.into(),json(&row)?.into()])).await.map_err(error)?;
        }
        b.cursor = batch.cursor;
        b.poll_failures = 0;
        if b.state == BindingState::Enabled {
            b.last_error = None;
        }
        b.updated_at_ms = now;
        b.next_poll_ms = now
            + i64::from(
                batch
                    .retry_after_seconds
                    .unwrap_or(b.spec.poll_seconds)
                    .max(b.spec.poll_seconds)
                    .min(86400),
            ) * 1000;
        put_binding(&tx, &b, expected).await?;
        tx.commit().await.map_err(error)?;
        Ok(())
    }
    async fn next_event(&self, id: Uuid, now: i64) -> Result<Option<IntegrationEvent>> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let mut b = read_binding(&tx, id).await?;
        if b.state != BindingState::Enabled || now >= b.spec.expires_at_ms {
            return Ok(None);
        }
        let rows=tx.query_all(statement("SELECT event_json FROM agena_integration_inbox WHERE binding_id=? AND state IN ('pending','processing') ORDER BY CASE WHEN state='processing' THEN 0 WHEN json_extract(event_json,'$.event.text')='/access' OR json_extract(event_json,'$.event.text') LIKE '/access %' THEN 1 ELSE 2 END,received_at_ms,event_id LIMIT 32",vec![id.to_string().into()])).await.map_err(error)?;
        for row in rows {
            let mut e: IntegrationEvent = decode(row, "event_json")?;
            if e.epoch != b.epoch {
                e.state = "ignored".into();
                e.note = Some("authorization_revoked".into());
                put_event(&tx, &e).await?;
                continue;
            }
            if let Some(reason) = b.rejection(&e.event, now) {
                e.state = "ignored".into();
                e.note = Some(reason.into());
                put_event(&tx, &e).await?;
                continue;
            }
            if e.state == "processing" {
                tx.commit().await.map_err(error)?;
                return Ok(Some(e));
            }
            if b.spec.mode == IntegrationMode::Channel
                && (e.event.text.trim() == "/access" || e.event.text.trim().starts_with("/access "))
            {
                // /access is interpreted by a deterministic authorization
                // handler. An exhausted token/work budget cannot prevent an
                // administrator from revoking access. It never starts a model.
                e.state = "processing".into();
                put_event(&tx, &e).await?;
                tx.commit().await.map_err(error)?;
                return Ok(Some(e));
            }
            let day = Utc
                .timestamp_millis_opt(now)
                .single()
                .ok_or(Error::Invalid("invalid host clock"))?
                .format("%Y-%m-%d")
                .to_string();
            if b.day != day {
                b.day = day;
                b.day_admitted = 0;
            }
            if b.admitted >= b.spec.total_limit {
                b.last_error=Some("total event budget exhausted; existing work retains its grant until pause or expiry".into());
                put_binding(&tx, &b, b.revision).await?;
                tx.commit().await.map_err(error)?;
                return Ok(None);
            }
            if b.day_admitted >= b.spec.daily_limit {
                put_binding(&tx, &b, b.revision).await?;
                tx.commit().await.map_err(error)?;
                return Ok(None);
            }
            if !channel_access::charge_channel_user(&tx, &b, &e.event, &b.day).await? {
                e.state = "ignored".into();
                e.note = Some("per_user_daily_limit".into());
                put_event(&tx, &e).await?;
                continue;
            }
            b.admitted += 1;
            b.day_admitted += 1;
            e.state = "processing".into();
            put_event(&tx, &e).await?;
            put_binding(&tx, &b, b.revision).await?;
            tx.commit().await.map_err(error)?;
            return Ok(Some(e));
        }
        tx.commit().await.map_err(error)?;
        Ok(None)
    }
    async fn finish_event(&self, e: IntegrationEvent) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        put_event(self.database.as_ref(), &e).await
    }
    async fn events(&self, id: Uuid) -> Result<Vec<IntegrationEvent>> {
        self.database.query_all(statement("SELECT event_json FROM agena_integration_inbox WHERE binding_id=? ORDER BY received_at_ms DESC,event_id LIMIT 100",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"event_json")).collect()
    }
    async fn execution_events(&self, id: Uuid) -> Result<Vec<(IntegrationEvent, i64)>> {
        // Read the kernel's immutable receipt using the same global business
        // key as dispatch. No independent worker state or timing is inferred.
        let rows = self.database.query_all(statement("SELECT i.event_json,e.session_id,e.id AS resume_execution_id,COALESCE(json_extract(i.event_json,'$.execution_generation'),d.resume_generation) AS resume_generation,e.created_at_ms FROM agena_integration_inbox i LEFT JOIN agena_deliveries d ON d.business_key='integration:'||i.binding_id||':'||i.event_id JOIN agena_executions e ON e.id=COALESCE(json_extract(i.event_json,'$.execution_id'),d.resume_execution_id) AND e.session_id=COALESCE(json_extract(i.event_json,'$.session_id'),d.session_id) WHERE i.binding_id=? AND i.state IN ('processing','executing') ORDER BY i.received_at_ms,i.event_id LIMIT 128", vec![id.to_string().into()])).await.map_err(error)?;
        rows.into_iter()
            .map(|row| {
                let mut event: IntegrationEvent =
                    serde_json::from_str(&row.try_get::<String>("", "event_json").map_err(error)?)
                        .map_err(error)?;
                event.session_id = Some(row.try_get("", "session_id").map_err(error)?);
                event.execution_id = Some(row.try_get("", "resume_execution_id").map_err(error)?);
                event.execution_generation =
                    Some(row.try_get("", "resume_generation").map_err(error)?);
                Ok((event, row.try_get("", "created_at_ms").map_err(error)?))
            })
            .collect()
    }
    async fn enqueue_delivery(&self, d: IntegrationDelivery) -> Result<()> {
        if d.text.len() > 24576 || d.id.len() > 160 {
            return Err(Error::Invalid("channel delivery exceeds bounded contract"));
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        self.database.execute(statement("INSERT INTO agena_integration_outbox(id,binding_id,state,attempts,next_attempt_ms,created_at_ms,delivery_json) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING",vec![d.id.clone().into(),d.binding_id.to_string().into(),d.state.clone().into(),i64::from(d.attempts).into(),d.next_attempt_ms.into(),d.created_at_ms.into(),json(&d)?.into()])).await.map_err(error)?;
        Ok(())
    }
    async fn deliveries(&self, id: Uuid) -> Result<Vec<IntegrationDelivery>> {
        self.database.query_all(statement("SELECT delivery_json FROM agena_integration_outbox WHERE binding_id=? ORDER BY created_at_ms DESC,id LIMIT 100",vec![id.to_string().into()])).await.map_err(error)?.into_iter().map(|r|decode(r,"delivery_json")).collect()
    }
    async fn claim_delivery(&self, id: Uuid, now: i64) -> Result<Option<IntegrationDelivery>> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let b = read_binding(&tx, id).await?;
        if b.state != BindingState::Enabled || now >= b.spec.expires_at_ms {
            return Ok(None);
        }
        let rows=tx.query_all(statement("SELECT delivery_json FROM agena_integration_outbox WHERE binding_id=? AND state='queued' AND next_attempt_ms<=? ORDER BY created_at_ms,id LIMIT 32",vec![id.to_string().into(),now.into()])).await.map_err(error)?;
        for row in rows {
            let mut d: IntegrationDelivery = decode(row, "delivery_json")?;
            if b.spec.mode == IntegrationMode::Channel
                && !d
                    .route
                    .as_ref()
                    .is_some_and(|route| b.authorizes_output(route, now))
            {
                d.state = "suppressed".into();
                d.error =
                    Some("channel identity, membership or policy changed before delivery".into());
                put_delivery(&tx, &d).await?;
                continue;
            }
            d.state = "sending".into();
            d.attempts += 1;
            put_delivery(&tx, &d).await?;
            tx.commit().await.map_err(error)?;
            return Ok(Some(d));
        }
        tx.commit().await.map_err(error)?;
        Ok(None)
    }
    async fn finish_delivery(&self, d: IntegrationDelivery) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        put_delivery(self.database.as_ref(), &d).await
    }
    async fn recover_sending(&self) -> Result<()> {
        loop {
            let _permit = crate::acquire_write_permit(&self.database)
                .await
                .map_err(error)?;
            let tx = self.database.begin().await.map_err(error)?;
            let rows = tx.query_all(statement(
                "SELECT delivery_json FROM agena_integration_outbox WHERE state='sending' ORDER BY id LIMIT 128", vec![],
            )).await.map_err(error)?;
            if rows.is_empty() {
                return Ok(());
            }
            for row in rows {
                let mut d: IntegrationDelivery = decode(row, "delivery_json")?;
                d.state = "uncertain".into();
                d.error =
                    Some("process ended before a remote acknowledgement; no blind replay".into());
                put_delivery(&tx, &d).await?;
            }
            tx.commit().await.map_err(error)?;
        }
    }
    async fn unexported_executions(
        &self,
        id: Uuid,
        session: i64,
    ) -> Result<Vec<(String, i64, String)>> {
        // Dedicated channel roots contain only their own authorized inputs and
        // control continuations. Select terminal facts; content stays behind
        // SessionKernel's owned text reader, never in an ad-hoc SQL projection.
        let prefix = format!("{}:", id);
        let rows=self.database.query_all(statement("SELECT e.id AS execution,a.generation,a.phase FROM agena_executions e JOIN agena_attempts a ON a.execution_id=e.id AND a.generation=e.current_generation WHERE e.session_id=? AND a.phase IN ('succeeded','failed','cancelled','interrupted') AND NOT EXISTS(SELECT 1 FROM agena_integration_outbox o WHERE o.id=?||e.id||':'||a.generation) ORDER BY a.finished_at_ms,e.id LIMIT 16",vec![session.into(),prefix.into()])).await.map_err(error)?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get("", "execution").map_err(error)?,
                    r.try_get("", "generation").map_err(error)?,
                    r.try_get("", "phase").map_err(error)?,
                ))
            })
            .collect()
    }
}
