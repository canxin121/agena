use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::permission::{PermissionReply, PermissionReplyKind, PermissionRequest};

use super::ExecutionStatus;

fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn user_input_answers_is_empty(value: &BTreeMap<String, Vec<String>>) -> bool {
    value.is_empty()
}

pub(crate) fn deserialize_user_input_answers<'de, D>(
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_content: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

impl ReasoningPart {
    /// Reasoning deltas arrive as arbitrary text fragments, not tokenized words.
    /// Concatenate them verbatim so we do not inject spaces inside words or
    /// duplicate whitespace that the model already emitted.
    pub fn summary_text(&self) -> String {
        self.summary.concat()
    }

    pub fn raw_text(&self) -> String {
        self.raw_content.concat()
    }

    pub fn preferred_text(&self) -> String {
        if !self.summary.is_empty() {
            self.summary_text()
        } else {
            self.raw_text()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeRecord {
    pub path: String,
    pub kind: FileChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Updated,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorPart {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::ReasoningPart;

    #[test]
    fn preferred_text_concatenates_reasoning_fragments_verbatim() {
        let reasoning = ReasoningPart {
            summary: vec![
                "The".to_string(),
                " user".to_string(),
                " wants".to_string(),
                " /m".to_string(),
                "ore".to_string(),
            ],
            raw_content: Vec::new(),
            encrypted_content: None,
        };

        assert_eq!(reasoning.preferred_text(), "The user wants /more");
    }

    #[test]
    fn preferred_text_falls_back_to_raw_content() {
        let reasoning = ReasoningPart {
            summary: Vec::new(),
            raw_content: vec!["raw".to_string(), " content".to_string()],
            encrypted_content: None,
        };

        assert_eq!(reasoning.preferred_text(), "raw content");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "request_type", rename_all = "snake_case")]
pub enum RequestPart {
    Permission(InteractiveRequestPart<PermissionRequest, PermissionReply>),
    UserInput(InteractiveRequestPart<UserInputRequest, UserInputReply>),
}

macro_rules! map_request_part {
    ($value:expr, |$part:ident| $body:expr) => {
        match $value {
            RequestPart::Permission($part) => $body,
            RequestPart::UserInput($part) => $body,
        }
    };
}

impl RequestPart {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::Permission(_) => PendingInteractiveRequestKind::Permission,
            Self::UserInput(_) => PendingInteractiveRequestKind::UserInput,
        }
    }

    pub const fn status(&self) -> ExecutionStatus {
        map_request_part!(self, |part| part.status())
    }

    pub fn summary_text(&self) -> String {
        map_request_part!(self, |part| part.summary_text())
    }

    pub fn request_id(&self) -> &str {
        map_request_part!(self, |part| part.request_id())
    }

    pub fn pending_interactive_request(&self) -> Option<PendingInteractiveRequest> {
        map_request_part!(self, |part| {
            part.pending_request().map(PendingInteractiveRequest::from)
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingInteractiveRequestKind {
    Permission,
    UserInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingInteractiveRequest {
    Permission {
        #[serde(flatten)]
        request: PermissionRequest,
    },
    UserInput {
        #[serde(flatten)]
        request: UserInputRequest,
    },
}

macro_rules! map_pending_interactive_request {
    ($value:expr, |$request:ident| $body:expr) => {
        match $value {
            PendingInteractiveRequest::Permission { request: $request } => $body,
            PendingInteractiveRequest::UserInput { request: $request } => $body,
        }
    };
}

impl PendingInteractiveRequest {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::Permission { .. } => PendingInteractiveRequestKind::Permission,
            Self::UserInput { .. } => PendingInteractiveRequestKind::UserInput,
        }
    }

    pub fn request_id(&self) -> &str {
        map_pending_interactive_request!(self, |request| request.request_id.as_str())
    }

    pub const fn session_id(&self) -> Option<i64> {
        map_pending_interactive_request!(self, |request| request.session_id)
    }

    pub fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        map_pending_interactive_request!(self, |request| request.created_at)
    }

    pub fn as_permission(&self) -> Option<&PermissionRequest> {
        match self {
            Self::Permission { request } => Some(request),
            Self::UserInput { .. } => None,
        }
    }

    pub fn as_user_input(&self) -> Option<&UserInputRequest> {
        match self {
            Self::Permission { .. } => None,
            Self::UserInput { request } => Some(request),
        }
    }
}

macro_rules! impl_pending_interactive_request_from {
    ($request_type:ty, $variant:ident) => {
        impl From<$request_type> for PendingInteractiveRequest {
            fn from(request: $request_type) -> Self {
                Self::$variant { request }
            }
        }
    };
}

impl_pending_interactive_request_from!(PermissionRequest, Permission);
impl_pending_interactive_request_from!(UserInputRequest, UserInput);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveRequestPart<Request, Reply> {
    pub request: Request,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
}

impl<Request, Reply> InteractiveRequestPart<Request, Reply> {
    pub fn pending(request: Request) -> Self {
        Self {
            request,
            reply: None,
        }
    }

    pub fn with_reply(mut self, reply: Reply) -> Self {
        self.reply = Some(reply);
        self
    }

    pub const fn status(&self) -> ExecutionStatus {
        if self.reply.is_some() {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Pending
        }
    }

    pub fn pending_request(&self) -> Option<Request>
    where
        Request: Clone,
    {
        self.reply.is_none().then_some(self.request.clone())
    }
}

impl InteractiveRequestPart<PermissionRequest, PermissionReply> {
    pub fn request_id(&self) -> &str {
        self.request.request_id.as_str()
    }

    pub fn summary_text(&self) -> String {
        match self.reply.as_ref() {
            None => format!("Awaiting permission: {}", self.request.reason),
            Some(reply) => {
                let reason = reply
                    .reason
                    .as_deref()
                    .unwrap_or(self.request.reason.as_str());
                let prefix = match reply.kind {
                    PermissionReplyKind::AllowOnce => "Permission allowed once",
                    PermissionReplyKind::AllowAlways => "Permission allowed always",
                    PermissionReplyKind::DenyOnce => "Permission denied once",
                    PermissionReplyKind::DenyAlways => "Permission denied always",
                };
                format!("{prefix}: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UserInputOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UserInputQuestion {
    pub id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub submit_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cancel_label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<UserInputQuestion>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInputReplyKind {
    Submit,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputReply {
    pub request_id: String,
    pub kind: UserInputReplyKind,
    #[serde(
        default,
        deserialize_with = "deserialize_user_input_answers",
        skip_serializing_if = "user_input_answers_is_empty"
    )]
    pub answers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl InteractiveRequestPart<UserInputRequest, UserInputReply> {
    pub fn request_id(&self) -> &str {
        self.request.request_id.as_str()
    }

    pub fn summary_text(&self) -> String {
        match self.reply.as_ref() {
            None => {
                let count = self.request.questions.len();
                match count {
                    0 => "Ask user".to_string(),
                    1 => "Waiting for answer".to_string(),
                    _ => format!("Waiting for {count} answers"),
                }
            }
            Some(reply) => match reply.kind {
                UserInputReplyKind::Submit => {
                    format!("Answered {} question(s)", reply.answers.len())
                }
                UserInputReplyKind::Cancel => {
                    let reason = reply
                        .reason
                        .as_deref()
                        .filter(|reason| !reason.trim().is_empty())
                        .unwrap_or("user declined to answer");
                    format!("Ask cancelled: {reason}")
                }
            },
        }
    }
}
