use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::AgentEvent;
use crate::message::{PartContent, PartKind, StructuredObject};
use crate::role::Role;
use crate::session::status::{
    ItemStatus, ItemStatusTransitionError, SessionStatus, SessionStatusTransitionError, TurnStatus,
    TurnStatusTransitionError,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub project_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub title: String,
    pub status: SessionStatus,
    pub last_event_seq: i64,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(id: i64, title: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            id,
            parent_id: None,
            project_id: None,
            workspace_id: None,
            title: title.into(),
            status: SessionStatus::Active,
            last_event_seq: 0,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn transition_status(
        &mut self,
        next: SessionStatus,
        now: DateTime<Utc>,
    ) -> Result<(), SessionStatusTransitionError> {
        let from = self.status;
        if !from.can_transition(next) {
            return Err(SessionStatusTransitionError { from, to: next });
        }
        self.status = next;
        self.version += 1;
        self.updated_at = now;
        Ok(())
    }

    pub fn advance_event_seq(&mut self, seq: i64, now: DateTime<Utc>) {
        if seq > self.last_event_seq {
            self.last_event_seq = seq;
            self.version += 1;
            self.updated_at = now;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTurn {
    pub id: i64,
    pub session_id: i64,
    pub turn_index: i32,
    pub status: TurnStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<TurnError>,
    pub version: i64,
}

impl SessionTurn {
    pub fn new(id: i64, session_id: i64, turn_index: i32, started_at: DateTime<Utc>) -> Self {
        Self {
            id,
            session_id,
            turn_index,
            status: TurnStatus::Running,
            started_at,
            ended_at: None,
            error: None,
            version: 1,
        }
    }

    pub fn complete(&mut self, ended_at: DateTime<Utc>) -> Result<(), TurnStatusTransitionError> {
        self.transition(TurnStatus::Completed, ended_at)
    }

    pub fn fail(
        &mut self,
        error: TurnError,
        ended_at: DateTime<Utc>,
    ) -> Result<(), TurnStatusTransitionError> {
        self.error = Some(error);
        self.transition(TurnStatus::Failed, ended_at)
    }

    pub fn abort(&mut self, ended_at: DateTime<Utc>) -> Result<(), TurnStatusTransitionError> {
        self.transition(TurnStatus::Aborted, ended_at)
    }

    fn transition(
        &mut self,
        next: TurnStatus,
        ended_at: DateTime<Utc>,
    ) -> Result<(), TurnStatusTransitionError> {
        let from = self.status;
        if !from.can_transition(next) {
            return Err(TurnStatusTransitionError { from, to: next });
        }
        self.status = next;
        self.ended_at = Some(ended_at);
        self.version += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionItemKind {
    Text,
    Reasoning,
    ToolExecution,
    CommandExecution,
    FileChange,
    WebSearch,
    TodoList,
    Error,
    Custom,
}

impl From<PartKind> for SessionItemKind {
    fn from(value: PartKind) -> Self {
        match value {
            PartKind::Text => Self::Text,
            PartKind::Reasoning => Self::Reasoning,
            PartKind::ToolExecution => Self::ToolExecution,
            PartKind::CommandExecution => Self::CommandExecution,
            PartKind::FileChange => Self::FileChange,
            PartKind::WebSearch => Self::WebSearch,
            PartKind::TodoList => Self::TodoList,
            PartKind::Error => Self::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionItemPayload {
    Part {
        content: PartContent,
    },
    Custom {
        name: String,
        data: StructuredObject,
    },
}

impl SessionItemPayload {
    pub fn kind(&self) -> SessionItemKind {
        match self {
            Self::Part { content } => SessionItemKind::from(content.kind()),
            Self::Custom { .. } => SessionItemKind::Custom,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionItem {
    pub id: i64,
    pub session_id: i64,
    pub turn_id: i64,
    pub item_index: i32,
    pub role: Role,
    pub kind: SessionItemKind,
    pub status: ItemStatus,
    pub payload: SessionItemPayload,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionItem {
    pub fn new(
        id: i64,
        session_id: i64,
        turn_id: i64,
        item_index: i32,
        role: Role,
        payload: SessionItemPayload,
        now: DateTime<Utc>,
    ) -> Self {
        let kind = payload.kind();
        Self {
            id,
            session_id,
            turn_id,
            item_index,
            role,
            kind,
            status: ItemStatus::Started,
            payload,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn replace_payload(
        &mut self,
        payload: SessionItemPayload,
        now: DateTime<Utc>,
    ) -> Result<(), ItemStatusTransitionError> {
        self.transition(ItemStatus::Updated, now)?;
        self.kind = payload.kind();
        self.payload = payload;
        Ok(())
    }

    pub fn complete(&mut self, now: DateTime<Utc>) -> Result<(), ItemStatusTransitionError> {
        self.transition(ItemStatus::Completed, now)
    }

    pub fn fail(&mut self, now: DateTime<Utc>) -> Result<(), ItemStatusTransitionError> {
        self.transition(ItemStatus::Failed, now)
    }

    fn transition(
        &mut self,
        next: ItemStatus,
        now: DateTime<Utc>,
    ) -> Result<(), ItemStatusTransitionError> {
        let from = self.status;
        if !from.can_transition(next) {
            return Err(ItemStatusTransitionError { from, to: next });
        }
        self.status = next;
        self.version += 1;
        self.updated_at = now;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    ThreadStarted,
    TurnStarted,
    TurnCompleted,
    TurnFailed,
    ItemStarted,
    ItemUpdated,
    ItemCompleted,
    MessagePartUpdated,
    MessagePartDelta,
    CommandBegin,
    CommandOutputDelta,
    CommandEnd,
    StreamError,
}

impl From<&AgentEvent> for SessionEventType {
    fn from(value: &AgentEvent) -> Self {
        match value {
            AgentEvent::ThreadStarted(_) => Self::ThreadStarted,
            AgentEvent::TurnStarted(_) => Self::TurnStarted,
            AgentEvent::TurnCompleted(_) => Self::TurnCompleted,
            AgentEvent::TurnFailed(_) => Self::TurnFailed,
            AgentEvent::ItemStarted(_) => Self::ItemStarted,
            AgentEvent::ItemUpdated(_) => Self::ItemUpdated,
            AgentEvent::ItemCompleted(_) => Self::ItemCompleted,
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
    pub turn_id: Option<i64>,
    pub item_id: Option<i64>,
    pub seq: i64,
    pub event_type: SessionEventType,
    pub payload: AgentEvent,
    pub causation_id: Option<i64>,
    pub correlation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl SessionEventRecord {
    pub fn new(
        session_id: i64,
        seq: i64,
        payload: AgentEvent,
        turn_id: Option<i64>,
        item_id: Option<i64>,
        created_at: DateTime<Utc>,
    ) -> Self {
        let event_type = SessionEventType::from(&payload);
        Self {
            event_id: None,
            session_id,
            turn_id,
            item_id,
            seq,
            event_type,
            payload,
            causation_id: None,
            correlation_id: None,
            created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSnapshot {
    pub session: Session,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<SessionTurn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<SessionItem>,
    pub last_event_seq: i64,
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
