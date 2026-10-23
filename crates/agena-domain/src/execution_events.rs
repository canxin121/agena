use serde::{Deserialize, Serialize};

use crate::{AssistantReplyId, ExecutionId, ExecutionOutcome, ExecutionSource, TurnId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Event emitted when an execution starts.
pub struct ExecutionStartedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub turn_id: TurnId,
    pub reply_id: AssistantReplyId,
    pub source: ExecutionSource,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Event emitted when an execution finishes.
pub struct ExecutionFinishedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub reply_id: AssistantReplyId,
    pub outcome: ExecutionOutcome,
    pub ts_ms: i64,
}
