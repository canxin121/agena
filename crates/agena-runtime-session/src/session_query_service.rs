//! Runtime-facing read operations over stable session representations.
//!
//! Full session/message/event materialization remains adapter-owned; this port
//! is intentionally restricted to values that can cross the boundary without
//! exposing those core implementation types.

use std::collections::BTreeMap;

use async_trait::async_trait;
use thiserror::Error;

use agena_domain::{
    PendingInteractiveRequestContext, PermissionConfig, SessionCostSummary, SessionSummary,
    SessionUsage, SubtaskStatus, UsageStats, UsageStatsQuery, WorkflowState,
};
use chrono::{DateTime, Utc};

/// Stable session-level presentation fields for consumers that do not need a
/// concrete transcript aggregate.
#[derive(Debug, Clone)]
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
pub struct SessionProjectedMessageHeader {
    pub id: i64,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub usage: Option<serde_json::Value>,
    pub part_count: u64,
}

/// Stable transcript projection for presentation paths that need message-part
/// summaries without depending on private message aggregates. Detail payloads
/// remain opaque JSON until the full transcript detail contract moves.
#[derive(Debug, Clone)]
pub struct SessionProjectedMessage {
    pub id: i64,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub usage: Option<serde_json::Value>,
    pub parts: Vec<SessionProjectedMessagePart>,
}

#[derive(Debug, Clone)]
pub struct SessionProjectedMessagePart {
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: agena_domain::ExecutionStatus,
    pub kind: agena_domain::PartKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub has_detail: bool,
    pub activity_id: Option<agena_domain::ActivityId>,
    pub segment_id: Option<agena_domain::ResponseSegmentId>,
    pub operation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub detail: Option<SessionProjectedPartDetail>,
    pub content: Option<serde_json::Value>,
}

/// A stable, runtime-owned projection of a recorded tool operation. The
/// persisted Runtime aggregate is adapted to this value; consumers must not use
/// its JSON serialization as an API contract.
#[derive(Debug, Clone)]
pub struct SessionProjectedOperationPart {
    pub call_id: i64,
    pub invocation: agena_domain::ToolInvocation,
    pub title: String,
    pub summary: String,
    pub model_output: SessionProjectedModelVisibleOutput,
    pub blocks: Vec<SessionProjectedOperationBlock>,
    pub artifacts: Vec<agena_domain::ArtifactRef>,
    pub attachments: Vec<agena_plugin_host::sdk::attachment::AttachmentItem>,
    pub details: agena_domain::ToolOutput,
    pub result: SessionProjectedToolResult,
    pub structured: Option<serde_json::Value>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub error: Option<agena_domain::OperationError>,
    pub raw: Option<serde_json::Value>,
    pub lifecycle: agena_domain::TimeRange,
}

#[derive(Debug, Clone, Default)]
pub struct SessionProjectedModelVisibleOutput {
    pub text: String,
    pub attachments: Vec<agena_plugin_host::sdk::attachment::AttachmentItem>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionProjectedToolResult {
    pub state: agena_domain::ToolResultState,
    pub structured: Option<serde_json::Value>,
    pub content: Vec<SessionProjectedOperationBlock>,
    pub model_preview: SessionProjectedModelVisibleOutput,
    pub managed_outputs: Vec<agena_domain::ToolManagedOutput>,
    pub display: agena_domain::ToolResultDisplay,
    pub attachments: Vec<agena_plugin_host::sdk::attachment::AttachmentItem>,
    pub error: Option<agena_domain::OperationError>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub raw: Option<serde_json::Value>,
}

/// Presentation blocks emitted by tools. This is deliberately exhaustive so
/// transcript consumers can map each value to their own protocol without
/// depending on Runtime's private message implementation.
#[derive(Debug, Clone)]
pub enum SessionProjectedOperationBlock {
    Text {
        text: String,
    },
    Markdown {
        text: String,
    },
    Json {
        value: serde_json::Value,
    },
    Table {
        columns: Vec<agena_domain::TableColumn>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    Log {
        stream: Option<String>,
        text: String,
    },
    Command {
        command: String,
        cwd: Option<String>,
        exit_code: Option<i32>,
        stdout: Option<String>,
        stderr: Option<String>,
    },
    Diff {
        diff: String,
        language: Option<String>,
    },
    FileChanges {
        changes: Vec<agena_domain::FileChangeRecord>,
    },
    SearchResults {
        query: Option<String>,
        results: Vec<agena_domain::SearchResultItem>,
    },
    Citation {
        uri: String,
        title: Option<String>,
        snippet: Option<String>,
    },
    Image {
        mime: String,
        url: String,
    },
    Audio {
        mime: String,
        url: String,
    },
    ResourceLink {
        uri: String,
        title: Option<String>,
        mime_type: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        mime: String,
        text: Option<String>,
        base64: Option<String>,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
    Media {
        mime_type: String,
        artifact: agena_domain::ArtifactRef,
    },
    Checklist {
        items: Vec<agena_domain::TodoItem>,
    },
    NestedTask {
        task_id: String,
        title: Option<String>,
        status: agena_domain::ExecutionStatus,
    },
    Progress {
        message: String,
        percent: Option<f32>,
    },
    Custom {
        schema: Option<String>,
        value: serde_json::Value,
    },
}

/// Typed detail values that are already stable outside Runtime's persisted
/// aggregate. `Opaque` remains solely for legacy/missing persisted content;
/// every current message-part variant has an explicit projection.
#[derive(Debug, Clone)]
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
        code: String,
        message: String,
    },
    Attachment(agena_plugin_host::sdk::attachment::AttachmentPart),
    SkillReference(crate::message::SkillReferencePart),
    PermissionRequest {
        request: agena_domain::PermissionRequest,
        reply: Option<agena_domain::PermissionReply>,
    },
    UserInputRequest {
        request: agena_domain::UserInputRequest,
        reply: Option<agena_domain::UserInputReply>,
    },
    Operation(Box<SessionProjectedOperationPart>),
    Opaque(serde_json::Value),
}

/// Stable execution-state projection needed by application presentation.
/// Runtime retains session/message persistence and lifecycle materialization.
#[derive(Debug, Clone)]
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
    pub subtask_error: Option<String>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("session query failed: {message}")]
pub struct SessionQueryError {
    message: String,
}

impl SessionQueryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Read-only session capabilities with stable result types.
#[async_trait]
pub trait SessionQueryService: Send + Sync {
    /// Resolve a persisted projected message to its owning session without
    /// exposing the concrete transcript repository.
    async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, SessionQueryError>;

    async fn find_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, SessionQueryError>;

    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<SessionSummary>, SessionQueryError>;

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<SessionPresentation, SessionQueryError>;

    async fn transcript_snapshot(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::TranscriptSnapshot, SessionQueryError> {
        Ok(agena_domain::TranscriptSnapshot {
            session_id,
            ..Default::default()
        })
    }

    async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionProjectedMessageHeader>, SessionQueryError>;

    async fn list_projected_messages(
        &self,
        session_id: i64,
        include_content: bool,
    ) -> Result<Vec<SessionProjectedMessage>, SessionQueryError>;
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
        async fn find_session_id_for_message(
            &self,
            _message_id: i64,
        ) -> Result<Option<i64>, SessionQueryError> {
            Ok(None)
        }
        async fn find_session_id_for_part(
            &self,
            _part_id: i64,
        ) -> Result<Option<i64>, SessionQueryError> {
            Ok(None)
        }

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

        async fn list_projected_message_headers(
            &self,
            _session_id: i64,
        ) -> Result<Vec<super::SessionProjectedMessageHeader>, SessionQueryError> {
            Ok(Vec::new())
        }
        async fn list_projected_messages(
            &self,
            _session_id: i64,
            _include_content: bool,
        ) -> Result<Vec<super::SessionProjectedMessage>, SessionQueryError> {
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
                subtask_error: None,
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
