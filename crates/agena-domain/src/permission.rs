use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// Policy outcome for an action that may require user permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Auto,
    Ask,
    Deny,
}

/// Persistence scope for a permission decision.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Scope of a permission decision or rule: session, workspace, or global.
pub enum PermissionScope {
    Session,
    Workspace,
    Global,
}

/// A user's answer to an interactive permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReplyKind {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
    /// Ask the automatic-approval classifier to decide this request. The
    /// runtime records the complete reply together with the classifier's
    /// resolved allow/deny outcome in the same interaction decision.
    AutoApprove,
}
