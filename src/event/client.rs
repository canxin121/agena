use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::message::{ExecutionStatus, MessagePart};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunStartedEvent {
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunFailedEvent {
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamErrorEvent {
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandContext {
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub call_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandBeginEvent {
    #[serde(flatten)]
    pub context: CommandContext,
    pub command: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_user_initiated: bool,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOutputDeltaEvent {
    #[serde(flatten)]
    pub context: CommandContext,
    pub stream: CommandOutputStream,
    pub seq: u64,
    pub ts_ms: i64,
    pub chunk: Vec<u8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview_text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub preview_lossy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandEndEvent {
    #[serde(flatten)]
    pub context: CommandContext,
    pub status: ExecutionStatus,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aggregated_output: String,
    pub ts_ms: i64,
}

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
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub message_id: i64,
    pub part: MessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartDeltaEvent {
    #[serde(alias = "thread_id")]
    pub session_id: i64,
    pub message_id: i64,
    pub part_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    pub field: PartDeltaField,
    pub delta: String,
    pub seq: u64,
    pub ts_ms: i64,
}

/// Frontend-facing session event protocol.
///
/// The backend must not depend on this enum to drive core state transitions.
/// State is updated directly in storage and memory; these events are the client
/// projection of committed session changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    #[serde(alias = "thread_started")]
    RunStarted(RunStartedEvent),
    #[serde(alias = "thread_failed")]
    RunFailed(RunFailedEvent),
    MessagePartUpdated(MessagePartUpdatedEvent),
    MessagePartDelta(MessagePartDeltaEvent),
    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),
    StreamError(StreamErrorEvent),
}

#[cfg(test)]
mod tests {
    use super::SessionEvent;

    #[test]
    fn deserializes_legacy_thread_started_payload() {
        let payload = serde_json::json!({
            "event": "thread_started",
            "thread_id": 7,
            "ts_ms": 99
        });

        let event =
            serde_json::from_value::<SessionEvent>(payload).expect("legacy payload should parse");

        assert_eq!(
            event,
            SessionEvent::RunStarted(super::RunStartedEvent {
                session_id: 7,
                ts_ms: 99,
            })
        );
    }
}
