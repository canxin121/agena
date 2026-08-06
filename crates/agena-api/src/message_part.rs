//! Typed public projections for message-part detail.
//!
//! These are deliberately protocol-owned values. They describe what a client
//! can render, rather than exposing the session runtime's message-part
//! implementation or its persistence representations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::resource::{MessageAttachment, MessageSkillReference, UserInputReply, UserInputRequest};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Stable wire header for one message part. Detail is represented by the
/// content resource once every runtime content variant has an explicit API
/// projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePartResource {
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: PartExecutionStatusResource,
    pub kind: MessagePartKindResource,
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
    pub content: Option<MessagePartDetailResource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessagePartKindResource {
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
pub enum MessagePartDetailResource {
    Text(MessageTextPartResource),
    Reasoning(MessageReasoningPartResource),
    Attachment(MessageAttachmentPartResource),
    SkillReference(MessageSkillReferencePartResource),
    Error(MessageErrorPartResource),
    Operation(Box<OperationPartResource>),
    Request(Box<MessageRequestPartResource>),
    Hook(MessageHookPartResource),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageTextPartResource {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageReasoningPartResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_content: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

impl MessageReasoningPartResource {
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
pub struct MessageAttachmentPartResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageSkillReferencePartResource {
    pub skills: Vec<MessageSkillReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageErrorPartResource {
    pub problem: agena_failure::UserProblem,
}

/// One observed plugin hook run recorded as a first-class transcript part.
/// Hook activity (for example the workflow plan's `agent.stop` autorun
/// continuation) rides the same activity pipeline as tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageHookPartResource {
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
}

/// An interactive request recorded in the conversation. The concrete request
/// and reply types are shared with the public command/query protocol rather
/// than re-declared for message history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "request_type", rename_all = "snake_case")]
pub enum MessageRequestPartResource {
    UserInput {
        request: UserInputRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<UserInputReply>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OperationPartResource {
    pub call_id: i64,
    pub invocation: ToolInvocationResource,
    #[serde(
        default,
        skip_serializing_if = "agena_domain::OperationAuthorization::is_empty"
    )]
    pub authorization: agena_domain::OperationAuthorization,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "ModelVisibleOutputResource::is_empty")]
    pub model_output: ModelVisibleOutputResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<OperationBlockResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRefResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default, skip_serializing_if = "ToolOutputResource::is_empty")]
    pub details: ToolOutputResource,
    #[serde(default, skip_serializing_if = "ToolResultEnvelopeResource::is_empty")]
    pub result: ToolResultEnvelopeResource,
    /// Provider/tool-defined structured result that has no fixed schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    /// Tool-defined metadata remains an explicitly named extensibility point.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationErrorResource>,
    /// Uninterpreted provider diagnostic payload, retained separately from the
    /// typed operation projection for forward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
    #[serde(default)]
    pub lifecycle: TimeRangeResource,
}

impl OperationPartResource {
    pub fn output_text(&self) -> Option<&str> {
        if self.result.model_preview.text.is_empty() {
            (!self.model_output.text.is_empty()).then_some(self.model_output.text.as_str())
        } else {
            Some(self.result.model_preview.text.as_str())
        }
    }
    pub fn title(&self) -> Option<&str> {
        (!self.title.is_empty()).then_some(self.title.as_str())
    }
    pub fn error_message(&self) -> Option<&str> {
        self.result
            .error
            .as_ref()
            .or(self.error.as_ref())
            .map(|error| error.failure.user.fallback.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolInvocationResource {
    #[serde(
        default,
        rename = "gateway_function",
        skip_serializing_if = "Option::is_none"
    )]
    pub gateway_function: Option<ToolGatewayFunctionResource>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default, skip_serializing_if = "StructuredObjectResource::is_empty")]
    pub input: StructuredObjectResource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolGatewayFunctionResource {
    #[serde(rename = "tools_list")]
    List,
    #[serde(rename = "tools_search")]
    Search,
    #[serde(rename = "tools_help")]
    Help,
    #[serde(rename = "tools_tags")]
    Tags,
    #[serde(rename = "tools_call")]
    Call,
}

/// Schema-independent object values for dynamic tool input/output. Unlike a
/// raw JSON value, this preserves number semantics and gives protocol clients
/// an exhaustive typed tree to render or inspect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StructuredObjectResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<StructuredFieldResource>,
}

impl StructuredObjectResource {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&StructuredValueResource> {
        self.fields
            .iter()
            .find_map(|field| (field.name == name).then_some(&field.value))
    }
}

impl From<StructuredObjectResource> for serde_json::Value {
    fn from(value: StructuredObjectResource) -> Self {
        Self::Object(
            value
                .fields
                .into_iter()
                .map(|field| (field.name, field.value.into()))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredFieldResource {
    pub name: String,
    pub value: StructuredValueResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredValueResource {
    Null,
    Boolean {
        value: bool,
    },
    Integer {
        value: i64,
    },
    /// Decimal/scientific notation text preserves arbitrary provider precision.
    Number {
        value: String,
    },
    Text {
        value: String,
    },
    Array {
        items: Vec<StructuredValueResource>,
    },
    Object {
        fields: Vec<StructuredFieldResource>,
    },
}

impl StructuredValueResource {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { value } => Some(value),
            _ => None,
        }
    }
}

impl From<StructuredValueResource> for serde_json::Value {
    fn from(value: StructuredValueResource) -> Self {
        match value {
            StructuredValueResource::Null => Self::Null,
            StructuredValueResource::Boolean { value } => Self::Bool(value),
            StructuredValueResource::Integer { value } => serde_json::json!(value),
            StructuredValueResource::Number { value } => serde_json::from_str(&value)
                .ok()
                .filter(Self::is_number)
                .unwrap_or(Self::String(value)),
            StructuredValueResource::Text { value } => Self::String(value),
            StructuredValueResource::Array { items } => {
                Self::Array(items.into_iter().map(Into::into).collect())
            }
            StructuredValueResource::Object { fields } => Self::Object(
                fields
                    .into_iter()
                    .map(|field| (field.name, field.value.into()))
                    .collect(),
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolManagedOutputResource {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolOutputResource {
    #[serde(default, skip_serializing_if = "StructuredObjectResource::is_empty")]
    pub payload: StructuredObjectResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<ToolManagedOutputResource>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

impl ToolOutputResource {
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty() && self.managed_outputs.is_empty() && !self.truncated
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationErrorResource {
    pub failure: agena_failure::UserProblem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelVisibleOutputResource {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

impl ModelVisibleOutputResource {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.attachments.is_empty() && !self.truncated
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStateResource {
    #[default]
    Pending,
    Running,
    Completed,
    PolicyDenied,
    UserDeclined,
    CapabilityUnavailable,
    ToolUnavailable,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolResultDisplayResource {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSectionResource>,
}

impl ToolResultDisplayResource {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.summary.is_empty() && self.sections.is_empty()
    }
}

/// Named, expanded-only presentation content for a tool Activity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolPresentationSectionResource {
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolResultEnvelopeResource {
    #[serde(default)]
    pub state: ToolResultStateResource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<OperationBlockResource>,
    #[serde(default, skip_serializing_if = "ModelVisibleOutputResource::is_empty")]
    pub model_preview: ModelVisibleOutputResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<ToolManagedOutputResource>,
    #[serde(default, skip_serializing_if = "ToolResultDisplayResource::is_empty")]
    pub display: ToolResultDisplayResource,
    /// Human-facing structured result (mirror of the runtime envelope).
    #[serde(default, skip_serializing_if = "HumanToolResultResource::is_empty")]
    pub human: HumanToolResultResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationErrorResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

impl ToolResultEnvelopeResource {
    pub fn is_empty(&self) -> bool {
        matches!(self.state, ToolResultStateResource::Pending)
            && self.structured.is_none()
            && self.content.is_empty()
            && self.model_preview.is_empty()
            && self.managed_outputs.is_empty()
            && self.display.is_empty()
            && self.human.is_empty()
            && self.attachments.is_empty()
            && self.error.is_none()
            && self.metadata.is_empty()
            && self.raw.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HumanToolResultResource {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
}

impl HumanToolResultResource {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty() && self.markdown.is_empty() && !self.streaming
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TimeRangeResource {
    pub start_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationBlockResource {
    Text {
        text: String,
    },
    Markdown {
        text: String,
    },
    Json {
        value: serde_json::Value,
    },
    Table {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        columns: Vec<TableColumnResource>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rows: Vec<Vec<serde_json::Value>>,
    },
    Log {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<String>,
        text: String,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },
    Diff {
        diff: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    FileChanges {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changes: Vec<FileChangeRecordResource>,
    },
    SearchResults {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<SearchResultItemResource>,
    },
    Citation {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snippet: Option<String>,
    },
    Image {
        mime: String,
        url: String,
    },
    Audio {
        mime: String,
        url: String,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        mime: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
    Media {
        mime_type: String,
        artifact: ArtifactRefResource,
    },
    Checklist {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<TodoItemResource>,
    },
    NestedTask {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        status: PartExecutionStatusResource,
    },
    Progress {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<f32>,
    },
    /// This is an intentionally extensible payload defined by a plugin or a
    /// tool schema; JSON is the payload's declared wire representation, not a
    /// substitute for the surrounding operation protocol.
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableColumnResource {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResultItemResource {
    pub title: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRefResource {
    pub uri: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeRecordResource {
    pub path: String,
    pub kind: FileChangeKindResource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKindResource {
    Added,
    Updated,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItemResource {
    pub content: String,
    pub status: TodoStatusResource,
    pub priority: TodoPriorityResource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatusResource {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriorityResource {
    High,
    Medium,
    Low,
}

#[cfg(test)]
mod tests {
    use super::{
        MessagePartDetailResource, MessageReasoningPartResource, MessageTextPartResource,
        OperationBlockResource, StructuredFieldResource, StructuredObjectResource,
        StructuredValueResource,
    };

    #[test]
    fn operation_blocks_and_message_details_are_explicitly_tagged() {
        assert_eq!(
            serde_json::to_value(OperationBlockResource::Command {
                command: "git status".to_owned(),
                cwd: None,
                exit_code: Some(0),
                stdout: None,
                stderr: None,
            })
            .expect("serialize operation block"),
            serde_json::json!({"type": "command", "command": "git status", "exit_code": 0})
        );
        assert_eq!(
            serde_json::to_value(MessagePartDetailResource::Text(MessageTextPartResource {
                text: "hello".to_owned(),
                synthetic: false,
            }))
            .expect("serialize message detail"),
            serde_json::json!({"type": "text", "text": "hello"})
        );
    }

    #[test]
    fn message_part_helpers_remain_protocol_owned() {
        let reasoning = MessageReasoningPartResource {
            summary: vec!["thinking ".to_owned(), "continues".to_owned()],
            raw_content: vec!["raw".to_owned()],
            encrypted_content: None,
        };
        assert_eq!(reasoning.preferred_text(), "thinking continues");

        let input = StructuredObjectResource {
            fields: vec![StructuredFieldResource {
                name: "path".to_owned(),
                value: StructuredValueResource::Text {
                    value: "/tmp/demo".to_owned(),
                },
            }],
        };
        assert_eq!(
            serde_json::Value::from(input),
            serde_json::json!({"path": "/tmp/demo"})
        );
    }
}
