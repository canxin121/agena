use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{DecisionTraceStep, PermissionAction, PermissionReplyKind, PermissionScope};

/// A permission decision requested from an interactive client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
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
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<DecisionTraceStep>,
    pub created_at: DateTime<Utc>,
}

/// An interactive client's reply to a permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReply {
    pub request_id: String,
    pub kind: PermissionReplyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Permission request awaiting a decision.
pub struct PendingPermission {
    pub request: PermissionRequest,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{PermissionReply, PermissionRequest};
    use crate::{PermissionAction, PermissionReplyKind};

    #[test]
    fn permission_interaction_values_use_the_stable_compact_wire_shape() {
        let request = PermissionRequest {
            request_id: "request-1".into(),
            session_id: None,
            action: PermissionAction::Tool {
                tool_name: "inspect_config".into(),
                qualifier: None,
            },
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "needed".into(),
            explanation: String::new(),
            source: None,
            scope: None,
            operator: None,
            trace: Vec::new(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("explanation").is_none());
        assert!(json.get("related_actions").is_none());
        assert!(json.get("risk").is_none());

        let reply = PermissionReply {
            request_id: "request-1".into(),
            kind: PermissionReplyKind::AllowOnce,
            reason: None,
            scope: None,
        };
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"request_id":"request-1","kind":"allow_once"}"#
        );
    }
}
