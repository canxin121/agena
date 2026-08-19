//! Session-scoped plugin-operation invocation with runtime-owned authorization.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
/// Request to run a plugin operation in a session.
pub struct SessionPluginOperationRequest {
    pub session_id: i64,
    pub plugin_id: String,
    pub operation_id: String,
    pub input: serde_json::Value,
    /// Presentation metadata preserved for plugins that inspect operation
    /// invocation context. Authorization remains entirely inside Runtime.
    pub slash: Option<String>,
    pub raw: String,
    pub workspace_root: Option<String>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
/// Error running a plugin operation.
pub enum SessionPluginOperationError {
    #[error("plugin operation execution failed: {0}")]
    Execution(String),
}

/// Invokes a plugin operation in an existing session.
///
/// Plugin operations are an explicit user-control surface, not
/// execution tools, so Runtime does not synthesize a separate tool permission
/// for the operation identity. Operations that need protected filesystem, network,
/// shell, credential, or other effects must delegate them to a registered tool
/// or a permission-enforcing Host API.
#[async_trait]
/// Service that runs plugin operations in sessions.
pub trait SessionPluginOperationService: Send + Sync {
    async fn invoke_session_plugin_operation(
        &self,
        request: SessionPluginOperationRequest,
    ) -> Result<agena_plugin_host::sdk::PluginOperationResult, SessionPluginOperationError>;
}
