use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// Kind of a file-system change reported in message activity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Updated,
    Deleted,
    Moved,
}

/// State of a todo item in message activity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

/// Relative priority of a todo item in message activity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    High,
    Medium,
    Low,
}

/// Stable message-activity todo item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Column definition for a table message part.
pub struct TableColumn {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Search result item in a message part.
pub struct SearchResultItem {
    pub title: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Reference to an artifact (URI, MIME, size, hash).
pub struct ArtifactRef {
    pub uri: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
/// Option offered in a user input question.
pub struct UserInputOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
/// Question asked to the user with selectable options.
pub struct UserInputQuestion {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<UserInputOption>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiple: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_custom: bool,
}

pub fn user_input_answers_is_empty(value: &BTreeMap<String, Vec<String>>) -> bool {
    value.is_empty()
}

pub fn deserialize_user_input_answers<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawAnswerValues {
        Single(String),
        Multiple(Vec<String>),
    }
    let raw = BTreeMap::<String, RawAnswerValues>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(question_id, values)| {
            let values = match values {
                RawAnswerValues::Single(value) => vec![value],
                RawAnswerValues::Multiple(values) => values,
            };
            (question_id, values)
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Request for user input presented to the user.
pub struct UserInputRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Optional Markdown body for the request. Only the plan-approval review
    /// (`kind == Review`) populates it with the full plan document so the
    /// review dialog can show what is being approved; other ask_user requests
    /// leave it empty and the dialog shows only the questions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(default)]
    pub kind: crate::UserInputKind,
    /// Origin of the request: the runtime's own host `ask_user` (`Host`) vs a
    /// third-party/tool `interaction.ask` (`Plugin`). Legacy rows predate the
    /// field and default to `Plugin`; their real origin is recovered from the
    /// correlation ids by the runtime's typed accessor.
    #[serde(default, skip_serializing_if = "user_input_source_is_default")]
    pub source: crate::UserInputSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resolution_ms: Option<u64>,
    /// Durable presentation acknowledgement: set when a client has shown this
    /// request to the user. Outstanding requests that were never presented are
    /// always surfaced; requests that were presented but remain unanswered are
    /// kept visible through persistent attention hints instead of re-prompting
    /// modals. `None` for requests created before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presented_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<UserInputQuestion>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub fn user_input_source_is_default(source: &crate::UserInputSource) -> bool {
    *source == crate::UserInputSource::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// The user's reply to a [`UserInputRequest`].
pub struct UserInputReply {
    pub request_id: String,
    pub kind: crate::UserInputReplyKind,
    #[serde(
        default,
        deserialize_with = "deserialize_user_input_answers",
        skip_serializing_if = "user_input_answers_is_empty"
    )]
    pub answers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Durable user-input history owned by one tool Operation.
///
/// Like permission, an interactive ask is not transcript content of its own:
/// the request and its reply live beside the invocation and result they govern
/// (inside the `tool_call` operation activity), so one host ask produces exactly
/// one transcript activity — the operation. Interactive clients derive their
/// pending user-input queue from unresolved records here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationUserInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<OperationUserInputRecord>,
}

impl OperationUserInput {
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn awaiting(&self) -> impl Iterator<Item = &OperationUserInputRecord> {
        self.requests.iter().filter(|record| record.reply.is_none())
    }

    pub fn find(&self, request_id: &str) -> Option<&OperationUserInputRecord> {
        self.requests
            .iter()
            .find(|record| record.request.request_id == request_id)
    }

    pub fn find_mut(&mut self, request_id: &str) -> Option<&mut OperationUserInputRecord> {
        self.requests
            .iter_mut()
            .find(|record| record.request.request_id == request_id)
    }

    pub fn push_pending(&mut self, request: UserInputRequest) -> bool {
        if self.find(request.request_id.as_str()).is_some() {
            return false;
        }
        self.requests.push(OperationUserInputRecord {
            request,
            reply: None,
            replied_at_ms: None,
        });
        true
    }

    pub fn record_reply(&mut self, reply: UserInputReply, replied_at_ms: i64) -> bool {
        let Some(record) = self.find_mut(reply.request_id.as_str()) else {
            return false;
        };
        if record.reply.is_some() {
            return false;
        }
        record.reply = Some(reply);
        record.replied_at_ms = Some(replied_at_ms);
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// User-input request/reply pair attached to an operation activity.
pub struct OperationUserInputRecord {
    pub request: UserInputRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<UserInputReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replied_at_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::{FileChangeKind, TodoPriority, TodoStatus, UserInputReply};

    #[test]
    fn message_activity_values_have_stable_wire_spellings() {
        assert_eq!(
            serde_json::to_string(&FileChangeKind::Moved).unwrap(),
            "\"moved\""
        );
        assert_eq!(
            serde_json::to_string(&TodoStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&TodoPriority::Medium).unwrap(),
            "\"medium\""
        );
    }

    #[test]
    fn user_input_reply_accepts_scalar_and_array_answers() {
        let reply: UserInputReply = serde_json::from_value(serde_json::json!({
            "request_id": "r1",
            "kind": "submit",
            "answers": {"one": "yes", "many": ["a", "b"]}
        }))
        .expect("decode mixed answer shapes");
        assert_eq!(reply.answers["one"], vec!["yes"]);
        assert_eq!(reply.answers["many"], vec!["a", "b"]);
    }
}
