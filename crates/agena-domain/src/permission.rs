use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// Policy outcome for an action that may require user permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

/// Persistence scope for a permission decision.
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

/// A user's answer to an interactive permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReplyKind {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
}

#[cfg(test)]
mod tests {
    use super::{PermissionMode, PermissionReplyKind, PermissionScope};

    #[test]
    fn permission_values_have_stable_wire_spellings() {
        assert_eq!(
            serde_json::to_string(&PermissionMode::Ask).unwrap(),
            "\"ask\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionScope::Workspace).unwrap(),
            "\"workspace\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionReplyKind::AllowAlways).unwrap(),
            "\"allow_always\""
        );
    }
}
