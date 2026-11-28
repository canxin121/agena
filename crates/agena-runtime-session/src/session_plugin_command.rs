//! Session-scoped plugin-command invocation with runtime-owned authorization.

use agena_domain::ComposerDocument;
use agena_failure::Failure;
use async_trait::async_trait;
use futures_util::future::BoxFuture;

#[derive(Debug, Clone)]
/// Request to run an attachment-aware plugin command in a session.
pub struct SessionPluginCommandRequest {
    pub session_id: i64,
    pub plugin_id: String,
    pub command_id: String,
    pub input: serde_json::Value,
    /// Presentation metadata preserved for plugins that inspect command
    /// invocation context. Authorization remains entirely inside Runtime.
    pub slash: Option<String>,
    pub raw: String,
    pub document: ComposerDocument,
}

/// Build a user submission using the normal ingress identity and run options.
/// Model selection is resolved only if the command produces a followup.
pub type BuildCommandFollowup<'a> = Box<
    dyn FnOnce(ComposerDocument) -> BoxFuture<'a, Result<crate::SessionUserRunRequest, Failure>>
        + Send
        + 'a,
>;

/// Invokes a plugin command in an existing session.
///
/// Plugin commands are an explicit user-control surface, not
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
        build_followup: BuildCommandFollowup<'_>,
    ) -> Result<agena_plugin_host::sdk::CommandResult, Failure>;
}
