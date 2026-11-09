//! Durable workspace preferences and consent-bounded proactive scheduling.
use agena_assistant::*;
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait, Value,
};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) const DECLARATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_assistant_profiles (workspace_id INTEGER PRIMARY KEY CHECK(workspace_id>0), revision INTEGER NOT NULL CHECK(revision>0), profile_json TEXT NOT NULL CHECK(json_valid(profile_json)))",
    "CREATE TABLE IF NOT EXISTS agena_assistant_profile_receipts (workspace_id INTEGER NOT NULL, request_key TEXT NOT NULL, input_json TEXT NOT NULL CHECK(json_valid(input_json)), result_json TEXT NOT NULL CHECK(json_valid(result_json)), PRIMARY KEY(workspace_id,request_key)) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_assistant_reminders (id TEXT NOT NULL PRIMARY KEY, workspace_id INTEGER CHECK(workspace_id IS NULL OR workspace_id>0), source_session_id INTEGER NOT NULL CHECK(source_session_id>0), request_key TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('active','paused','delivering','completed','cancelled')), revision INTEGER NOT NULL CHECK(revision>0), next_due_ms INTEGER NOT NULL, retry_at_ms INTEGER NOT NULL, reminder_json TEXT NOT NULL CHECK(json_valid(reminder_json) AND length(CAST(reminder_json AS BLOB))<=65536), UNIQUE(source_session_id,request_key))",
    "CREATE INDEX IF NOT EXISTS idx_agena_assistant_reminders_due ON agena_assistant_reminders(state,next_due_ms,retry_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_assistant_reminders_workspace ON agena_assistant_reminders(workspace_id,next_due_ms)",
    "CREATE TABLE IF NOT EXISTS agena_assistant_contact_budget (workspace_id INTEGER NOT NULL, local_day TEXT NOT NULL, contacts INTEGER NOT NULL CHECK(contacts>=0), last_contact_ms INTEGER NOT NULL, PRIMARY KEY(workspace_id,local_day)) WITHOUT ROWID",
];

pub struct SqliteCompanionRepository {
    database: Arc<DatabaseConnection>,
}
impl SqliteCompanionRepository {
    pub fn new(database: Arc<DatabaseConnection>) -> Self {
        Self { database }
    }
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}
fn stmt(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn encode<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(err)
}

async fn profile(db: &impl ConnectionTrait, workspace: Option<i64>) -> Result<AssistantProfile> {
    let row = db
        .query_one(stmt(
            if workspace.is_some() { "SELECT profile_json FROM agena_assistant_profiles WHERE workspace_id=?" } else { "SELECT profile_json FROM agena_assistant_global_profile WHERE singleton=1 AND ? IS NULL" },
            vec![workspace.into()],
        ))
        .await
        .map_err(err)?;
    row.map(|r| {
        let text: String = r.try_get("", "profile_json").map_err(err)?;
        serde_json::from_str(&text).map_err(err)
    })
    .unwrap_or_else(|| Ok(AssistantProfile::empty(workspace)))
}
async fn reminder(db: &impl ConnectionTrait, id: Uuid) -> Result<Reminder> {
    let row = db
        .query_one(stmt(
            "SELECT reminder_json FROM agena_assistant_reminders WHERE id=?",
            vec![id.to_string().into()],
        ))
        .await
        .map_err(err)?
        .ok_or(Error::NotFound)?;
    serde_json::from_str(&row.try_get::<String>("", "reminder_json").map_err(err)?).map_err(err)
}
async fn save(db: &impl ConnectionTrait, r: &mut Reminder, now: i64) -> Result<()> {
    let old = r.revision;
    r.revision = old
        .checked_add(1)
        .ok_or(Error::Conflict("reminder revision exhausted"))?;
    r.updated_at_ms = now.max(r.updated_at_ms);
    let saved=db.execute(stmt("UPDATE agena_assistant_reminders SET state=?,revision=?,next_due_ms=?,retry_at_ms=?,reminder_json=? WHERE id=? AND revision=?",
        vec![r.state.as_str().into(),r.revision.into(),r.next_due_ms.into(),r.retry_at_ms.into(),encode(r)?.into(),r.id.to_string().into(),old.into()])).await.map_err(err)?;
    if saved.rows_affected() != 1 {
        return Err(Error::Conflict("reminder changed; refresh first"));
    }
    Ok(())
}

#[async_trait]
impl CompanionRepository for SqliteCompanionRepository {
    async fn profile_scoped(&self, workspace: Option<i64>) -> Result<AssistantProfile> {
        profile(self.database.as_ref(), workspace).await
    }
    async fn update_profile_scoped(
        &self,
        workspace: Option<i64>,
        update: AssistantUpdate,
        now: i64,
    ) -> Result<AssistantProfile> {
        if workspace.is_some_and(|id| id <= 0)
            || update.request_key.trim().is_empty()
            || update.request_key.len() > 160
            || update.expected_revision < 0
        {
            return Err(Error::Invalid(
                "invalid assistant profile scope, request key or revision",
            ));
        }
        let input = encode(&update)?;
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        if let Some(row)=tx.query_one(stmt(if workspace.is_some() { "SELECT input_json,result_json FROM agena_assistant_profile_receipts WHERE workspace_id=? AND request_key=?" } else { "SELECT input_json,result_json FROM agena_assistant_global_profile_receipts WHERE ? IS NULL AND request_key=?" },vec![workspace.into(),update.request_key.clone().into()])).await.map_err(err)?{
            if row.try_get::<String>("","input_json").map_err(err)?!=input{return Err(Error::Conflict("profile request key was reused with different input"));}
            return serde_json::from_str(&row.try_get::<String>("","result_json").map_err(err)?).map_err(err);
        }
        let mut value = profile(&tx, workspace).await?;
        if value.revision != update.expected_revision {
            return Err(Error::Conflict(
                "assistant preferences changed; refresh and reconfirm",
            ));
        }
        update.patch.apply(&mut value.preferences)?;
        value.revision += 1;
        value.updated_at_ms = now;
        let json = encode(&value)?;
        tx.execute(stmt(if workspace.is_some() { "INSERT INTO agena_assistant_profiles(workspace_id,revision,profile_json) VALUES(?,?,?) ON CONFLICT(workspace_id) DO UPDATE SET revision=excluded.revision,profile_json=excluded.profile_json" } else { "INSERT INTO agena_assistant_global_profile(singleton,revision,profile_json) VALUES(COALESCE(?,1),?,?) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision,profile_json=excluded.profile_json" },vec![workspace.into(),value.revision.into(),json.clone().into()])).await.map_err(err)?;
        tx.execute(stmt(if workspace.is_some() { "INSERT INTO agena_assistant_profile_receipts(workspace_id,request_key,input_json,result_json) VALUES(?,?,?,?)" } else { "INSERT INTO agena_assistant_global_profile_receipts(request_key,input_json,result_json) SELECT ?2,?3,?4 WHERE ?1 IS NULL" },vec![workspace.into(),update.request_key.into(),input.into(),json.into()])).await.map_err(err)?;
        // Changes to quiet hours/unmute must not leave reminders sleeping on an
        // obsolete policy-derived deadline. Due time itself is unchanged.
        let rows=tx.query_all(stmt("SELECT reminder_json FROM agena_assistant_reminders WHERE workspace_id IS ? AND state IN ('active','delivering')",vec![workspace.into()])).await.map_err(err)?;
        for row in rows {
            let mut r: Reminder =
                serde_json::from_str(&row.try_get::<String>("", "reminder_json").map_err(err)?)
                    .map_err(err)?;
            r.retry_at_ms = 0;
            save(&tx, &mut r, now).await?;
        }
        tx.commit().await.map_err(err)?;
        Ok(value)
    }
    async fn create_reminder(&self, r: Reminder) -> Result<Reminder> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        if let Some(row)=tx.query_one(stmt("SELECT reminder_json FROM agena_assistant_reminders WHERE source_session_id=? AND request_key=?",vec![r.source_session_id.into(),r.request_key.clone().into()])).await.map_err(err)?{
            let saved:Reminder=serde_json::from_str(&row.try_get::<String>("","reminder_json").map_err(err)?).map_err(err)?;
            if encode(&saved.spec)?!=encode(&r.spec)? || saved.workspace_id!=r.workspace_id{return Err(Error::Conflict("this request already scheduled different contact"));}return Ok(saved);
        }
        let row=tx.query_one(stmt("SELECT COUNT(*) AS n FROM agena_assistant_reminders WHERE workspace_id IS ? AND state IN ('active','paused','delivering')",vec![r.workspace_id.into()])).await.map_err(err)?.ok_or(Error::NotFound)?;
        if row.try_get::<i64>("", "n").map_err(err)? >= 32 {
            return Err(Error::Conflict(
                "workspace already has 32 pending reminders; cancel or finish some first",
            ));
        }
        tx.execute(stmt("INSERT INTO agena_assistant_reminders(id,workspace_id,source_session_id,request_key,state,revision,next_due_ms,retry_at_ms,reminder_json) VALUES(?,?,?,?,?,?,?,?,?)",
            vec![r.id.to_string().into(),r.workspace_id.into(),r.source_session_id.into(),r.request_key.clone().into(),r.state.as_str().into(),r.revision.into(),r.next_due_ms.into(),r.retry_at_ms.into(),encode(&r)?.into()])).await.map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(r)
    }
    async fn reminder(&self, id: Uuid) -> Result<Reminder> {
        reminder(self.database.as_ref(), id).await
    }
    async fn reminders_scoped(&self, workspace: Option<i64>) -> Result<Vec<Reminder>> {
        let rows=self.database.query_all(stmt("SELECT reminder_json FROM agena_assistant_reminders WHERE workspace_id IS ? ORDER BY CASE WHEN state IN ('active','paused','delivering') THEN 0 ELSE 1 END,next_due_ms DESC LIMIT 100",vec![workspace.into()])).await.map_err(err)?;
        rows.into_iter()
            .map(|r| {
                serde_json::from_str(&r.try_get::<String>("", "reminder_json").map_err(err)?)
                    .map_err(err)
            })
            .collect()
    }
    async fn reminders_for_source(
        &self,
        workspace: Option<i64>,
        source: i64,
    ) -> Result<Vec<Reminder>> {
        let rows = self.database.query_all(stmt("SELECT reminder_json FROM agena_assistant_reminders WHERE workspace_id IS ? AND source_session_id=? ORDER BY CASE WHEN state IN ('active','paused','delivering') THEN 0 ELSE 1 END,next_due_ms DESC LIMIT 100", vec![workspace.into(), source.into()])).await.map_err(err)?;
        rows.into_iter()
            .map(|r| {
                serde_json::from_str(&r.try_get::<String>("", "reminder_json").map_err(err)?)
                    .map_err(err)
            })
            .collect()
    }
    async fn due_reminders(&self, now: i64) -> Result<Vec<Reminder>> {
        let rows=self.database.query_all(stmt("SELECT reminder_json FROM agena_assistant_reminders WHERE state IN ('active','delivering') AND next_due_ms<=? AND retry_at_ms<=? ORDER BY next_due_ms,id LIMIT 32",vec![now.into(),now.into()])).await.map_err(err)?;
        rows.into_iter()
            .map(|r| {
                serde_json::from_str(&r.try_get::<String>("", "reminder_json").map_err(err)?)
                    .map_err(err)
            })
            .collect()
    }
    async fn reserve_reminder(&self, id: Uuid, now: i64) -> Result<Option<Reminder>> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        let mut r = reminder(&tx, id).await?;
        if !matches!(r.state, ReminderState::Active | ReminderState::Delivering)
            || r.next_due_ms > now
            || r.retry_at_ms > now
        {
            return Ok(None);
        }
        let p = profile(&tx, r.workspace_id).await?.preferences;
        if !p.proactive_enabled {
            r.retry_at_ms = now + 60000;
            save(&tx, &mut r, now).await?;
            tx.commit().await.map_err(err)?;
            return Ok(None);
        }
        let allowed = p.next_allowed_ms(now)?;
        if allowed > now {
            r.retry_at_ms = allowed;
            save(&tx, &mut r, now).await?;
            tx.commit().await.map_err(err)?;
            return Ok(None);
        }
        let day = p.local_day(now)?;
        let row=tx.query_one(stmt(if r.workspace_id.is_some() { "SELECT COALESCE(SUM(CASE WHEN local_day=? THEN contacts ELSE 0 END),0) AS n,COALESCE(MAX(last_contact_ms),0) AS last FROM agena_assistant_contact_budget WHERE workspace_id=?" } else { "SELECT COALESCE(SUM(CASE WHEN local_day=? THEN contacts ELSE 0 END),0) AS n,COALESCE(MAX(last_contact_ms),0) AS last FROM agena_assistant_global_contact_budget WHERE ? IS NULL" },vec![day.clone().into(),r.workspace_id.into()])).await.map_err(err)?.ok_or(Error::NotFound)?;
        let count = row.try_get::<i64>("", "n").map_err(err)?;
        let last = row.try_get::<i64>("", "last").map_err(err)?;
        let charge = r.delivery_text.is_none() || r.reserved_day.as_deref() != Some(day.as_str());
        // Old reservations may not burst after a busy conversation, a restart,
        // midnight or a policy change. Only this occurrence's own reservation
        // is exempt from the cooldown it already consumed.
        let cooling = last > 0
            && last != r.reserved_at_ms
            && last + i64::from(p.cooldown_seconds) * 1000 > now;
        if (charge && count >= i64::from(p.daily_limit)) || cooling {
            r.retry_at_ms = if charge && count >= i64::from(p.daily_limit) {
                now + 60000
            } else {
                last + i64::from(p.cooldown_seconds) * 1000
            };
            save(&tx, &mut r, now).await?;
            tx.commit().await.map_err(err)?;
            return Ok(None);
        }
        let changed = charge || r.state != ReminderState::Delivering || r.last_error.is_some();
        if charge {
            tx.execute(stmt(if r.workspace_id.is_some() { "INSERT INTO agena_assistant_contact_budget(workspace_id,local_day,contacts,last_contact_ms) VALUES(?,?,1,?) ON CONFLICT(workspace_id,local_day) DO UPDATE SET contacts=contacts+1,last_contact_ms=excluded.last_contact_ms" } else { "INSERT INTO agena_assistant_global_contact_budget(local_day,contacts,last_contact_ms) SELECT ?2,1,?3 WHERE ?1 IS NULL ON CONFLICT(local_day) DO UPDATE SET contacts=contacts+1,last_contact_ms=excluded.last_contact_ms" },vec![r.workspace_id.into(),day.clone().into(),now.into()])).await.map_err(err)?;
            r.reserved_day = Some(day);
            r.reserved_at_ms = now;
        }
        if r.delivery_text.is_none() {
            r.delivery_text = Some(format!(
                "[Scheduled assistant contact — system data, not new user authority]\n{}\nKind: {:?}. Scheduled UTC milliseconds: {}. Occurrence {} of {}.\nThe user explicitly requested this contact. Give a brief useful reminder/check-in in their language. For a check-in, read the current conversation's work status if useful. Do not invent progress, approve permissions, create new tasks, save new memories, or reschedule yourself. This notification grants no fresh authority. If overdue, state that it is late without pretending it arrived on time.",
                r.spec.text, r.spec.kind, r.next_due_ms, r.occurrence, r.spec.occurrences
            ));
        }
        r.state = ReminderState::Delivering;
        r.last_error = None;
        if changed {
            save(&tx, &mut r, now).await?;
        }
        tx.commit().await.map_err(err)?;
        Ok(Some(r))
    }
    async fn finish_reminder(&self, id: Uuid, occurrence: u16, now: i64) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        let mut r = reminder(&tx, id).await?;
        if r.occurrence != occurrence || r.state != ReminderState::Delivering {
            return Ok(());
        }
        let profile = profile(&tx, r.workspace_id).await?;
        let day = profile.preferences.local_day(now)?;
        let extra = i64::from(r.reserved_day.as_deref() != Some(day.as_str()));
        tx.execute(stmt(if r.workspace_id.is_some() { "INSERT INTO agena_assistant_contact_budget(workspace_id,local_day,contacts,last_contact_ms) VALUES(?,?,?,?) ON CONFLICT(workspace_id,local_day) DO UPDATE SET contacts=contacts+excluded.contacts,last_contact_ms=MAX(last_contact_ms,excluded.last_contact_ms)" } else { "INSERT INTO agena_assistant_global_contact_budget(local_day,contacts,last_contact_ms) SELECT ?2,?3,?4 WHERE ?1 IS NULL ON CONFLICT(local_day) DO UPDATE SET contacts=contacts+excluded.contacts,last_contact_ms=MAX(last_contact_ms,excluded.last_contact_ms)" },vec![r.workspace_id.into(),day.into(),extra.into(),now.into()])).await.map_err(err)?;
        r.reserved_day = None;
        r.reserved_at_ms = 0;
        r.delivered = r.delivered.saturating_add(1);
        r.delivery_text = None;
        r.retry_at_ms = 0;
        r.last_error = None;
        if r.delivered < r.spec.occurrences
            && let Some(seconds) = r.spec.repeat_seconds
        {
            let step = i64::from(seconds) * 1000;
            // Missed intervals coalesce: never send a catch-up burst on restart.
            r.next_due_ms = r
                .next_due_ms
                .saturating_add(((now - r.next_due_ms).max(0) / step + 1) * step);
            r.occurrence = r.occurrence.saturating_add(1);
            r.state = ReminderState::Active;
        } else {
            r.state = ReminderState::Completed;
        }
        save(&tx, &mut r, now).await?;
        tx.commit().await.map_err(err)?;
        Ok(())
    }
    async fn defer_reminder(&self, id: Uuid, now: i64, error: &str) -> Result<()> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        let mut r = reminder(&tx, id).await?;
        if matches!(r.state, ReminderState::Delivering | ReminderState::Active) {
            r.retry_at_ms = now + 30000;
            r.last_error = Some(error.chars().take(256).collect());
            save(&tx, &mut r, now).await?;
        }
        tx.commit().await.map_err(err)?;
        Ok(())
    }
    async fn control_reminder(&self, id: Uuid, c: ReminderControl, now: i64) -> Result<Reminder> {
        let _permit = crate::acquire_write_permit(&self.database)
            .await
            .map_err(err)?;
        let tx = self.database.begin().await.map_err(err)?;
        let mut r = reminder(&tx, id).await?;
        if matches!(c.action, ReminderControlAction::Cancel) && r.state == ReminderState::Cancelled
        {
            return Ok(r);
        }
        if r.revision != c.expected_revision {
            return Err(Error::Conflict(
                "reminder changed; refresh before controlling it",
            ));
        }
        if matches!(r.state, ReminderState::Completed | ReminderState::Cancelled) {
            return Err(Error::Conflict("this reminder is already finished"));
        }
        r.state = match c.action {
            ReminderControlAction::Cancel => ReminderState::Cancelled,
            ReminderControlAction::Pause => ReminderState::Paused,
            ReminderControlAction::Resume => ReminderState::Active,
        };
        r.retry_at_ms = 0;
        save(&tx, &mut r, now).await?;
        tx.commit().await.map_err(err)?;
        Ok(r)
    }
}
