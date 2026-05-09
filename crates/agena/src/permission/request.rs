use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PermissionScope {
    Session,
    Workspace,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionAction {
    BuiltinTool {
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualifier: Option<String>,
    },
    PathAccess {
        access_kind: String,
        workspace_root: String,
        target_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    pub action: PermissionAction,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReplyKind {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
}

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
pub struct PendingPermission {
    pub request: PermissionRequest,
}
