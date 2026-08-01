use serde::{Deserialize, Serialize};

use crate::{DecisionTraceStep, PermissionAction, PermissionReplyKind, PermissionRiskLevel};
use crate::{PolicyDeniedResult, UserDeclinedResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRequestedEvent {
    pub session_id: i64,
    pub operation_id: String,
    pub call_id: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRepliedEvent {
    pub session_id: i64,
    pub operation_id: String,
    pub call_id: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolPolicyDeniedEvent {
    pub session_id: i64,
    pub call_id: i64,
    pub denial: PolicyDeniedResult,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolUserDeclinedEvent {
    pub session_id: i64,
    pub call_id: i64,
    pub decline: UserDeclinedResult,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::PermissionRepliedEvent;
    use crate::PermissionReplyKind;

    #[test]
    fn permission_reply_event_omits_absent_optional_fields() {
        let value = PermissionRepliedEvent {
            session_id: 1,
            operation_id: "operation".into(),
            call_id: 7,
            request_id: "request".into(),
            kind: PermissionReplyKind::AllowOnce,
            reason: None,
            scope: None,
            ts_ms: 2,
        };
        let json = serde_json::to_value(value).unwrap();
        assert!(json.get("reason").is_none());
        assert!(json.get("scope").is_none());
    }
}
