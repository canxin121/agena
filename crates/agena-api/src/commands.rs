//! Side-effectful operations the client can invoke. One enum,
//! `#[serde(tag = "method", content = "params")]`, exhaustive.
//!
//! Both REST and WS transports route into the same enum; the server
//! dispatches by `method`. Each variant pairs with a `…Result` type defined
//! below.

use serde::{Deserialize, Serialize};

use agena::session::GoalStatus;

use crate::resource::{
    MessagePartContent, PermissionMode, PermissionReply, RunOptions, SessionExecutionResource,
    SessionGoalResource, SessionResource, UserInputReply, WorkspaceResource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Command {
    // ── workspace ──
    CreateWorkspace(CreateWorkspaceParams),
    UpdateWorkspace(UpdateWorkspaceParams),
    DeleteWorkspace(DeleteWorkspaceParams),
    ResolveWorkspace(ResolveWorkspaceParams),

    // ── session lifecycle ──
    CreateSession(CreateSessionParams),
    UpdateSession(UpdateSessionParams),
    DeleteSession(DeleteSessionParams),
    SetSessionGoal(SetSessionGoalParams),
    CreateSessionGoal(CreateSessionGoalParams),
    CompleteSessionGoal(CompleteSessionGoalParams),
    ClearSessionGoal(ClearSessionGoalParams),

    // ── turn / run ──
    SubmitTurn(SubmitTurnParams),
    ContinueRun(ContinueRunParams),
    CompactSession(CompactSessionParams),
    CancelRun(CancelRunParams),
    RewindSession(RewindSessionParams),

    // ── tree / fork / portability ──
    ForkSession(ForkSessionParams),
    ListSessionTree(ListSessionTreeParams),
    ListRewindCheckpoints(ListRewindCheckpointsParams),
    ExportSession(ExportSessionParams),
    ImportSession(ImportSessionParams),

    // ── interactive replies ──
    ReplyPermission(ReplyPermissionParams),
    ReplyUserInput(ReplyUserInputParams),

    // ── permission rules ──
    UpsertPermissionRule(UpsertPermissionRuleParams),
    ReplacePermissionRule(ReplacePermissionRuleParams),
    RevokePermissionRule(RevokePermissionRuleParams),
    DeletePermissionRule(DeletePermissionRuleParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // protocol-shaped enum; boxing breaks wire format
pub enum CommandResult {
    Workspace(WorkspaceResource),
    WorkspaceDeleted { id: i64 },
    Session(SessionResource),
    SessionDeleted { id: i64 },
    SessionGoal(SessionGoalResource),
    SessionGoalCleared { session_id: i64 },
    SessionTree(Vec<SessionResource>),
    SessionExport { jsonl: String },
    RewindCheckpoints(Vec<crate::resource::RewindCheckpointResource>),
    Execution(SessionExecutionResource),
    PermissionRule(crate::resource::PermissionRuleResource),
    PermissionRuleDeleted { id: i64 },
    Ack,
}

// ─── workspace ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceParams {
    pub workspace_id: i64,
    pub path: String,
    /// Optional `If-Match`-style optimistic concurrency check.
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteWorkspaceParams {
    pub workspace_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveWorkspaceParams {
    pub path: String,
    #[serde(default)]
    pub create_if_missing: bool,
}

// ─── session ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionParams {
    pub workspace_id: i64,
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionParams {
    pub session_id: i64,
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSessionParams {
    pub session_id: i64,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionGoalParams {
    pub session_id: i64,
    pub objective: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetSessionGoalParams {
    pub session_id: i64,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteSessionGoalParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearSessionGoalParams {
    pub session_id: i64,
}

// ─── turn / run ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTurnParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub parts: Vec<MessagePartContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinueRunParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactSessionParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelRunParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindSessionParams {
    pub session_id: i64,
    pub message_id: i64,
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

/// List every persisted rewind audit checkpoint for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRewindCheckpointsParams {
    pub session_id: i64,
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
pub struct ReplyPermissionParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyUserInputParams {
    pub session_id: i64,
    #[serde(default)]
    pub options: RunOptions,
    pub reply: UserInputReply,
}

// ─── permission rules ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct ReplacePermissionRuleParams {
    pub rule_id: i64,
    #[serde(flatten)]
    pub rule: UpsertPermissionRuleParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokePermissionRuleParams {
    pub rule_id: i64,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePermissionRuleParams {
    pub rule_id: i64,
}
