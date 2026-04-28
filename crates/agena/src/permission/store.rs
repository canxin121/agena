use thiserror::Error;

use super::request::PermissionAction;
use super::{PermissionDecision, PermissionMode};

#[derive(Debug, Error)]
pub enum PermissionStoreError {
    #[error("permission store lock poisoned")]
    LockPoisoned,
}

pub trait PermissionRuleStore: Send + Sync {
    fn lookup(
        &self,
        action: &PermissionAction,
    ) -> Result<Option<PermissionMode>, PermissionStoreError>;
    fn save(
        &self,
        action: PermissionAction,
        mode: PermissionMode,
    ) -> Result<(), PermissionStoreError>;
}

pub fn decide_from_mode(mode: PermissionMode, reason: impl Into<String>) -> PermissionDecision {
    match mode {
        PermissionMode::Allow => PermissionDecision::Allow,
        PermissionMode::Ask => PermissionDecision::Ask {
            reason: reason.into(),
        },
        PermissionMode::Deny => PermissionDecision::Deny {
            reason: reason.into(),
        },
    }
}
