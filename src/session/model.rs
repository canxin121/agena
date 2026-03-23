use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

use crate::event::AgentEvent;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(id: i64, workspace_id: i64, title: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            id,
            parent_id: None,
            workspace_id,
            title: title.into(),
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
    EnumIter,
    DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum SessionEventType {
    #[sea_orm(num_value = 1)]
    ThreadStarted,
    #[sea_orm(num_value = 2)]
    ThreadFailed,
    #[sea_orm(num_value = 3)]
    MessagePartUpdated,
    #[sea_orm(num_value = 4)]
    MessagePartDelta,
    #[sea_orm(num_value = 5)]
    CommandBegin,
    #[sea_orm(num_value = 6)]
    CommandOutputDelta,
    #[sea_orm(num_value = 7)]
    CommandEnd,
    #[sea_orm(num_value = 8)]
    StreamError,
}

impl From<&AgentEvent> for SessionEventType {
    fn from(value: &AgentEvent) -> Self {
        match value {
            AgentEvent::ThreadStarted(_) => Self::ThreadStarted,
            AgentEvent::ThreadFailed(_) => Self::ThreadFailed,
            AgentEvent::MessagePartUpdated(_) => Self::MessagePartUpdated,
            AgentEvent::MessagePartDelta(_) => Self::MessagePartDelta,
            AgentEvent::CommandBegin(_) => Self::CommandBegin,
            AgentEvent::CommandOutputDelta(_) => Self::CommandOutputDelta,
            AgentEvent::CommandEnd(_) => Self::CommandEnd,
            AgentEvent::StreamError(_) => Self::StreamError,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEventRecord {
    pub event_id: Option<i64>,
    pub session_id: i64,
    pub seq: i64,
    pub event_type: SessionEventType,
    pub payload: AgentEvent,
    pub causation_id: Option<i64>,
    pub correlation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl SessionEventRecord {
    pub fn new(session_id: i64, seq: i64, payload: AgentEvent, created_at: DateTime<Utc>) -> Self {
        let event_type = SessionEventType::from(&payload);
        Self {
            event_id: None,
            session_id,
            seq,
            event_type,
            payload,
            causation_id: None,
            correlation_id: None,
            created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct SessionSnapshot {
    pub session: Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionCheckpoint {
    pub id: i64,
    pub session_id: i64,
    pub upto_seq: i64,
    pub snapshot: SessionSnapshot,
    pub state_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}
