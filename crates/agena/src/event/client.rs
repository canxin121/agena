use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    message::{ExecutionStatus, MessagePart, MessageStatus},
    permission::{DecisionTraceStep, PermissionAction, PermissionReplyKind, PermissionRiskLevel},
    role::Role,
};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionStartedEvent {
    pub session_id: i64,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionFailedEvent {
    pub session_id: i64,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamErrorEvent {
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
    pub session_id: i64,
    pub message_id: i64,
    pub message_role: Role,
    pub message_state: MessageStatus,
    pub message_created_at: DateTime<Utc>,
    pub part: MessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartDeltaEvent {
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PermissionRequestedEvent {
    pub session_id: i64,
    pub request_id: String,
    pub action: PermissionAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_actions: Vec<PermissionAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_actions: Vec<PermissionAction>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default)]
    pub risk: PermissionRiskLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<DecisionTraceStep>,
    pub ts_ms: i64,
}

#[derive(Debug, Deserialize)]
struct PermissionRequestedEventWire {
    session_id: i64,
    request_id: String,
    #[serde(default)]
    action: Option<PermissionAction>,
    #[serde(default)]
    related_actions: Vec<PermissionAction>,
    #[serde(default)]
    requested_actions: Vec<PermissionAction>,
    reason: String,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    operator: Option<String>,
    #[serde(default)]
    risk: PermissionRiskLevel,
    #[serde(default)]
    trace: Vec<DecisionTraceStep>,
    ts_ms: i64,
}

impl<'de> Deserialize<'de> for PermissionRequestedEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PermissionRequestedEventWire::deserialize(deserializer)?;
        let action = wire
            .action
            .or_else(|| wire.requested_actions.first().cloned())
            .or_else(|| wire.related_actions.first().cloned())
            .ok_or_else(|| serde::de::Error::missing_field("action"))?;

        Ok(Self {
            session_id: wire.session_id,
            request_id: wire.request_id,
            action,
            related_actions: wire.related_actions,
            requested_actions: wire.requested_actions,
            reason: wire.reason,
            explanation: wire.explanation,
            source: wire.source,
            scope: wire.scope,
            operator: wire.operator,
            risk: wire.risk,
            trace: wire.trace,
            ts_ms: wire.ts_ms,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRepliedEvent {
    pub session_id: i64,
    pub request_id: String,
    pub kind: PermissionReplyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRuleEvent {
    pub session_id: Option<i64>,
    pub rule_id: i64,
    pub action_key: String,
    pub mode: String,
    pub scope: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
    pub ts_ms: i64,
}

// NOTE: the wrapper enum `SessionEvent` has been removed in favor of the
// unified `crate::event::EventKind`. The payload structs above are still the
// canonical definitions — they are referenced verbatim by the corresponding
// `EventKind` variants.

#[cfg(test)]
mod tests {
    use super::PermissionRequestedEvent;
    use crate::permission::PermissionAction;
    use serde_json::json;

    #[test]
    fn permission_requested_event_deserializes_legacy_payload_without_action() {
        let payload = json!({
            "session_id": 1,
            "request_id": "call_legacy",
            "related_actions": [
                {
                    "kind": "tool",
                    "tool_name": "agena.workflow/plan"
                }
            ],
            "requested_actions": [
                {
                    "kind": "tool",
                    "tool_name": "agena.workflow/plan"
                }
            ],
            "reason": "tool requires confirmation",
            "explanation": "legacy payload omitted top-level action",
            "source": "static_policy",
            "risk": "medium",
            "trace": [],
            "ts_ms": 123
        });

        let event: PermissionRequestedEvent =
            serde_json::from_value(payload).expect("deserialize legacy permission request");

        assert_eq!(
            event.action,
            PermissionAction::Tool {
                tool_name: "agena.workflow/plan".to_string(),
                qualifier: None,
            }
        );
        assert_eq!(event.requested_actions.len(), 1);
        assert_eq!(event.related_actions.len(), 1);
    }
}
