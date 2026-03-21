use serde::{Deserialize, Serialize};

use crate::event::{ThreadId, TurnId};
use crate::message::MessageUsage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadStartedEvent {
    pub thread_id: ThreadId,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnStartedEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnCompletedEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub usage: MessageUsage,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnFailedEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamErrorEvent {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}
