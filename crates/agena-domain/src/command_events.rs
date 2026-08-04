use serde::{Deserialize, Serialize};

use crate::ExecutionStatus;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandContext {
    pub session_id: i64,
    pub call_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<i64>,
    /// The tool Activity this command belongs to, when known. Lets live
    /// presentation consumers route streaming output deltas to the correct
    /// expanded Activity without a DB round trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::{CommandContext, CommandOutputDeltaEvent, CommandOutputStream};

    #[test]
    fn command_delta_flattens_context_and_omits_empty_preview() {
        let value = CommandOutputDeltaEvent {
            context: CommandContext {
                session_id: 1,
                call_id: 2,
                message_id: None,
                part_id: None,
                activity_id: None,
            },
            stream: CommandOutputStream::Stdout,
            seq: 3,
            ts_ms: 4,
            chunk: b"ok".to_vec(),
            preview_text: String::new(),
            preview_lossy: false,
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["session_id"], 1);
        assert!(json.get("context").is_none());
        assert!(json.get("preview_text").is_none());
    }
}
