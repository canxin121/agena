//! Declarative read/write permission modes for a path scope.

use serde::{Deserialize, Serialize};

use crate::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Read/write permission modes for a path class.
pub struct PathAccessModes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<PermissionMode>,
}

impl PathAccessModes {
    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.read.is_some() {
            self.read = overlay.read;
        }
        if overlay.write.is_some() {
            self.write = overlay.write;
        }
    }
}

/// Declarative path-rule shape. Interpretation of shorthand strings belongs
/// to the policy adapter rather than this value crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathAccessRuleConfig {
    Modes(PathAccessModes),
    Shorthand(String),
}

#[cfg(test)]
mod tests {
    use super::PathAccessModes;
    use crate::PermissionMode;

    #[test]
    fn overlay_preserves_unspecified_modes() {
        let mut base = PathAccessModes {
            read: Some(PermissionMode::Allow),
            write: Some(PermissionMode::Ask),
        };
        base.merge_from(PathAccessModes {
            read: None,
            write: Some(PermissionMode::Deny),
        });
        assert_eq!(base.read, Some(PermissionMode::Allow));
        assert_eq!(base.write, Some(PermissionMode::Deny));
    }
}
