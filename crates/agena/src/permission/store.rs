use thiserror::Error;

use super::request::{PermissionAction, PermissionScope};
use super::{PermissionDecision, PermissionMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedPermissionRule {
    pub action_key: String,
    pub mode: PermissionMode,
    pub scope: PermissionScope,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub source: String,
    pub reason: Option<String>,
    pub operator: Option<String>,
    pub revoked_at_ms: Option<i64>,
    pub revoked_reason: Option<String>,
    pub revoked_by: Option<String>,
}

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
