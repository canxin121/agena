//! Side-effectful operations the client can invoke. One enum,
//! `#[serde(tag = "method", content = "params")]`, exhaustive.
//!
//! Both REST and WS transports route into the same enum; the server
//! dispatches by `method`. Each variant pairs with a `…Result` type defined
//! below.

use serde::{Deserialize, Serialize};

use super::resource::{PermissionMode, PermissionReply, RunOptions, UserInputReply};
use agena_domain::{ComposerDocument, ExecutionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
/// A validated command sent to the runtime, expressed as typed parameters.
pub enum Command {
    // ── workspace ──
    CreateWorkspace(CreateWorkspaceParams),
    UpdateWorkspace(UpdateWorkspaceParams),
    DeleteWorkspace(DeleteWorkspaceParams),
    ResolveWorkspace(ResolveWorkspaceParams),

    // ── session lifecycle ──
    CreateSession(CreateSessionParams),
    UpdateSession(UpdateSessionParams),
    UpdateSessionSelection(UpdateSessionSelectionParams),
    DeleteSession(DeleteSessionParams),

    // ── message / run ──
    SubmitMessage(SubmitRunParams),
    ContinueRun(ContinueRunParams),
    CompactSession(CompactSessionParams),
    ClearSession(ClearSessionParams),
    CancelRun(CancelRunParams),
    RewindSession(RewindSessionParams),

    // ── tree / fork / portability ──
    ForkSession(ForkSessionParams),
    ListSessionTree(ListSessionTreeParams),

    // ── interactive replies ──
    ReplyPermission(ReplyPermissionParams),
    ReplyUserInput(ReplyUserInputParams),
    MarkInteractiveRequestPresented(MarkInteractiveRequestPresentedParams),

    // ── permission rules ──
    UpsertPermissionRule(UpsertPermissionRuleParams),
    ReplacePermissionRule(ReplacePermissionRuleParams),
    RevokePermissionRule(RevokePermissionRuleParams),
    DeletePermissionRule(DeletePermissionRuleParams),

    // ── background activities ──
    StopActivity(StopActivityParams),
    DismissActivity(DismissActivityParams),
    ClearFinishedActivities,
}

// ─── workspace ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for creating a workspace.
pub struct CreateWorkspaceParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for updating a workspace (path and optimistic version).
pub struct UpdateWorkspaceParams {
    pub workspace_id: i64,
    pub path: String,
    /// Optional `If-Match`-style optimistic concurrency check.
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for deleting a workspace.
pub struct DeleteWorkspaceParams {
    pub workspace_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for resolving a workspace by path, optionally creating it.
pub struct ResolveWorkspaceParams {
    pub path: String,
    #[serde(default)]
    pub create_if_missing: bool,
}

// ─── session ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for creating a session inside a workspace.
pub struct CreateSessionParams {
    pub workspace_id: i64,
    pub title: String,
    /// Treat the initial title as a placeholder eligible for automatic naming.
    #[serde(default)]
    pub auto_title: bool,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for updating session metadata.
pub struct UpdateSessionParams {
    pub session_id: i64,
    pub title: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for updating the run selection (model, thinking, speed mode) of a session.
pub struct UpdateSessionSelectionParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for deleting a session.
pub struct DeleteSessionParams {
    pub session_id: i64,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

// ─── message / run ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for submitting a user message (composer document) to a session.
pub struct SubmitRunParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub document: ComposerDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for continuing a paused or interrupted run.
pub struct ContinueRunParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for requesting context compaction of a session.
pub struct CompactSessionParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Parameters for clearing a conversation's history without deleting it.
pub struct ClearSessionParams {
    pub session_id: i64,
    #[serde(default)]
    pub mode: SessionClearMode,
    #[serde(default)]
    pub options: RunOptions,
}

/// How much of a conversation a clear hides. Both modes keep stored history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionClearMode {
    /// Start over completely: transcript and model both see nothing from before.
    #[default]
    History,
    /// Keep the transcript readable and restart only the model's context.
    ContextOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for cancelling an active execution.
pub struct CancelRunParams {
    pub session_id: i64,
    /// Optional for the user-facing stop action. When absent, the server
    /// cancels whichever execution is current and suppresses queued wakes;
    /// when present, cancellation remains exact and race-safe.
    #[serde(default)]
    pub execution_id: Option<ExecutionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Parameters for retracting a completed user message.
pub struct RewindSessionParams {
    pub session_id: i64,
    pub at_message_id: i64,
    /// `fork` (default) keeps the original session and creates a branch;
    /// `in_place` hard-truncates this session's history before the message.
    #[serde(default)]
    pub mode: RewindMode,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

pub use agena_domain::RewindMode;

/// Clone a session's history into a new child session.
///
/// `at_message_id = None` clones the entire history; otherwise the fork stops
/// at (and includes) the last event tied to that message id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSessionParams {
    #[serde(default)]
    pub conversation_mode: Option<agena_domain::ConversationMode>,
    pub session_id: i64,
    #[serde(default)]
    pub at_message_id: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
}

/// List every session sharing the given tree root, in `(depth, id)` order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionTreeParams {
    pub root_id: i64,
}

// ─── interactive replies ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for replying to a pending permission request.
pub struct ReplyPermissionParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for replying to a pending user-input request.
pub struct ReplyUserInputParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for marking an interactive request as presented to the user.
pub struct MarkInteractiveRequestPresentedParams {
    pub session_id: i64,
    pub request_id: String,
}

// ─── permission rules ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for creating or updating a permission rule.
pub struct UpsertPermissionRuleParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_access_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for replacing an existing permission rule by id.
pub struct ReplacePermissionRuleParams {
    pub rule_id: i64,
    #[serde(flatten)]
    pub rule: UpsertPermissionRuleParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for revoking a permission rule.
pub struct RevokePermissionRuleParams {
    pub rule_id: i64,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for deleting a permission rule.
pub struct DeletePermissionRuleParams {
    pub rule_id: i64,
}

// ─── background activities ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for stopping a background activity.
pub struct StopActivityParams {
    pub activity_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for dismissing a background activity.
pub struct DismissActivityParams {
    pub activity_id: String,
}

impl Command {
    /// The session this write targets, when it is session-scoped.
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Command::CreateWorkspace(_)
            | Command::UpdateWorkspace(_)
            | Command::DeleteWorkspace(_)
            | Command::ResolveWorkspace(_)
            | Command::UpsertPermissionRule(_)
            | Command::ReplacePermissionRule(_)
            | Command::RevokePermissionRule(_)
            | Command::DeletePermissionRule(_)
            | Command::StopActivity(_)
            | Command::DismissActivity(_)
            | Command::ClearFinishedActivities => None,
            // Creating a top-level session has no existing session to scope.
            Command::CreateSession(params) => params.parent_id,
            Command::UpdateSession(params) => Some(params.session_id),
            Command::UpdateSessionSelection(params) => Some(params.session_id),
            Command::DeleteSession(params) => Some(params.session_id),
            Command::SubmitMessage(params) => Some(params.session_id),
            Command::ContinueRun(params) => Some(params.session_id),
            Command::CompactSession(params) => Some(params.session_id),
            Command::ClearSession(params) => Some(params.session_id),
            Command::CancelRun(params) => Some(params.session_id),
            Command::RewindSession(params) => Some(params.session_id),
            Command::ForkSession(params) => Some(params.session_id),
            Command::ListSessionTree(params) => Some(params.root_id),
            Command::ReplyPermission(params) => Some(params.session_id),
            Command::ReplyUserInput(params) => Some(params.session_id),
            Command::MarkInteractiveRequestPresented(params) => Some(params.session_id),
        }
    }

    /// Compatibility check for callers using the shared plugin command API.
    ///
    /// Plugins may invoke every command. Operation-specific validation is
    /// performed by the runtime when the command executes.
    pub fn plugin_allowed(&self) -> Result<(), &'static str> {
        Ok(())
    }
}

#[cfg(test)]
mod permission_write_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permission_write_preserves_missing_null_and_present_fields() {
        for wire in [
            json!({"mode":"ask"}),
            json!({"mode":"ask", "scope":null, "session_id":null}),
        ] {
            let request: UpsertPermissionRuleParams = serde_json::from_value(wire).unwrap();
            assert_eq!(
                serde_json::to_value(request).unwrap(),
                json!({"mode":"ask"})
            );
        }
        let wire = json!({
            "action_key":"network:https://example.test:443", "subject_kind":"network",
            "tool_name":"fetch", "qualifier":"", "path_access_kind":"read",
            "workspace_root":"/work", "target_path":"/work/file.txt",
            "network_target":"https://example.test:443", "network_host":"example.test",
            "network_port":443, "scope":"session", "session_id":7, "mode":"allow"
        });
        let request: UpsertPermissionRuleParams = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), wire);
        for invalid in [
            json!({}),
            json!({"mode":null}),
            json!({"mode":"unknown"}),
            json!({"mode":"ask", "network_port":65536}),
        ] {
            assert!(serde_json::from_value::<UpsertPermissionRuleParams>(invalid).is_err());
        }
    }
}

#[cfg(test)]
mod plugin_access_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugins_can_use_workspace_session_policy_and_activity_commands() {
        let requests = [
            json!({"method": "create_workspace", "params": {"path": "/work"}}),
            json!({"method": "resolve_workspace", "params": {"path": "/work", "create_if_missing": true}}),
            json!({"method": "update_workspace", "params": {"workspace_id": 1, "path": "/new-work"}}),
            json!({"method": "delete_workspace", "params": {"workspace_id": 1}}),
            json!({"method": "create_session", "params": {"workspace_id": 1, "title": "Root"}}),
            json!({"method": "delete_session", "params": {"session_id": 7}}),
            json!({"method": "upsert_permission_rule", "params": {"mode": "ask"}}),
            json!({"method": "replace_permission_rule", "params": {"rule_id": 1, "mode": "ask"}}),
            json!({"method": "revoke_permission_rule", "params": {"rule_id": 1}}),
            json!({"method": "delete_permission_rule", "params": {"rule_id": 1}}),
            json!({"method": "stop_activity", "params": {"activity_id": "activity-1"}}),
            json!({"method": "dismiss_activity", "params": {"activity_id": "activity-1"}}),
            json!({"method": "clear_finished_activities"}),
        ];
        for request in requests {
            let command: Command = serde_json::from_value(request).unwrap();
            assert_eq!(command.plugin_allowed(), Ok(()), "{command:?}");
        }
    }
}
