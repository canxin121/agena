//! Runtime-facing read operations over stable session representations.
//!
//! Full session/message/event materialization remains adapter-owned; this port
//! is intentionally restricted to values that can cross the boundary without
//! exposing those core implementation types.

use async_trait::async_trait;

use agena_domain::{
    PendingInteractiveRequestContext, PermissionConfig, SessionCostSummary, SessionSummary,
    SessionUsage, SubtaskStatus, UsageStats, UsageStatsQuery, WorkflowState,
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
    pub task_id: Option<String>,
    pub subtask_status: Option<SubtaskStatus>,
    pub subtask_started_at: Option<DateTime<Utc>>,
    pub subtask_finished_at: Option<DateTime<Utc>>,
    pub subtask_failure: Option<agena_failure::Failure>,
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

    async fn read_part_window(
        &self,
        session_id: i64,
        before: Option<agena_storage::store::PartCursor>,
        limit: i64,
    ) -> Result<agena_storage::store::SessionPartPage, SessionQueryError>;
    async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, SessionQueryError>;

    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, SessionQueryError>;

    async fn latest_event_seq(&self, session_id: i64) -> Result<Option<i64>, SessionQueryError>;

    async fn session_usage(&self, session_id: i64) -> Result<SessionUsage, SessionQueryError>;

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<SessionCostSummary, SessionQueryError>;

    async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, SessionQueryError>;

    async fn pending_interactive_requests(
        &self,
        session_id: i64,
    ) -> Result<Vec<PendingInteractiveRequestContext>, SessionQueryError>;

    async fn execution_context(
        &self,
        session_id: i64,
    ) -> Result<SessionExecutionContext, SessionQueryError>;

    /// Whether `descendant_id` is a strict descendant of `ancestor_id` in the
    /// persisted session lineage.
    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, SessionQueryError>;
}

#[cfg(test)]
mod tests {
    use super::{SessionQueryError, SessionQueryService};

    struct FakeQueries;

    #[async_trait::async_trait]
    impl SessionQueryService for FakeQueries {
        async fn list_session_summaries(
            &self,
            _request: agena_domain::SessionListRequest,
        ) -> Result<Vec<agena_domain::SessionSummary>, SessionQueryError> {
            Ok(Vec::new())
        }

        async fn session_presentation(
            &self,
            session_id: i64,
        ) -> Result<super::SessionPresentation, SessionQueryError> {
            Ok(super::SessionPresentation {
                id: session_id,
                parent_id: None,
                workspace_id: 0,
                title: "test".to_owned(),
                version: 0,
                created_at: chrono::DateTime::UNIX_EPOCH,
                updated_at: chrono::DateTime::UNIX_EPOCH,
                message_count: 0,
                workflow_state: agena_domain::WorkflowState::Quiescent,
            })
        }

        async fn read_part_window(
            &self,
            _session_id: i64,
            _before: Option<agena_storage::store::PartCursor>,
            _limit: i64,
        ) -> Result<agena_storage::store::SessionPartPage, SessionQueryError> {
            Err(SessionQueryError::internal("fixture has no part window"))
        }
        async fn list_session_tree(
            &self,
            _root_id: i64,
        ) -> Result<Vec<agena_domain::SessionSummary>, SessionQueryError> {
            Ok(Vec::new())
        }

        async fn export_session_jsonl(&self, session_id: i64) -> Result<String, SessionQueryError> {
            Ok(format!("{{\"session_id\":{session_id}}}"))
        }

        async fn latest_event_seq(
            &self,
            _session_id: i64,
        ) -> Result<Option<i64>, SessionQueryError> {
            Ok(None)
        }

        async fn session_usage(
            &self,
            _session_id: i64,
        ) -> Result<agena_domain::SessionUsage, SessionQueryError> {
            Ok(agena_domain::SessionUsage {
                measured_prompt_tokens: None,
                current_tokens: 0,
                projected_tokens: None,
                input_limit_tokens: None,
                limit_tokens: None,
                limit_basis: None,
                reserved_tokens: None,
                model_context_window_tokens: None,
                model_max_input_tokens: None,
                model_max_output_tokens: None,
            })
        }

        async fn session_cost_summary(
            &self,
            _session_id: i64,
        ) -> Result<agena_domain::SessionCostSummary, SessionQueryError> {
            Ok(agena_domain::SessionCostSummary::default())
        }

        async fn usage_stats(
            &self,
            _query: agena_domain::UsageStatsQuery,
        ) -> Result<agena_domain::UsageStats, SessionQueryError> {
            Ok(agena_domain::UsageStats {
                generated_at: chrono::Utc::now(),
                period: agena_domain::UsagePeriod::AllTime,
                period_label: "all_time".to_owned(),
                from: None,
                to: None,
                timezone_offset_minutes: 0,
                totals: agena_domain::UsageTotals::default(),
                active_days: 0,
                average_cost_per_request_usd: 0.0,
                average_tokens_per_request: 0.0,
                average_cost_per_active_day_usd: 0.0,
                average_tokens_per_active_day: 0.0,
                peak_cost_date: None,
                peak_cost_usd: 0.0,
                peak_tokens_date: None,
                peak_tokens: 0,
                by_day: Vec::new(),
                by_provider: Vec::new(),
                by_model: Vec::new(),
                by_session: Vec::new(),
            })
        }

        async fn pending_interactive_requests(
            &self,
            _session_id: i64,
        ) -> Result<Vec<agena_domain::PendingInteractiveRequestContext>, SessionQueryError>
        {
            Ok(Vec::new())
        }

        async fn execution_context(
            &self,
            _session_id: i64,
        ) -> Result<super::SessionExecutionContext, SessionQueryError> {
            Ok(super::SessionExecutionContext {
                conversation: None,
                workflow_state: agena_domain::WorkflowState::Quiescent,
                agent_id: agena_runtime_contracts::identity::AGENA_AGENT_ID.to_owned(),
                selected_permission: agena_domain::PermissionConfig::default(),
                effective_permission: agena_domain::PermissionConfig::default(),
                permission_ceiling: agena_domain::PermissionConfig::default(),
                model_provider_id: None,
                model_adapter_id: None,
                model_id: None,
                model_thinking_mode: None,
                model_speed_mode: None,
                model_verbosity: None,
                model_parallel_tool_calls: None,
                effective_workspace_root: None,
                task_id: None,
                subtask_status: None,
                subtask_started_at: None,
                subtask_finished_at: None,
                subtask_failure: None,
            })
        }

        async fn is_descendant_session(
            &self,
            _descendant_id: i64,
            _ancestor_id: i64,
        ) -> Result<bool, SessionQueryError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn trait_object_only_exposes_stable_query_results() {
        let service: &dyn SessionQueryService = &FakeQueries;
        assert!(service.list_session_tree(7).await.expect("tree").is_empty());
        assert_eq!(
            service.export_session_jsonl(7).await.expect("export"),
            "{\"session_id\":7}"
        );
    }
}
