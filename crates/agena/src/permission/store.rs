use std::sync::{Arc, RwLock};

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

#[derive(Debug, Clone, Default)]
pub struct InMemoryPermissionRuleStore {
    inner: Arc<RwLock<Vec<(PermissionAction, PermissionMode)>>>,
}

impl InMemoryPermissionRuleStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PermissionRuleStore for InMemoryPermissionRuleStore {
    fn lookup(
        &self,
        action: &PermissionAction,
    ) -> Result<Option<PermissionMode>, PermissionStoreError> {
        let guard = self
            .inner
            .read()
            .map_err(|_| PermissionStoreError::LockPoisoned)?;
        Ok(guard
            .iter()
            .rev()
            .find_map(|(saved_action, mode)| (saved_action == action).then_some(*mode)))
    }

    fn save(
        &self,
        action: PermissionAction,
        mode: PermissionMode,
    ) -> Result<(), PermissionStoreError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| PermissionStoreError::LockPoisoned)?;
        guard.push((action, mode));
        Ok(())
    }
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
