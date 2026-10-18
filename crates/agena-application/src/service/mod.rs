//! Application services: use cases that orchestrate the runtime.

use std::{
    collections::HashMap,
    fmt::Display,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_domain::PermissionScope;
use agena_domain::{
    AdapterId, ModelRef, ModelSpeedModeRequestOverride, PermissionAction, PermissionMode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena_storage::{
    MemoryError, MemoryRecord, NewMemory, PermissionRuleBatchScope, PermissionRuleRepository,
    PersistedPermissionRule, WorkspaceRepository,
};

use crate::{
    ApplicationError,
    dto::{
        ActiveExecutionResource, GitCommitRequest, GitCommitResource, GitPullRequestCreateRequest,
        GitPullRequestResource, GitStageRequest, GitStatusResource, MemoryResource,
        MemoryWriteRequest, PendingInteractiveRequestResource, PermissionRuleBatchOperation,
        PermissionRuleBatchRequest, PermissionRuleBatchResponse, PermissionRuleResource,
        PermissionRuleWriteRequest, ScheduledJobResource, ScheduledJobRunResource,
        SearchPaginationQuery, SessionAutomationResource, SessionCreateRequest,
        SessionExecutionContextResource, SessionExecutionResource, SessionResource,
        SessionRunOptionsRequest, SessionState, SessionUpdateRequest, SessionUsageResource,
        WorkspaceFileDownloadQuery, WorkspaceFileKind, WorkspaceFileNode, WorkspaceFileTreeQuery,
        WorkspaceFileTreeResource, WorkspaceFileUploadRequest, WorkspaceFileUploadResource,
        WorkspaceListQuery, WorkspacePathRequest, WorkspaceResolveRequest, WorkspaceResource,
    },
    pagination::{
        PageInfo, PageOrder, PaginatedResponse, decode_cursor, encode_cursor, normalize_limit,
    },
};

type ApplicationResult<T> = Result<T, ApplicationError>;

pub(crate) mod execution;
mod git;
mod memory;
mod permissions;
pub(crate) mod sessions;
mod workspaces;

pub use execution::{
    list_scheduled_jobs, permission_config_domain_from_resource,
    permission_config_resource_from_domain, scheduled_job_resource, sort_jobs_for_display,
};

/// Maps wire `ListActivitiesParams` to the domain activity filter. HTTP and
/// WS/IPC transports both funnel through this so CSV kind/status parsing stays
/// identical regardless of transport.
pub fn activity_filter_from_params(
    params: &agena_api::queries::ListActivitiesParams,
) -> agena_domain::BackgroundActivityFilter {
    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};
    let parse_kinds = |value: &Option<String>| -> Vec<BackgroundActivityKind> {
        value.as_deref().map_or_else(Vec::new, |csv| {
            csv.split(',')
                .filter_map(|raw| raw.trim().parse::<BackgroundActivityKind>().ok())
                .collect()
        })
    };
    let parse_statuses = |value: &Option<String>| -> Vec<BackgroundActivityStatus> {
        value.as_deref().map_or_else(Vec::new, |csv| {
            csv.split(',')
                .filter_map(|raw| raw.trim().parse::<BackgroundActivityStatus>().ok())
                .collect()
        })
    };
    agena_domain::BackgroundActivityFilter {
        kinds: parse_kinds(&params.kinds),
        statuses: parse_statuses(&params.statuses),
        session_id: params.session_id,
        active_only: params.active_only,
    }
}

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
/// Application service implementing the runtime API surface on top of domain and storage layers.
pub struct ApplicationService {
    workspace_root: String,
    memory_repository: Arc<dyn agena_storage::MemoryRepository>,
    workspace_repository: Arc<dyn WorkspaceRepository>,
    permission_rule_repository: Arc<dyn PermissionRuleRepository>,
    session_kernel: agena_storage::kernel::SessionKernel,
}

impl ApplicationService {
    pub fn session_kernel(&self) -> &agena_storage::kernel::SessionKernel {
        &self.session_kernel
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WorkspaceCursor {
    updated_at_ms: i64,
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
        memory_repository: Arc<dyn agena_storage::MemoryRepository>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        permission_rule_repository: Arc<dyn PermissionRuleRepository>,
        session_kernel: agena_storage::kernel::SessionKernel,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            memory_repository,
            workspace_repository,
            permission_rule_repository,
            session_kernel,
        }
    }

    async fn ensure_workspace_exists(&self, workspace_id: i64) -> ApplicationResult<()> {
        let exists = self
            .workspace_repository
            .path_by_id(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {workspace_id}"),
            ))
        }
    }

    async fn ensure_session_exists(&self, session_id: i64) -> ApplicationResult<()> {
        self.ensure_session_model(session_id).await.map(|_| ())
    }

    async fn ensure_session_model(
        &self,
        session_id: i64,
    ) -> ApplicationResult<agena_storage::kernel::ReadView<agena_storage::kernel::SessionSummaryView>>
    {
        self.session_kernel
            .reader
            .summary(crate::kernel_session_id(session_id)?)
            .await
            .map_err(crate::kernel_error)
    }

    async fn workspace_id_by_path(&self, path: &str) -> ApplicationResult<Option<i64>> {
        self.workspace_repository
            .lookup_id(path)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))
    }

    async fn workspace_session_counts(
        &self,
        workspace_ids: &[i64],
    ) -> ApplicationResult<HashMap<i64, u64>> {
        let stats = self
            .session_kernel
            .reader
            .workspace_statistics(workspace_ids)
            .await
            .map_err(crate::kernel_error)?;
        Ok(stats
            .iter()
            .map(|(id, statistics)| (*id, statistics.total))
            .collect())
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
        total: None,
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
    let limit = usize::try_from(limit).map_err(|error| {
        ApplicationError::bad_request_with_diagnostic(
            "The requested page size is too large.",
            format!("page limit {limit} cannot be represented: {error}"),
        )
    })?;
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
