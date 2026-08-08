//! Session-scoped plugin-command invocation with runtime-owned authorization.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
/// Request to run a plugin command in a session.
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
/// Error running a plugin command.
pub enum SessionPluginCommandError {
    #[error("plugin command execution failed: {0}")]
    Execution(String),
}

/// Invokes a plugin command in an existing session.
///
/// Plugin commands are an explicit user-control and UI-routing surface, not
/// execution tools, so Runtime does not synthesize a separate tool permission
/// for the command identity. Commands that need protected filesystem, network,
/// shell, credential, or other effects must delegate them to a registered tool
/// or a permission-enforcing Host API.
#[async_trait]
/// Service that runs plugin commands in sessions.
pub trait SessionPluginCommandService: Send + Sync {
    async fn invoke_session_plugin_command(
        &self,
        request: SessionPluginCommandRequest,
    ) -> Result<agena_plugin_host::sdk::PluginCommandOutput, SessionPluginCommandError>;
}
