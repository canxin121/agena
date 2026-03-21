use serde::{Deserialize, Serialize};

use crate::event::{CallId, ItemId, MessageId, PartId, ThreadId, TurnId};
use crate::message::{ExecutionStatus, PartKind, SessionMessagePart};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemRef {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_id: ItemId,
    pub message_id: MessageId,
    pub part_id: PartId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
    pub kind: PartKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemSnapshot {
    #[serde(flatten)]
    pub item: ItemRef,
    pub status: ExecutionStatus,
    pub part: SessionMessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemStartedEvent {
    #[serde(flatten)]
    pub snapshot: ItemSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemUpdatedEvent {
    #[serde(flatten)]
    pub snapshot: ItemSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemCompletedEvent {
    #[serde(flatten)]
    pub snapshot: ItemSnapshot,
}
