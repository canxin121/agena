use serde::{Deserialize, Serialize};

/// Hard capability boundary for one Agena execution.
///
/// This is deliberately separate from permission policy: access determines
/// which tools can be presented at all, while permission policy decides
/// whether a presented operation is allowed, denied, or requires approval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAccess {
    /// Inherit the complete live tool catalog from the parent/runtime.
    #[default]
    Inherit,
    /// Present only tools explicitly tagged as read-only.
    ReadOnly,
}

impl ExecutionAccess {
    pub fn is_inherit(&self) -> bool {
        *self == Self::Inherit
    }
}
