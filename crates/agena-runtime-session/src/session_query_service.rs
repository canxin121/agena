//! Runtime-facing read operations over stable session representations.
//!
//! Full session/message/event materialization remains adapter-owned; this port
//! is intentionally restricted to values that can cross the boundary without
//! exposing those core implementation types.

use async_trait::async_trait;

use agena_domain::{
    AgentStatus, PendingInteractiveRequestContext, PermissionConfig, SessionCostSummary,
    SessionSummary, SessionUsage, UsageStats, UsageStatsQuery, WorkflowState,
};
use chrono::{DateTime, Utc};

/// Stable session-level presentation fields for consumers that do not need a
/// concrete transcript aggregate.
#[derive(Debug, Clone)]
/// Presentation view of a session.
pub struct SessionPresentation {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    pub workflow_state: WorkflowState,
}

/// Stable execution-state projection needed by application presentation.
/// Runtime retains session/message persistence and lifecycle materialization.
#[derive(Debug, Clone)]
/// Execution context of a session.
pub struct SessionExecutionContext {
    pub conversation: Option<agena_domain::SessionConversation>,
    pub additional_system_prompt: Option<String>,
    pub effective_additional_system_prompt: Option<String>,
    pub workflow_state: WorkflowState,
    pub agent_id: String,
    pub selected_permission: PermissionConfig,
    pub effective_permission: PermissionConfig,
    pub permission_ceiling: PermissionConfig,
    pub model_provider_id: Option<String>,
    pub model_adapter_id: Option<String>,
    pub model_id: Option<String>,
    pub model_thinking_mode: Option<String>,
    pub model_speed_mode: Option<String>,
    pub model_verbosity: Option<String>,
    pub model_parallel_tool_calls: Option<bool>,
    pub effective_workspace_root: Option<String>,
    pub agent_name: Option<String>,
    pub agent_status: Option<AgentStatus>,
    pub agent_started_at: Option<DateTime<Utc>>,
    pub agent_finished_at: Option<DateTime<Utc>>,
    pub agent_failure: Option<agena_failure::Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Error of a session query.
pub struct SessionQueryError {
    pub failure: Box<agena_failure::Failure>,
}

impl SessionQueryError {
    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        Self {
            failure: Box::new(crate::service_failure::unexpected_service_failure(
                "session.query_failed",
                "Session data could not be loaded.",
                diagnostic,
            )),
        }
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }
}

impl std::fmt::Display for SessionQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::service_failure::display_service_failure(&self.failure, formatter)
    }
}

impl std::error::Error for SessionQueryError {}

/// Read-only session capabilities with stable result types.
#[async_trait]
/// Service for querying projected session state.
pub trait SessionQueryService: Send + Sync {
    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<SessionSummary>, SessionQueryError>;

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<SessionPresentation, SessionQueryError>;

    async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, SessionQueryError>;

    /// Stream a complete session archive with bounded memory. The sink owns
    /// any partial output if export fails or the caller cancels it.
    async fn export_session_to(
        &self,
        session_id: i64,
        sink: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<(), SessionQueryError>;

    async fn latest_event_seq(&self, session_id: i64) -> Result<Option<i64>, SessionQueryError>;

    async fn session_usage(&self, session_id: i64) -> Result<SessionUsage, SessionQueryError>;

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<SessionCostSummary, SessionQueryError>;

    async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, SessionQueryError>;

    async fn pending_interactive_requests(
        &self,
        snapshot: &agena_storage::kernel::SessionControlSnapshot,
    ) -> Result<Vec<PendingInteractiveRequestContext>, SessionQueryError>;

    /// Apply the current runtime defaults to one immutable kernel control
    /// snapshot. This projection must not reload persisted session facts.
    fn execution_context(
        &self,
        snapshot: &agena_storage::kernel::SessionControlSnapshot,
    ) -> Result<SessionExecutionContext, SessionQueryError>;

    /// Whether `descendant_id` is a strict descendant of `ancestor_id` in the
    /// persisted session lineage.
    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, SessionQueryError>;
}
