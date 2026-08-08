use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A pending request for user input.
pub struct UserInputRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resolution_ms: Option<u64>,
    /// Durable presentation acknowledgement set when a client has shown this
    /// request to the user. Outstanding requests with `presented_at == null`
    /// must always be surfaced; presented-but-unanswered requests remain
    /// visible through persistent attention hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presented_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<UserInputQuestion>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a pending interactive request.
pub enum PendingInteractiveRequestKind {
    Permission,
    UserInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// A pending interactive request: permission or user input.
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

impl PendingInteractiveRequest {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::Permission { .. } => PendingInteractiveRequestKind::Permission,
            Self::UserInput { .. } => PendingInteractiveRequestKind::UserInput,
        }
    }
    pub const fn as_permission(&self) -> Option<&PermissionRequest> {
        match self {
            Self::Permission { request } => Some(request),
            Self::UserInput { .. } => None,
        }
    }
    pub const fn as_user_input(&self) -> Option<&UserInputRequest> {
        match self {
            Self::Permission { .. } => None,
            Self::UserInput { request } => Some(request),
        }
    }
    pub fn request_id(&self) -> &str {
        match self {
            Self::Permission { request } => request.request_id.as_str(),
            Self::UserInput { request } => request.request_id.as_str(),
        }
    }
}

pub fn user_input_answers_is_empty(
    value: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    value.is_empty()
}

pub fn deserialize_user_input_answers<'de, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawAnswerValues {
        Single(String),
        Multiple(Vec<String>),
    }

    let raw = std::collections::BTreeMap::<String, RawAnswerValues>::deserialize(deserializer)?;
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

#[cfg(test)]
mod user_input_reply_contract_tests {
    use super::{UserInputReply, UserInputReplyKind};

    #[test]
    fn user_input_reply_accepts_legacy_single_and_multiple_answers() {
        let reply: UserInputReply = serde_json::from_value(serde_json::json!({
            "request_id": "request-1",
            "kind": "submit",
            "answers": { "one": "yes", "many": ["a", "b"] }
        }))
        .expect("deserialize reply");
        assert_eq!(reply.kind, UserInputReplyKind::Submit);
        assert_eq!(reply.answers["one"], ["yes"]);
        assert_eq!(reply.answers["many"], ["a", "b"]);
    }
}

#[cfg(test)]
mod permission_reply_contract_tests {
    use super::{PermissionReply, PermissionReplyKind, PermissionScope};

    #[test]
    fn permission_reply_has_a_runtime_independent_wire_shape() {
        let reply = PermissionReply {
            request_id: "request-1".to_owned(),
            kind: PermissionReplyKind::AllowAlways,
            reason: Some("trusted workspace".to_owned()),
            scope: Some(PermissionScope::Workspace),
        };
        assert_eq!(
            serde_json::to_value(reply).expect("serialize permission reply"),
            serde_json::json!({
                "request_id": "request-1",
                "kind": "allow_always",
                "reason": "trusted workspace",
                "scope": "workspace"
            })
        );
    }
}
