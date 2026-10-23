//! Backend-neutral persistence for small, instance-scoped application state.
//!
//! Values are serialized JSON documents. Credentials belong in SecretStore;
//! this repository shares the service instance's ordinary database lifecycle.

use std::path::Path;

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum InstanceStateError {
    #[error("invalid instance-state key: {0}")]
    InvalidKey(String),
    #[error("instance-state storage failed: {0}")]
    Backend(String),
}

#[async_trait]
pub trait InstanceStateRepository: Send + Sync {
    /// Stable location of the owning instance database, for diagnostics and
    /// cache identity. In-memory instances expose a synthetic unique path.
    fn path(&self) -> &Path;

    async fn get_payload(&self, key: &str) -> Result<Option<String>, InstanceStateError>;

    /// Store one complete JSON document atomically. Identical writes are a
    /// no-op and preserve the value's update timestamp.
    async fn set_payload(&self, key: &str, payload: String) -> Result<(), InstanceStateError>;
}
