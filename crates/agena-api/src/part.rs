//! Typed public projections for message-part detail.
//!
//! These are deliberately protocol-owned values. They describe what a client
//! can render, rather than exposing the session runtime's message-part
//! implementation or its persistence representations.

use serde::{Deserialize, Serialize};

use crate::resource::{PartAttachment, PartSkillReference};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Stable wire header for one message part. Detail is represented by the
/// content resource once every runtime content variant has an explicit API
/// projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartResource {
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: PartExecutionStatusResource,
    pub kind: PartKindResource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_detail: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<agena_domain::ActivityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<agena_domain::TextSegmentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<PartDetailResource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a message part; pairs with [`agena_domain::PartKind`] and drives how the part is rendered.
pub enum PartKindResource {
    Text,
    Activity,
}

/// Execution state for a message part, operation, or interactive request.
/// This is intentionally distinct from the containing message's state even
/// though their current wire values overlap.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartExecutionStatusResource {
    #[default]
    Pending,
    InProgress,
    Completed,
    PolicyDenied,
    UserDeclined,
    CapabilityUnavailable,
    ToolUnavailable,
    Failed,
    Cancelled,
}

/// Detail variants that are safe to expose independently of a runtime
/// implementation. Additional variants are added alongside their complete,
/// typed request and tool-result contracts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartDetailResource {
    Text(TextPartResource),
    Reasoning(ReasoningPartResource),
    Attachment(AttachmentPartResource),
    SkillReference(SkillReferencePartResource),
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
/// A message part that references skills used by the run.
pub struct SkillReferencePartResource {
    pub skills: Vec<PartSkillReference>,
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
/// and is kept beside those facts rather than flattened into a second result
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
    #[serde(default)]
    pub state: agena_domain::ToolResultState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<agena_domain::OperationError>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub lifecycle: agena_domain::TimeRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<crate::live::ToolHumanPresentationResource>,
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
                presentation: None,
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
