//! Side-effectful operations the client can invoke. One enum,
//! `#[serde(tag = "method", content = "params")]`, exhaustive.
//!
//! Both REST and WS transports route into the same enum; the server
//! dispatches by `method`. Each variant pairs with a `…Result` type defined
//! below.

use serde::{Deserialize, Serialize};

use crate::resource::{
    MessagePartContent, PermissionMode, PermissionReply, RunOptions, SessionExecutionResource,
    SessionResource, UserInputReply, WorkspaceResource,
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

    // ── turn / run ──
    SubmitTurn(SubmitTurnParams),
    ContinueRun(ContinueRunParams),
    CancelTurn(CancelTurnParams),

    // ── interactive replies ──
    ReplyPermission(ReplyPermissionParams),
    ReplyUserInput(ReplyUserInputParams),

    // ── permission rules ──
    UpsertPermissionRule(UpsertPermissionRuleParams),
    DeletePermissionRule(DeletePermissionRuleParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    Workspace(WorkspaceResource),
    WorkspaceDeleted { id: i64 },
    Session(SessionResource),
    SessionDeleted { id: i64 },
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
pub struct CancelTurnParams {
    pub session_id: i64,
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
    pub action_key: String,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePermissionRuleParams {
    pub rule_id: i64,
}
