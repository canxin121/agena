//! Session read/write surface for plugins.
//!
//! A plugin that serves a chat platform must be able to do what the built-in
//! web and TUI clients do: read a transcript, submit a message (including rich
//! `ComposerDocument` content), cancel work, manage the queue, answer a
//! permission card. These types are the plugin-facing shape of that surface.
//!
//! The host still owns authorization. A plugin receives a scoped client bound
//! to one plugin instance; every call is checked against the sessions that
//! instance is allowed to touch. A plugin cannot widen its own scope by
//! naming an id, and it cannot mint [`AuthorKind::InstanceOperator`] for an
//! external speaker — the host fills the author from the verified context.
//!
//! [`AuthorKind::InstanceOperator`]: agena_domain::AuthorKind::InstanceOperator

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "request", rename_all = "snake_case")]
/// One session operation a plugin may run.
///
/// These wrap the shared `agena_session_api` enums rather than re-declaring
/// them, so plugin reach matches the REST and WebSocket surfaces exactly.
pub enum SessionOp {
    Query(agena_session_api::queries::Query),
    Command(agena_session_api::commands::Command),
}

impl SessionOp {
    /// Whether a plugin may run this operation.
    ///
    /// Plugins act for a conversation, never for the operator: instance-wide
    /// destruction and permission-policy writes stay on the local surfaces.
    /// The rules live on the shared enums, so the Web and plugin surfaces
    /// cannot disagree about what a plugin may do.
    pub fn plugin_allowed(&self) -> Result<(), &'static str> {
        match self {
            SessionOp::Query(
                agena_session_api::queries::Query::Runtime
                | agena_session_api::queries::Query::ListProviders
                | agena_session_api::queries::Query::ListProviderModels(_)
                | agena_session_api::queries::Query::ListProviderAdapterModels(_)
                | agena_session_api::queries::Query::ListSavedProviderAdapterModels(_),
            ) => Err("runtime, provider and model inspection is operator-only; \
                 plugins read sessions through the session queries"),
            SessionOp::Query(_) => Ok(()),
            SessionOp::Command(command) => command.plugin_allowed(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "response", rename_all = "snake_case")]
/// The answer to one [`SessionOp`].
///
/// The payload is the same resource the HTTP transport returns for that
/// operation, wrapped in that transport's result envelope.
pub enum SessionOpResponse {
    Query(Value),
    Command(Value),
}

/// Which live stream to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSessionStreamKind {
    /// Ordered transcript changes: parts added, updated or removed.
    Parts,
    /// Session metadata, executions and runtime signals.
    Notifications,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionStreamRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub kind: HostSessionStreamKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Durable control facts for one session.
pub struct HostSessionState {
    pub session: crate::host_api::HostSession,
    pub running: bool,
    pub queue_paused: bool,
    pub queued_messages: Vec<HostQueuedMessage>,
    /// Durable session control version; orders snapshots and receipts.
    pub control_version: i64,
    pub workflow_state: String,
    pub selected_permission: Value,
    pub effective_permission: Value,
    pub permission_ceiling: Value,
    pub model: HostSessionModel,
    pub workspace_root: Option<String>,
    pub agent: HostSessionAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// One accepted input that has not started.
pub struct HostQueuedMessage {
    pub execution_id: String,
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Effective model selection of one session.
pub struct HostSessionModel {
    pub provider_id: Option<String>,
    pub adapter_id: Option<String>,
    pub model_id: Option<String>,
    pub thinking_mode: Option<String>,
    pub speed_mode: Option<String>,
    pub verbosity: Option<String>,
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Agent identity and terminal state of one session, when it is a subagent.
pub struct HostSessionAgent {
    pub id: String,
    pub name: Option<String>,
    pub status: Option<agena_domain::AgentStatus>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub failure: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// One execution summary, mirroring the HTTP `RunSummaryResource`.
pub struct HostExecutionSummary {
    pub execution_id: i64,
    pub part_count: u64,
    pub loaded_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Bounded execution list of one session.
pub struct HostExecutionList {
    pub executions: Vec<HostExecutionSummary>,
    pub user_message_count: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// One ordered transcript page.
pub struct HostPartsPage {
    pub entries: Value,
    pub user_message_count: u64,
    pub has_more: bool,
}
