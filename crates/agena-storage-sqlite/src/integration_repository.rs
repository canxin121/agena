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
mod channel_participants;
mod inbox;
mod outbox;
pub(crate) use channel_access::DECLARATIONS as CHANNEL_DECLARATIONS;
pub(crate) use channel_participants::DECLARATIONS as PARTICIPANT_DECLARATIONS;
use inbox::{decode_event, insert_event, put_event};
use outbox::{decode_delivery, insert_delivery, put_delivery};

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_integration_bindings (id TEXT PRIMARY KEY, workspace_id INTEGER NULL CHECK(workspace_id IS NULL OR workspace_id>0), owner_bot_id TEXT NULL CHECK(owner_bot_id IS NULL OR length(owner_bot_id)=36), source_session_id INTEGER NOT NULL CHECK(source_session_id>0), request_key TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), state TEXT NOT NULL, next_poll_ms INTEGER NOT NULL, binding_json TEXT NOT NULL CHECK(json_valid(binding_json)), UNIQUE(source_session_id,request_key), CHECK((workspace_id IS NULL) <> (owner_bot_id IS NULL)))",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_bindings_workspace ON agena_integration_bindings(workspace_id,next_poll_ms,id) WHERE workspace_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_bindings_owner ON agena_integration_bindings(owner_bot_id,next_poll_ms,id) WHERE owner_bot_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_due ON agena_integration_bindings(next_poll_ms,id) WHERE state='enabled'",
    "CREATE TABLE IF NOT EXISTS agena_integration_inbox (
        id INTEGER PRIMARY KEY,
        binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id), event_id TEXT NOT NULL,
        epoch INTEGER NOT NULL, state TEXT NOT NULL, received_at_ms INTEGER NOT NULL,
        is_access INTEGER NOT NULL CHECK(is_access IN (0,1)),
        session_id INTEGER, execution_generation INTEGER, execution_id TEXT, note TEXT,
        queue_priority INTEGER GENERATED ALWAYS AS (CASE WHEN state='processing' THEN 0 WHEN is_access=1 THEN 1 ELSE 2 END) VIRTUAL,
        UNIQUE(binding_id,event_id)
    )",
    "CREATE TABLE IF NOT EXISTS agena_integration_event_payloads (
        inbox_id INTEGER PRIMARY KEY REFERENCES agena_integration_inbox(id) ON DELETE CASCADE,
        event_json TEXT NOT NULL CHECK(json_valid(event_json) AND length(CAST(event_json AS BLOB))<=65536)
    )",
    "CREATE VIEW IF NOT EXISTS agena_integration_events AS
     SELECT i.*,p.event_json FROM agena_integration_inbox i
     CROSS JOIN agena_integration_event_payloads p ON p.inbox_id=i.id",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_inbox_pending ON agena_integration_inbox(binding_id,epoch,queue_priority,received_at_ms,event_id) WHERE state IN ('pending','processing')",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_inbox_active ON agena_integration_inbox(binding_id,received_at_ms,event_id) WHERE state IN ('processing','executing')",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_inbox_recent ON agena_integration_inbox(binding_id,received_at_ms DESC,event_id)",
    "CREATE TABLE IF NOT EXISTS agena_integration_outbox (
        delivery_key INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
        binding_id TEXT NOT NULL REFERENCES agena_integration_bindings(id),
        state TEXT NOT NULL, attempts INTEGER NOT NULL, next_attempt_ms INTEGER NOT NULL,
        created_at_ms INTEGER NOT NULL, remote_id TEXT, error TEXT,
        route_json TEXT CHECK(route_json IS NULL OR json_valid(route_json))
    )",
    "CREATE TABLE IF NOT EXISTS agena_integration_delivery_text (
        delivery_key INTEGER PRIMARY KEY REFERENCES agena_integration_outbox(delivery_key) ON DELETE CASCADE,
        text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) BETWEEN 1 AND 24576)
    )",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_outbox_ready ON agena_integration_outbox(binding_id,state,next_attempt_ms,created_at_ms,id) WHERE state='queued' OR state='uncertain'",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_outbox_recent ON agena_integration_outbox(binding_id,created_at_ms DESC,id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_integration_outbox_sending ON agena_integration_outbox(id) WHERE state='sending'",
];

const RECONCILE_BINDINGS_SQL: &str = "
    WITH active AS (
        SELECT id,next_poll_ms FROM agena_integration_bindings WHERE state='enabled'
        UNION ALL
        SELECT DISTINCT b.id,b.next_poll_ms
        FROM agena_integration_inbox i
        CROSS JOIN agena_integration_bindings b ON b.id=i.binding_id
        WHERE i.state IN ('processing','executing') AND b.state!='enabled'
          AND (i.execution_id IS NOT NULL
               OR EXISTS(SELECT 1 FROM agena_deliveries d
                         WHERE d.business_key='integration:'||i.binding_id||':'||i.event_id))
        ORDER BY next_poll_ms,id LIMIT 256
    )
    SELECT b.binding_json FROM active
    CROSS JOIN agena_integration_bindings b ON b.id=active.id
    ORDER BY active.next_poll_ms,active.id";
const CHANNEL_ATTENTION: &str = "i.decision_json IS NULL AND NOT EXISTS(
    SELECT 1 FROM agena_integration_outbox o WHERE o.id=c.binding_id||':attention:'||i.id
)";

// New deliveries use a zero deadline. The equality range is already ordered
// by creation time, and the retry ranges skip those zero-deadline rows.
const READY_DELIVERIES_SQL: &str = "WITH due AS (
    SELECT id,created_at_ms FROM (
        SELECT id,created_at_ms FROM agena_integration_outbox INDEXED BY idx_agena_integration_outbox_ready
        WHERE binding_id=?1 AND state='queued' AND next_attempt_ms=0 AND ?2>=0
        ORDER BY created_at_ms,id LIMIT 32
    ) UNION ALL
    SELECT id,created_at_ms FROM (
        SELECT id,created_at_ms FROM agena_integration_outbox INDEXED BY idx_agena_integration_outbox_ready
        WHERE binding_id=?1 AND state='queued' AND next_attempt_ms>0 AND next_attempt_ms<=?2
        ORDER BY created_at_ms,id LIMIT 32
    ) UNION ALL
    SELECT id,created_at_ms FROM (
        SELECT id,created_at_ms FROM agena_integration_outbox INDEXED BY idx_agena_integration_outbox_ready
        WHERE binding_id=?1 AND state='queued' AND next_attempt_ms<0 AND next_attempt_ms<=?2
        ORDER BY created_at_ms,id LIMIT 32
    )
)
SELECT d.* FROM (
    SELECT id,created_at_ms FROM due ORDER BY created_at_ms,id LIMIT 32
) ready CROSS JOIN agena_integration_outbox d ON d.id=ready.id
ORDER BY ready.created_at_ms,ready.id";

pub struct SqliteIntegrationRepository {
    database: Arc<DatabaseConnection>,
    reader: Arc<DatabaseConnection>,
}
impl SqliteIntegrationRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self::with_reader(database.clone(), database)
    }

    pub fn with_reader(database: Arc<DatabaseConnection>, reader: Arc<DatabaseConnection>) -> Self {
        Self { database, reader }
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
fn pack_receipt(value: &IntegrationBinding) -> Result<Vec<u8>> {
    let raw = serde_json::to_vec(value).map_err(error)?;
    crate::receipt_payload::pack(&raw).map_err(error)
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

#[async_trait]
impl IntegrationRepository for SqliteIntegrationRepository {
    async fn channel_execution_actor(&self, id: Uuid, execution: &str) -> Result<Option<String>> {
        let row = self
            .reader
            .query_one(statement(
                "SELECT json_extract(event_json,'$.actor') AS actor
             FROM agena_integration_events WHERE binding_id=? AND execution_id=? LIMIT 1",
                vec![id.to_string().into(), execution.into()],
            ))
            .await
            .map_err(error)?;
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
    async fn review_channel_binding(
        &self,
        binding: IntegrationBinding,
        expected_revision: i64,
        actor: &str,
        now: i64,
    ) -> Result<IntegrationBinding> {
        self.channel_review_write(binding, expected_revision, actor, now)
            .await
    }
    async fn recover_channel_session(
        &self,
        expected: ChannelSessionRecord,
        current: ChannelSessionRecord,
        now: i64,
    ) -> Result<ChannelSessionRecord> {
        self.channel_session_recover(expected, current, now).await
    }
    async fn channel_audit(&self, id: Uuid) -> Result<Vec<ChannelAccessAudit>> {
        self.channel_audit_read(id).await
    }
    async fn channel_sessions(&self, id: Uuid) -> Result<Vec<ChannelSessionRecord>> {
        self.channel_sessions_read(id).await
    }
    async fn channel_sessions_with_output(&self, id: Uuid) -> Result<Vec<ChannelSessionRecord>> {
        self.reader
            .query_all(statement(
                "SELECT record_json FROM agena_channel_sessions
             WHERE binding_id=? AND has_work=1 ORDER BY epoch DESC,session_id DESC LIMIT 2048",
                vec![id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .into_iter()
            .map(|row| decode(row, "record_json"))
            .collect()
    }
    async fn unnotified_channel_interactions(&self, id: Uuid, session: i64) -> Result<Vec<i64>> {
        self.reader
            .query_all(statement(
                &format!(
                    "SELECT i.id FROM agena_channel_sessions c
                 CROSS JOIN agena_interactions i ON i.session_id=c.session_id
                 WHERE c.session_id=? AND c.binding_id=? AND {CHANNEL_ATTENTION}
                 ORDER BY i.id LIMIT 32"
                ),
                vec![session.into(), id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .into_iter()
            .map(|row| row.try_get("", "id").map_err(error))
            .collect()
    }
    async fn observed_channel_rooms(&self, id: Uuid) -> Result<Vec<ObservedChannelRoom>> {
        self.observed_channel_rooms_read(id).await
    }
    async fn channel_participants(
        &self,
        id: Uuid,
        query: ChannelParticipantsQuery,
    ) -> Result<ChannelParticipantPage> {
        self.channel_participants_read(id, query).await
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
        let (count_sql, owner) = match (b.workspace_id, b.owner_bot_id) {
            (Some(workspace), None) => (
                "SELECT count(*) AS n FROM agena_integration_bindings WHERE workspace_id=?",
                workspace.into(),
            ),
            (None, Some(bot)) => (
                "SELECT count(*) AS n FROM agena_integration_bindings WHERE owner_bot_id=?",
                bot.to_string().into(),
            ),
            _ => return Err(Error::Invalid("binding must have exactly one owner")),
        };
        let count = tx
            .query_one(statement(count_sql, vec![owner]))
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
        read_binding(self.reader.as_ref(), id).await
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
            _ => (RECONCILE_BINDINGS_SQL, vec![]),
        };
        self.reader
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
        if b.revision != expected || b.state != BindingState::Enabled || b.spec.expired_at(now) {
            return Err(Error::Conflict(
                "integration authority changed during poll; cursor was not committed",
            ));
        }
        // A signed platform removal is a security control, not a model/user
        // message. Apply all removals before admitting messages from the same
        // batch, even when chat quotas are exhausted. Revocation and cursor
        // commit together, so a replay cannot restore or double-apply access.
        channel_access::ingest_platform_revocations(&tx, &mut b, &batch.events, now).await?;
        let mut count = tx
            .query_one(statement(
                "SELECT COUNT(*) AS n FROM agena_integration_inbox
             WHERE binding_id=? AND epoch=? AND state IN ('pending','processing')",
                vec![id.to_string().into(), b.epoch.into()],
            ))
            .await
            .map_err(error)?
            .ok_or(Error::NotFound)?
            .try_get::<i64>("", "n")
            .map_err(error)?;
        for event in batch.events {
            // First accepted payload owns this identity. A provider reporting
            // edited issue text under an old 'opened' ID cannot replace it or
            // trigger more work; checkpoint progress is still allowed.
            let note = b.rejection(&event, now).map(str::to_owned);
            let row = IntegrationEvent {
                binding_id: id,
                epoch: b.epoch,
                event,
                state: if note.is_some() { "ignored" } else { "pending" }.into(),
                received_at_ms: now,
                session_id: None,
                execution_generation: None,
                execution_id: None,
                note,
            };
            if !insert_event(&tx, &row).await? {
                continue;
            }
            if row.state == "pending" {
                count += 1;
                if count > 128 {
                    return Err(Error::Conflict(
                        "integration inbox is full; cursor remains unchanged until work drains",
                    ));
                }
            }
            channel_participants::record_interaction(&tx, &b, &row.event, now).await?;
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
        if b.state != BindingState::Enabled || b.spec.expired_at(now) {
            return Ok(None);
        }
        inbox::reap_stale_events(&tx, id, b.epoch).await?;
        let rows = tx
            .query_all(statement(
                "SELECT * FROM agena_integration_events
                 WHERE binding_id=? AND epoch=? AND state IN ('pending','processing')
                 ORDER BY queue_priority,received_at_ms,event_id LIMIT 32",
                vec![id.to_string().into(), b.epoch.into()],
            ))
            .await
            .map_err(error)?;
        for row in rows {
            let mut e = decode_event(&row)?;
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
            if b.spec.total_budget_exhausted(b.admitted) {
                b.last_error=Some("total event budget exhausted; existing work retains its grant until pause or expiry".into());
                put_binding(&tx, &b, b.revision).await?;
                tx.commit().await.map_err(error)?;
                return Ok(None);
            }
            if b.spec.daily_budget_exhausted(b.day_admitted) {
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
            b.admitted = b.admitted.saturating_add(1);
            b.day_admitted = b.day_admitted.saturating_add(1);
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
        self.reader
            .query_all(statement(
                "SELECT * FROM agena_integration_events WHERE binding_id=?
             ORDER BY received_at_ms DESC,event_id LIMIT 100",
                vec![id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .iter()
            .map(decode_event)
            .collect()
    }
    async fn execution_events(&self, id: Uuid) -> Result<Vec<(IntegrationEvent, i64)>> {
        // Read the kernel's immutable receipt using the same global business
        // key as dispatch. No independent worker state or timing is inferred.
        let rows = self.reader.query_all(statement(
            "SELECT i.*,p.event_json,e.session_id AS resume_session_id,e.id AS resume_execution_id,
                    COALESCE(i.execution_generation,d.resume_generation) AS resume_generation,e.created_at_ms
             FROM agena_integration_inbox i
             LEFT JOIN agena_deliveries d ON d.business_key='integration:'||i.binding_id||':'||i.event_id
             CROSS JOIN agena_executions e ON e.id=COALESCE(i.execution_id,d.resume_execution_id)
                AND e.session_id=COALESCE(i.session_id,d.session_id)
             CROSS JOIN agena_integration_event_payloads p ON p.inbox_id=i.id
             WHERE i.binding_id=? AND i.state IN ('processing','executing')
             ORDER BY i.received_at_ms,i.event_id LIMIT 128",
            vec![id.to_string().into()],
        )).await.map_err(error)?;
        rows.into_iter()
            .map(|row| {
                let mut event = decode_event(&row)?;
                event.session_id = Some(row.try_get("", "resume_session_id").map_err(error)?);
                event.execution_id = Some(row.try_get("", "resume_execution_id").map_err(error)?);
                event.execution_generation =
                    Some(row.try_get("", "resume_generation").map_err(error)?);
                Ok((event, row.try_get("", "created_at_ms").map_err(error)?))
            })
            .collect()
    }
    async fn enqueue_delivery(&self, d: IntegrationDelivery) -> Result<()> {
        insert_delivery(&self.database, d, false).await
    }
    async fn enqueue_execution_delivery(&self, d: IntegrationDelivery) -> Result<()> {
        insert_delivery(&self.database, d, true).await
    }
    async fn deliveries(&self, id: Uuid) -> Result<Vec<IntegrationDelivery>> {
        self.reader
            .query_all(statement(
                "SELECT d.*,COALESCE(p.text,'') AS text FROM agena_integration_outbox d
             LEFT JOIN agena_integration_delivery_text p ON p.delivery_key=d.delivery_key
             WHERE d.binding_id=? ORDER BY d.created_at_ms DESC,d.id LIMIT 100",
                vec![id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .iter()
            .map(|row| decode_delivery(row, row.try_get("", "text").map_err(error)?))
            .collect()
    }
    async fn claim_delivery(&self, id: Uuid, now: i64) -> Result<Option<IntegrationDelivery>> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        let tx = self.database.begin().await.map_err(error)?;
        let b = read_binding(&tx, id).await?;
        if b.state != BindingState::Enabled || b.spec.expired_at(now) {
            return Ok(None);
        }
        let rows = tx
            .query_all(statement(
                READY_DELIVERIES_SQL,
                vec![id.to_string().into(), now.into()],
            ))
            .await
            .map_err(error)?;
        for row in rows {
            let mut d = decode_delivery(&row, String::new())?;
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
            d.text =
                outbox::read_delivery_text(&tx, row.try_get("", "delivery_key").map_err(error)?)
                    .await?;
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
    async fn retry_uncertain_deliveries(&self, id: Uuid, retry_at_ms: i64) -> Result<()> {
        const RETRYABLE: &str = "SELECT delivery_key FROM (
            SELECT delivery_key,state,attempts FROM agena_integration_outbox
            WHERE binding_id=?1 ORDER BY created_at_ms DESC,id LIMIT 100
        ) WHERE state='uncertain' AND attempts<5";
        if self
            .reader
            .query_one(statement(
                &format!("{RETRYABLE} LIMIT 1"),
                vec![id.to_string().into()],
            ))
            .await
            .map_err(error)?
            .is_none()
        {
            return Ok(());
        }
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(error)?;
        self.database
            .execute(statement(
                &format!(
                    "UPDATE agena_integration_outbox SET state='queued',next_attempt_ms=?2
                 WHERE delivery_key IN ({RETRYABLE})"
                ),
                vec![id.to_string().into(), retry_at_ms.into()],
            ))
            .await
            .map_err(error)?;
        Ok(())
    }
    async fn recover_sending(&self) -> Result<()> {
        loop {
            let _permit = crate::acquire_write_permit(&self.database)
                .await
                .map_err(error)?;
            let result = self.database.execute(statement(
                "UPDATE agena_integration_outbox
                 SET state='uncertain',error='process ended before a remote acknowledgement; no blind replay'
                 WHERE delivery_key IN (
                     SELECT delivery_key FROM agena_integration_outbox WHERE state='sending'
                     ORDER BY id LIMIT 128
                 )",
                vec![],
            )).await.map_err(error)?;
            if result.rows_affected() < 128 {
                return Ok(());
            }
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
        let rows=self.reader.query_all(statement(
            "SELECT p.execution_id AS execution,p.generation,a.phase
             FROM agena_channel_sessions c
             CROSS JOIN agena_channel_pending_outputs p ON p.session_id=c.session_id
             CROSS JOIN agena_attempts a ON a.execution_id=p.execution_id AND a.generation=p.generation
             WHERE c.session_id=? AND c.binding_id=?
             ORDER BY p.finished_at_ms,p.execution_id,p.generation LIMIT 16",
            vec![session.into(),id.to_string().into()],
        )).await.map_err(error)?;
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
