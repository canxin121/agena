//! Session-scoped plugin-command invocation with runtime-owned authorization.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SessionPluginCommandRequest {
    pub session_id: i64,
    pub plugin_id: String,
    pub command_id: String,
    pub input: serde_json::Value,
    /// Presentation metadata preserved for plugins that inspect command
    /// invocation context. Authorization remains entirely inside Runtime.
    pub slash: Option<String>,
    pub raw: String,
    pub workspace_root: Option<String>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SessionPluginCommandError {
    #[error("plugin command requires approval: {0}")]
    ApprovalRequired(String),
    #[error("plugin command denied: {0}")]
    Denied(String),
    #[error("plugin command execution failed: {0}")]
    Execution(String),
}

#[async_trait]
pub trait SessionPluginCommandService: Send + Sync {
    async fn invoke_session_plugin_command(
        &self,
        request: SessionPluginCommandRequest,
    ) -> Result<agena_plugin_host::sdk::PluginCommandOutput, SessionPluginCommandError>;
}
