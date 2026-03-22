use serde::{Deserialize, Serialize};

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
    pub thread_id: i64,
    pub message_id: i64,
    pub part: SessionMessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartDeltaEvent {
    pub thread_id: i64,
    pub message_id: i64,
    pub part_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    pub field: PartDeltaField,
    pub delta: String,
    pub seq: u64,
    pub ts_ms: i64,
}
