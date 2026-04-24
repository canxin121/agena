use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr};

use crate::{
    db::crud::{message, session_history},
    message::Message,
    session::SessionRuntimeState,
};

use super::{
    HistoryItem, HistoryRecord, SessionHistoryProjection, history_items_from_legacy_snapshot,
    replay_history,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionHistoryStore {
    db: DatabaseConnection,
}

impl SessionHistoryStore {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub(crate) async fn load_projection(
        &self,
        session_id: i64,
        base_runtime: SessionRuntimeState,
    ) -> Result<SessionHistoryProjection, DbErr> {
        self.ensure_legacy_imported(session_id).await?;
        let records = session_history::list_history_records(&self.db, session_id).await?;
        let mut projection = replay_history(records.as_slice())
            .map_err(|err| DbErr::Custom(format!("failed to replay session history: {err}")))?;
        if projection.runtime == SessionRuntimeState::default() {
            projection.runtime = base_runtime;
        }
        Ok(projection)
    }

    async fn ensure_legacy_imported(&self, session_id: i64) -> Result<(), DbErr> {
        ensure_legacy_imported(&self.db, session_id).await
    }
}

pub(crate) async fn ensure_legacy_imported<C>(db: &C, session_id: i64) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if session_history::latest_history_seq(db, session_id).await?.is_some() {
        return Ok(());
    }

    let messages = message::list_messages_with_parts(db, session_id).await?;
    if messages.is_empty() {
        return Ok(());
    }

    append_items(
        db,
        session_id,
        0,
        history_items_from_legacy_snapshot(messages.as_slice()),
        Utc::now(),
    )
    .await?;
    Ok(())
}

pub(crate) async fn append_items<C>(
    db: &C,
    session_id: i64,
    next_seq_start: i64,
    items: Vec<HistoryItem>,
    now: DateTime<Utc>,
) -> Result<Vec<HistoryRecord>, DbErr>
where
    C: ConnectionTrait,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    session_history::append_history_items(db, session_id, next_seq_start, items, now).await
}

pub(crate) async fn append_message_snapshot<C>(
    db: &C,
    session_id: i64,
    next_seq_start: i64,
    message: &Message,
    now: DateTime<Utc>,
) -> Result<Vec<HistoryRecord>, DbErr>
where
    C: ConnectionTrait,
{
    append_items(
        db,
        session_id,
        next_seq_start,
        super::history_items_from_message_snapshot(message),
        now,
    )
    .await
}
