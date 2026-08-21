use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A pending request for user input.
pub struct UserInputRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Optional Markdown body shown in the review dialog (the full plan
    /// document for plan-approval reviews; empty for other requests).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(
        default,
        rename = "input_kind",
        skip_serializing_if = "String::is_empty"
    )]
    pub kind: String,
    /// Origin of the request: `Host` for the runtime's own `ask_user`, `Plugin`
    /// for third-party/tool asks.
    pub source: agena_domain::UserInputSource,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_user_input_keeps_variant_and_input_kinds_distinct() {
        let pending = PendingInteractiveRequest::UserInput {
            request: UserInputRequest {
                request_id: "input-1".to_owned(),
                session_id: Some(1),
                title: "Choose".to_owned(),
                body_markdown: String::new(),
                kind: "ask_user".to_owned(),
                source: agena_domain::UserInputSource::Plugin,
                auto_resolution_ms: None,
                presented_at: None,
                questions: Vec::new(),
                created_at: Utc::now(),
            },
        };
        let value = serde_json::to_value(&pending).expect("serialize pending user input");
        assert_eq!(value["kind"], "user_input");
        assert_eq!(value["input_kind"], "ask_user");
        let decoded: PendingInteractiveRequest =
            serde_json::from_value(value).expect("deserialize pending user input");
        let PendingInteractiveRequest::UserInput { request } = decoded else {
            panic!("expected user-input variant");
        };
        assert_eq!(request.kind, "ask_user");
    }
}

pub fn user_input_answers_is_empty(
    value: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    value.is_empty()
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
