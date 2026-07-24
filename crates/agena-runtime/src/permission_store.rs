use agena_domain::{PermissionAction, PermissionMode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PermissionStoreError {
    #[error("permission store failure: {0}")]
    Message(String),
}

pub trait PermissionRuleStore {
    fn lookup(
        &self,
        action: &PermissionAction,
    ) -> Result<Option<PermissionMode>, PermissionStoreError>;

    fn save(
        &mut self,
        action: PermissionAction,
        mode: PermissionMode,
    ) -> Result<(), PermissionStoreError>;
}
