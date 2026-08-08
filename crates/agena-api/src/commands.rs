//! Side-effectful operations the client can invoke. One enum,
//! `#[serde(tag = "method", content = "params")]`, exhaustive.
//!
//! Both REST and WS transports route into the same enum; the server
//! dispatches by `method`. Each variant pairs with a `…Result` type defined
//! below.

use serde::{Deserialize, Serialize};

use crate::resource::{
    BackgroundActivityResource, PermissionMode, PermissionReply, RunOptions,
    SessionExecutionResource, SessionResource, UserInputReply, WorkspaceResource,
};
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
    SubmitMessage(SubmitMessageParams),
    ContinueRun(ContinueRunParams),
    CompactSession(CompactSessionParams),
    CancelRun(CancelRunParams),
    RewindSession(RewindSessionParams),

    // ── tree / fork / portability ──
    ForkSession(ForkSessionParams),
    ListSessionTree(ListSessionTreeParams),
    ExportSession(ExportSessionParams),
    ImportSession(ImportSessionParams),

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // protocol-shaped enum; boxing breaks wire format
/// Result of executing a [`Command`].
pub enum CommandResult {
    Workspace(WorkspaceResource),
    WorkspaceDeleted { id: i64 },
    Session(SessionResource),
    SessionDeleted { id: i64 },
    SessionTree(Vec<SessionResource>),
    SessionExport { jsonl: String },
    Execution(SessionExecutionResource),
    Cancellation(agena_domain::CancellationResult),
    PermissionRule(crate::resource::PermissionRuleResource),
    PermissionRuleDeleted { id: i64 },
    Activity(BackgroundActivityResource),
    ActivityDeleted { id: String },
    ActivitiesCleared { count: usize },
    Ack,
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
pub struct SubmitMessageParams {
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
/// Parameters for cancelling an active execution.
pub struct CancelRunParams {
    pub session_id: i64,
    pub execution_id: ExecutionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for rewinding a session to an earlier turn.
pub struct RewindSessionParams {
    pub session_id: i64,
    pub turn_id: agena_domain::TurnId,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

/// Clone a session's history into a new child session.
///
/// `at_message_id = None` clones the entire history; otherwise the fork stops
/// at (and includes) the last event tied to that message id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSessionParams {
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

/// Export a session as a portable JSONL bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSessionParams {
    pub session_id: i64,
}

/// Replay a JSONL bundle into the current workspace as a fresh session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSessionParams {
    pub jsonl: String,
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
