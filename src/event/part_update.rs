use serde::{Deserialize, Serialize};

use crate::event::{CallId, ItemId, MessageId, PartId, ThreadId, TurnId};
use crate::message::SessionMessagePart;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum PartDeltaField {
    Text,
    ReasoningSummary,
    ReasoningRawContent,
    CommandStdout,
    CommandStderr,
    ToolOutputText,
    Custom { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePartUpdatedEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub part: SessionMessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartDeltaEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub part_id: PartId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<ItemId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
    pub field: PartDeltaField,
    pub delta: String,
    pub seq: u64,
    pub ts_ms: i64,
}
