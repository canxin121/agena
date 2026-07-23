use std::{
    collections::HashMap,
    fmt::Display,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use agena_domain::PermissionScope;
use agena_domain::{
    AdapterId, ModelRef, ModelSpeedModeRequestOverride, PermissionAction, PermissionMode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena_domain::PermissionRuleEvent;
use agena_storage::{
    MemoryError, MemoryRecord, NewMemory, PermissionRuleRepository, PersistedPermissionRule,
    SessionMutationRepository, SessionStatsRepository, SessionSummaryRepository,
    WorkspaceRepository,
};

use crate::{
    ApplicationError,
    dto::{
        ActiveExecutionResource, ActiveSnapshotResource, CursorPaginationQuery, GitCommitRequest,
        GitCommitResource, GitPullRequestCreateRequest, GitPullRequestResource, GitStageRequest,
        GitStatusResource, ManagedSnapshotResource, MemoryResource, MemoryWriteRequest,
        MessageListQuery, MessageResource, PartLoadMode, PendingInteractiveRequestResource,
        PermissionRuleResource, PermissionRuleWriteRequest, ScheduledJobResource,
        ScheduledJobRunResource, SearchPaginationQuery, SessionAutomationResource,
        SessionCreateRequest, SessionExecutionContextResource, SessionExecutionResource,
        SessionLifecycleState, SessionRelationKind, SessionResource, SessionRunOptionsRequest,
        SessionUpdateRequest, SessionUsageResource, SnapshotBackendSupportResource,
        SnapshotStatusResource, SubtaskStatus, WorkspaceFileDownloadQuery, WorkspaceFileKind,
        WorkspaceFileNode, WorkspaceFileTreeQuery, WorkspaceFileTreeResource, WorkspaceListQuery,
        WorkspacePathRequest, WorkspaceResolveRequest, WorkspaceResource,
    },
    pagination::{
        PageInfo, PageOrder, PaginatedResponse, decode_cursor, encode_cursor, normalize_limit,
    },
};

type ApplicationResult<T> = Result<T, ApplicationError>;

pub(crate) mod execution;
mod git;
mod memory;
mod messages;
mod permissions;
pub(crate) mod sessions;
mod workspaces;

pub(crate) use execution::permission_config_resource_from_domain;
pub use execution::{list_scheduled_jobs, scheduled_job_resource, sort_jobs_for_display};
pub use messages::message_part_resource_from_runtime;

/// Transport-neutral permission-rule mutation request.
///
/// HTTP handlers map their wire DTO at the transport edge; CLI and other
/// in-process consumers construct this application command directly. The
/// actor fields are system-provided audit provenance, never caller-provided
/// wire input.
#[derive(Debug, Clone)]
pub struct PermissionRuleWriteCommand {
    pub action: PermissionAction,
    pub mode: PermissionMode,
    pub scope: PermissionScope,
    pub session_id: Option<i64>,
    pub source: String,
    pub operator: Option<String>,
}

#[derive(Clone)]
pub struct ApplicationService {
    workspace_root: String,
    publisher: Option<Arc<dyn agena_runtime::RuntimeEventPublishService>>,
    memory_repository: Arc<dyn agena_storage::MemoryRepository>,
    workspace_repository: Arc<dyn WorkspaceRepository>,
    permission_rule_repository: Arc<dyn PermissionRuleRepository>,
    session_stats_repository: Arc<dyn SessionStatsRepository>,
    session_summary_repository: Arc<dyn SessionSummaryRepository>,
    session_mutation_repository: Arc<dyn SessionMutationRepository>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WorkspaceCursor {
    updated_at_ms: i64,
    id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SessionCursor {
    updated_at_ms: i64,
    id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct MessageCursor {
    created_at_ms: i64,
    id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct EventCursor {
    seq: i64,
    id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct PermissionRuleCursor {
    updated_at_ms: i64,
    id: i64,
}

impl ApplicationService {
    pub fn new(
        workspace_root: impl Into<String>,
        publisher: Option<Arc<dyn agena_runtime::RuntimeEventPublishService>>,
        memory_repository: Arc<dyn agena_storage::MemoryRepository>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        permission_rule_repository: Arc<dyn PermissionRuleRepository>,
        session_stats_repository: Arc<dyn SessionStatsRepository>,
        session_summary_repository: Arc<dyn SessionSummaryRepository>,
        session_mutation_repository: Arc<dyn SessionMutationRepository>,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            publisher,
            memory_repository,
            workspace_repository,
            permission_rule_repository,
            session_stats_repository,
            session_summary_repository,
            session_mutation_repository,
        }
    }

    async fn ensure_workspace_exists(&self, workspace_id: i64) -> ApplicationResult<()> {
        let exists = self
            .workspace_repository
            .path_by_id(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(ApplicationError::not_found(format!(
                "workspace not found: {workspace_id}"
            )))
        }
    }

    async fn ensure_session_exists(&self, session_id: i64) -> ApplicationResult<()> {
        self.ensure_session_model(session_id).await.map(|_| ())
    }

    async fn ensure_session_model(
        &self,
        session_id: i64,
    ) -> ApplicationResult<agena_storage::SessionSummaryRecord> {
        let record = self
            .session_summary_repository
            .get(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .ok_or_else(|| {
                ApplicationError::not_found(format!("session not found: {session_id}"))
            })?;
        if record.lifecycle_state != agena_domain::SessionLifecycleState::Ready {
            return Err(ApplicationError::not_found(format!(
                "session not found: {session_id}"
            )));
        }
        Ok(record)
    }

    async fn workspace_id_by_path(&self, path: &str) -> ApplicationResult<Option<i64>> {
        self.workspace_repository
            .lookup_id(path)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))
    }

    async fn workspace_session_counts(
        &self,
        workspace_ids: &[i64],
    ) -> ApplicationResult<HashMap<i64, u64>> {
        self.session_stats_repository
            .workspace_counts(workspace_ids)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .into_iter()
            .map(|(workspace_id, count)| {
                Ok((
                    workspace_id,
                    u64::try_from(count).map_err(|_| {
                        ApplicationError::internal(format!(
                            "invalid negative session count for workspace {workspace_id}"
                        ))
                    })?,
                ))
            })
            .collect()
    }
}

fn api_error_from_app(error: impl Display) -> ApplicationError {
    ApplicationError::internal(error.to_string())
}

fn build_page<T, C>(
    items: Vec<T>,
    has_more: bool,
    next_cursor: Option<C>,
    order: PageOrder,
    limit: u64,
) -> ApplicationResult<PaginatedResponse<T>>
where
    C: Serialize,
{
    Ok(PaginatedResponse {
        page: PageInfo {
            limit,
            returned: items.len(),
            has_more,
            next_cursor: next_cursor
                .map(|cursor| encode_cursor(&cursor))
                .transpose()?,
            order,
        },
        items,
    })
}

fn trim_page<T>(mut rows: Vec<T>, limit: u64) -> ApplicationResult<(Vec<T>, bool)> {
    let limit = usize::try_from(limit)
        .map_err(|_| ApplicationError::bad_request(format!("page limit too large: {limit}")))?;
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    Ok((rows, has_more))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> ApplicationResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        ApplicationError::internal(format!("invalid timestamp millis: {timestamp_ms}"))
    })
}
