//! Typed public projections for message-part detail.
//!
//! These are deliberately protocol-owned values. They describe what a client
//! can render, rather than exposing the session runtime's message-part
//! implementation or its persistence representations.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::resource::{PartAttachment, PartCommandReference};

fn is_false(value: &bool) -> bool {
    !*value
}

/// One persisted part as exposed by the public API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PartResource {
    pub part_id: i64,
    pub kind: String,
    pub role: String,
    pub state: String,
    pub content: Value,
    /// An omitted section is unloaded. A listed section may have an empty or
    /// null value; its revision identifies the exact facts it was derived from.
    pub sections: Vec<LoadedPartSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<agena_domain::PartDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_part_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
    pub origin_session_id: i64,
    pub revision: i64,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// 1-based ordinal of this part among the session's user-send messages in
    /// durable `(created_at_ms, part_id)` order. Present only on user-send run
    /// markers. Derived from the durable order rather than a stored counter, so
    /// it is always contiguous (`1..=user_message_count`) and never drifts
    /// after rewind, fork, compaction, import, or withdrawal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message_ordinal: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedPartSection {
    pub section: crate::live::ToolDetailSection,
    pub revision: i64,
}

/// Execution state for a message part, operation, or interactive request.
///
/// The canonical definition is [`agena_domain::ExecutionStatus`]; the API
/// re-exports it so wire clients and every server layer share one enum.
pub use agena_domain::ExecutionStatus as PartExecutionStatusResource;

/// Detail variants that are safe to expose independently of a runtime
/// implementation. Additional variants are added alongside their complete,
/// typed request and tool-result contracts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartDetailResource {
    Text(TextPartResource),
    Reasoning(ReasoningPartResource),
    Attachment(AttachmentPartResource),
    CommandReference(CommandReferencePartResource),
    Error(ErrorPartResource),
    ToolCall(Box<ToolCallPartResource>),
    Hook(HookPartResource),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plain text part of a message.
pub struct TextPartResource {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Model reasoning text attached to a message part.
pub struct ReasoningPartResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_content: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

impl ReasoningPartResource {
    pub fn summary_text(&self) -> String {
        self.summary.concat()
    }
    pub fn raw_text(&self) -> String {
        self.raw_content.concat()
    }
    pub fn preferred_text(&self) -> String {
        if self.summary.is_empty() {
            self.raw_text()
        } else {
            self.summary_text()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// A message part carrying [`PartAttachment`]s.
pub struct AttachmentPartResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PartAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A message part that references the commands selected for the run.
pub struct CommandReferencePartResource {
    pub commands: Vec<PartCommandReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A message part representing a failure.
pub struct ErrorPartResource {
    pub problem: agena_failure::UserProblem,
}

/// One observed plugin hook run recorded as a first-class transcript part.
/// Hook activity (for example the workflow plan's `agent.stop` autorun
/// continuation) rides the same activity pipeline as tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookPartResource {
    /// The hook identifier that ran, for example `agent.stop`.
    pub hook: String,
    /// The plugin that ran the hook, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Short human-facing summary of the hook outcome.
    pub summary: String,
    /// Optional human-facing detail rendered when the activity is expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Optional message the hook sent to keep the run going (for example the
    /// workflow plan autorun's continuation). Carried by the hook activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The current public read-time view of one `tool_call` part.
///
/// The durable facts are the same fields as the canonical runtime contract:
/// invocation, lifecycle, state, error, metadata, and one optional
/// [`agena_domain::RawOutput`]. Human presentation is explicitly ephemeral
/// and is kept on the part header rather than flattened into a second result
/// envelope. AI output is not represented here; it is projected from
/// `output` when needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallPartResource {
    pub call_id: i64,
    pub invocation: agena_domain::ToolInvocation,
    #[serde(
        default,
        skip_serializing_if = "agena_domain::OperationAuthorization::is_empty"
    )]
    pub authorization: agena_domain::OperationAuthorization,
    #[serde(
        default,
        skip_serializing_if = "agena_domain::OperationUserInput::is_empty"
    )]
    pub user_input: agena_domain::OperationUserInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<agena_domain::RawOutput>,
    pub resources: Vec<agena_domain::ContentRef>,
    #[serde(default)]
    pub state: agena_domain::ToolResultState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<agena_domain::OperationError>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub lifecycle: agena_domain::TimeRange,
}

#[cfg(test)]
mod tests {
    use super::{PartDetailResource, ReasoningPartResource, TextPartResource};

    #[test]
    fn message_details_are_explicitly_tagged() {
        assert_eq!(
            serde_json::to_value(PartDetailResource::Text(TextPartResource {
                text: "hello".to_owned(),
                synthetic: false,
            }))
            .expect("serialize message detail"),
            serde_json::json!({"type": "text", "text": "hello"})
        );
    }

    #[test]
    fn canonical_tool_calls_do_not_contain_projection_copies() {
        let value = serde_json::to_value(PartDetailResource::ToolCall(Box::new(
            super::ToolCallPartResource {
                resources: Vec::new(),
                call_id: 7,
                invocation: agena_domain::ToolInvocation::new(
                    "fs.read",
                    agena_domain::StructuredObject::default(),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(agena_domain::RawOutput::text("done")),
                state: agena_domain::ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: Default::default(),
            },
        )))
        .expect("serialize canonical tool call");
        assert!(value.get("model_output").is_none());
        assert!(value.get("result").is_none());
        assert!(value.get("blocks").is_none());
        assert!(value.get("output").is_some());
    }

    #[test]
    fn part_helpers_remain_protocol_owned() {
        let reasoning = ReasoningPartResource {
            summary: vec!["thinking ".to_owned(), "continues".to_owned()],
            raw_content: vec!["raw".to_owned()],
            encrypted_content: None,
        };
        assert_eq!(reasoning.preferred_text(), "thinking continues");
    }
}
