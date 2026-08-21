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

#[derive(Debug, Clone)]
/// Header of a projected run.
pub struct SessionProjectedRunHeader {
    pub id: i64,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub usage: Option<serde_json::Value>,
    pub part_count: u64,
}

/// Stable transcript projection for presentation paths that need run-part
/// summaries without depending on private run aggregates. Detail payloads
/// remain opaque JSON until the full transcript detail contract moves.
#[derive(Debug, Clone)]
/// A projected session run.
pub struct SessionProjectedRun {
    pub id: i64,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub usage: Option<serde_json::Value>,
    pub parts: Vec<SessionProjectedPart>,
}

#[derive(Debug, Clone)]
/// A projected run part.
pub struct SessionProjectedPart {
    pub id: i64,
    pub run_id: i64,
    pub part_index: i32,
    pub status: agena_domain::ExecutionStatus,
    /// The precise v2 part kind (`text`, `think`, `tool_call`, ...). The
    /// v1 `PartKind` binary (Text/Activity) is gone: the transcript surfaces
    /// dispatch on this exact kind, so it must round-trip the storage column.
    pub kind: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub has_detail: bool,
    pub activity_id: Option<agena_domain::ActivityId>,
    pub segment_id: Option<agena_domain::TextSegmentId>,
    pub operation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub detail: Option<SessionProjectedPartDetail>,
    pub content: Option<serde_json::Value>,
}

/// A stable, runtime-owned projection of a recorded hook run. Hook activity
/// (for example the workflow plan's `agent.stop` autorun continuation) rides
/// the same transcript pipeline as tool calls.
#[derive(Debug, Clone)]
/// A projected hook part.
pub struct SessionProjectedHookPart {
    pub hook: String,
    pub plugin_id: Option<String>,
    pub summary: String,
    pub detail: Option<String>,
    /// The message the hook sent to keep the run going, when it blocked the
    /// stop. Carried by the hook activity, never injected as a separate
    /// assistant message.
    pub message: Option<String>,
}

/// Typed detail values that are already stable outside Runtime's persisted
/// aggregate. Every current message-part variant has an explicit projection.
#[derive(Debug, Clone)]
/// Detail kind of a projected part.
pub enum SessionProjectedPartDetail {
    Text {
        text: String,
        synthetic: bool,
    },
    Reasoning {
        summary: Vec<String>,
        raw_content: Vec<String>,
        encrypted_content: Option<String>,
    },
    Error {
        problem: agena_failure::UserProblem,
    },
    Attachment(agena_plugin_host::sdk::attachment::AttachmentPart),
    SkillReference(crate::part::SkillReferencePart),
    UserInputRequest {
        request: agena_domain::UserInputRequest,
        reply: Option<agena_domain::UserInputReply>,
    },
    ToolCall(agena_runtime_contracts::part_content::ToolCallContent),
    Hook(Box<SessionProjectedHookPart>),
    Notice {
        summary: String,
        detail: Option<String>,
    },
    /// A background-operation completion/event notification (the agena analog
    /// of Claude's `<task-notification>`).
    SystemNotification {
        operation_id: String,
        operation_kind: String,
        status: String,
        summary: String,
        detail: Option<String>,
        body: String,
        event_seq: Option<u64>,
    },
    Opaque(serde_json::Value),
}

/// Stable execution-state projection needed by application presentation.
/// Runtime retains session/message persistence and lifecycle materialization.
#[derive(Debug, Clone)]
/// Execution context of a session.
pub struct SessionExecutionContext {
    pub workflow_state: WorkflowState,
    pub agent_id: String,
    pub execution_access: agena_domain::ExecutionAccess,
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

    async fn list_projected_runs(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionProjectedRun>, SessionQueryError>;
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

        async fn list_projected_runs(
            &self,
            _session_id: i64,
        ) -> Result<Vec<super::SessionProjectedRun>, SessionQueryError> {
            Ok(Vec::new())
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
                average_cost_per_run_usd: 0.0,
                average_tokens_per_run: 0.0,
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
                workflow_state: agena_domain::WorkflowState::Quiescent,
                agent_id: agena_runtime_contracts::identity::AGENA_AGENT_ID.to_owned(),
                execution_access: agena_domain::ExecutionAccess::Inherit,
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
