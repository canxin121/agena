use agena_domain::{ExecutionId, ExecutionStatus, Role, RunId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::message::{MessageMetadata, MessagePart};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePartCheckpointedEvent {
    pub session_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<ExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<agena_domain::TurnId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<agena_domain::AssistantReplyId>,
    pub message_id: i64,
    pub message_role: Role,
    pub message_state: ExecutionStatus,
    pub message_created_at: DateTime<Utc>,
    pub message_metadata: MessageMetadata,
    pub part: MessagePart,
    pub ts_ms: i64,
}

/// Ephemeral full-node update used by live transcript consumers.
///
/// Streaming publishes the current typed part instead of a field-specific
/// string delta. Consumers therefore reduce the same Text/Activity value that
/// durable snapshots expose and never need a second content taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptPartUpsertedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub turn_id: agena_domain::TurnId,
    pub reply_id: agena_domain::AssistantReplyId,
    pub message_role: Role,
    pub part: MessagePart,
    pub ts_ms: i64,
}

// NOTE: the wrapper enum `SessionEvent` has been removed in favor of the
// unified `crate::event::EventKind`. The payload structs above are still the
// canonical definitions — they are referenced verbatim by the corresponding
// `EventKind` variants.
