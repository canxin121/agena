use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait,
    FromQueryResult, QueryFilter, QuerySelect,
};
use serde::{Deserialize, Serialize};

use agena::{
    AppError,
    db::{
        crud::{
            permission_rule as permission_rule_crud, session as session_crud,
            workspace as workspace_crud,
        },
        entities,
    },
    event::{EventKind, EventPublisher, PermissionRuleEvent, PublishContext},
    message::{Message, UserInputRequest},
    model::{AdapterId, ModelRef, ModelSpeedModeRequestOverride},
    permission::{PermissionAction, PermissionMode, PermissionScope, PersistedPermissionRule},
    provider::ProviderRegistry,
    session::{Session, SessionManager},
};

use super::{
    dto::{
        CursorPaginationQuery, GitStatusResource, MessageListQuery, MessageResource, PartLoadMode,
        PermissionRuleResource, PermissionRuleWriteRequest, ScheduledJobResource,
        ScheduledJobRunResource, SearchPaginationQuery, SessionAutomationResource,
        SessionCreateRequest, SessionExecutionContextResource, SessionExecutionResource,
        SessionHierarchyRequest, SessionResource, SessionRunOptionsRequest, SessionRunState,
        SessionUsageResource, WorkspaceFileKind, WorkspaceFileNode, WorkspaceFileTreeQuery,
        WorkspaceFileTreeResource, WorkspaceListQuery, WorkspacePathRequest,
        WorkspaceResolveRequest, WorkspaceResource,
    },
    error::ApiError,
    pagination::{
        PageInfo, PageOrder, PaginatedResponse, decode_cursor, encode_cursor, normalize_limit,
    },
};

type ApiResult<T> = Result<T, ApiError>;

mod execution;
mod git;
mod messages;
mod permissions;
mod sessions;
mod workspaces;

pub use execution::{list_scheduled_jobs, scheduled_job_resource, sort_jobs_for_display};

#[derive(Clone)]
pub struct ApiService {
    db: Arc<DatabaseConnection>,
    workspace_root: String,
    publisher: Option<Arc<EventPublisher>>,
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

#[derive(Debug, Clone, FromQueryResult)]
struct WorkspaceSessionCountRow {
    workspace_id: i64,
    session_count: i64,
}

impl ApiService {
    pub fn new(
        db: Arc<DatabaseConnection>,
        workspace_root: impl Into<String>,
        publisher: Option<Arc<EventPublisher>>,
    ) -> Self {
        Self {
            db,
            workspace_root: workspace_root.into(),
            publisher,
        }
    }

    pub fn clone_db(&self) -> Arc<DatabaseConnection> {
        Arc::clone(&self.db)
    }

    async fn ensure_workspace_exists(&self, workspace_id: i64) -> ApiResult<()> {
        let exists = entities::workspace::Entity::find_by_id(workspace_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(ApiError::not_found(format!(
                "workspace not found: {workspace_id}"
            )))
        }
    }

    async fn ensure_session_exists(&self, session_id: i64) -> ApiResult<()> {
        self.ensure_session_model(session_id).await.map(|_| ())
    }

    async fn ensure_session_model(&self, session_id: i64) -> ApiResult<entities::session::Model> {
        entities::session::Entity::find_by_id(session_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found(format!("session not found: {session_id}")))
    }

    async fn workspace_id_by_path(&self, path: &str) -> ApiResult<Option<i64>> {
        agena::db::crud::workspace::get_workspace_id_by_path(self.db.as_ref(), path)
            .await
            .map_err(db_error)
    }

    async fn workspace_session_counts(
        &self,
        workspace_ids: &[i64],
    ) -> ApiResult<HashMap<i64, u64>> {
        if workspace_ids.is_empty() {
            return Ok(HashMap::new());
        }

        entities::session::Entity::find()
            .select_only()
            .column_as(entities::session::Column::WorkspaceId, "workspace_id")
            .column_as(entities::session::Column::Id.count(), "session_count")
            .filter(entities::session::Column::WorkspaceId.is_in(workspace_ids.iter().copied()))
            .group_by(entities::session::Column::WorkspaceId)
            .into_model::<WorkspaceSessionCountRow>()
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.workspace_id,
                    u64::try_from(row.session_count).map_err(|_| {
                        ApiError::internal(format!(
                            "invalid negative session count for workspace {}",
                            row.workspace_id
                        ))
                    })?,
                ))
            })
            .collect()
    }
}

fn api_error_from_app(error: AppError) -> ApiError {
    ApiError::from(error)
}

fn build_page<T, C>(
    items: Vec<T>,
    has_more: bool,
    next_cursor: Option<C>,
    order: PageOrder,
    limit: u64,
) -> ApiResult<PaginatedResponse<T>>
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

fn trim_page<T>(mut rows: Vec<T>, limit: u64) -> ApiResult<(Vec<T>, bool)> {
    let limit = usize::try_from(limit)
        .map_err(|_| ApiError::bad_request(format!("page limit too large: {limit}")))?;
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

fn timestamp_millis_to_utc(timestamp_ms: i64) -> ApiResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| ApiError::internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn db_error(error: DbErr) -> ApiError {
    ApiError::from(AppError::Database(error))
}
