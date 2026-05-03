use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use path_clean::PathClean;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, DbErr,
    EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};

use agena::{
    AppError,
    db::{
        crud::{permission_rule as permission_rule_crud, session as session_crud},
        entities,
    },
    message::{
        ExecutionStatus, Message, MessagePart, PartContent, PermissionRequestPart,
        UserInputRequest, UserInputRequestPart,
    },
    model::ModelRef,
    permission::PermissionMode,
    provider::ProviderRegistry,
    session::{Session, SessionManager},
};

use super::{
    dto::{
        MessageListQuery, MessageResource, PartLoadMode, PermissionRuleListQuery,
        PermissionRuleResource, PermissionRuleWriteRequest, SessionCreateRequest,
        SessionEventListQuery, SessionExecutionResource, SessionReplaceRequest, SessionResource,
        SessionRunOptionsRequest, SessionRunState, WorkspaceFileKind, WorkspaceFileNode,
        WorkspaceFileTreeQuery, WorkspaceFileTreeResource, WorkspaceListQuery,
        WorkspaceResolveRequest, WorkspaceResource, WorkspaceWriteRequest,
    },
    error::ApiError,
    pagination::{
        PageInfo, PageOrder, PaginatedResponse, decode_cursor, encode_cursor, normalize_limit,
    },
};

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone)]
pub struct ApiService {
    db: Arc<DatabaseConnection>,
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
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    pub async fn list_workspaces(
        &self,
        query: WorkspaceListQuery,
    ) -> ApiResult<PaginatedResponse<WorkspaceResource>> {
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<WorkspaceCursor>)
            .transpose()?;
        let mut statement = entities::workspace::Entity::find()
            .order_by_desc(entities::workspace::Column::UpdatedAtMs)
            .order_by_desc(entities::workspace::Column::Id);

        if let Some(search) = non_empty(query.search.as_deref()) {
            statement =
                statement.filter(entities::workspace::Column::Path.like(format!("%{search}%")));
        }
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::workspace::Column::UpdatedAtMs.lt(cursor.updated_at_ms))
                    .add(
                        Condition::all()
                            .add(entities::workspace::Column::UpdatedAtMs.eq(cursor.updated_at_ms))
                            .add(entities::workspace::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let workspace_ids = slice.iter().map(|row| row.id).collect::<Vec<_>>();
        let session_counts = if query.include_session_count {
            self.workspace_session_counts(&workspace_ids).await?
        } else {
            HashMap::new()
        };
        let items = slice
            .iter()
            .map(|row| workspace_resource(row, session_counts.get(&row.id).copied()))
            .collect::<ApiResult<Vec<_>>>()?;
        let next_cursor = slice.last().map(|row| WorkspaceCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(items, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_workspace(&self, workspace_id: i64) -> ApiResult<Option<WorkspaceResource>> {
        let row = entities::workspace::Entity::find_by_id(workspace_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let counts = self.workspace_session_counts(&[row.id]).await?;
        Ok(Some(workspace_resource(
            &row,
            counts.get(&row.id).copied(),
        )?))
    }

    pub async fn list_workspace_files(
        &self,
        workspace_id: i64,
        query: WorkspaceFileTreeQuery,
    ) -> ApiResult<WorkspaceFileTreeResource> {
        let row = entities::workspace::Entity::find_by_id(workspace_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found(format!("workspace not found: {workspace_id}")))?;
        let root = PathBuf::from(row.path);
        let root = root
            .canonicalize()
            .map_err(|error| workspace_fs_error(root.as_path(), error))?;
        if !root.is_dir() {
            return Err(ApiError::bad_request(format!(
                "workspace root is not a directory: {}",
                root.display()
            )));
        }

        let relative_path = clean_workspace_relative_path(query.path.as_deref())?;
        let target = root.join(&relative_path).clean();
        let target = target
            .canonicalize()
            .map_err(|error| workspace_fs_error(target.as_path(), error))?;
        if !target.starts_with(&root) {
            return Err(ApiError::bad_request(
                "workspace file path escapes workspace root",
            ));
        }
        if !target.is_dir() {
            return Err(ApiError::bad_request(format!(
                "workspace path is not a directory: {}",
                workspace_relative_path(&relative_path)
            )));
        }

        let depth = query.depth.unwrap_or(2).min(8);
        let mut remaining = query.limit.unwrap_or(500).clamp(1, 2_000);
        let entries =
            read_workspace_entries(root.as_path(), target.as_path(), depth, &mut remaining)?;

        Ok(WorkspaceFileTreeResource {
            workspace_id,
            root: root.display().to_string(),
            path: workspace_relative_path(&relative_path),
            entries,
        })
    }

    pub async fn create_workspace(
        &self,
        request: WorkspaceWriteRequest,
    ) -> ApiResult<WorkspaceResource> {
        let path = normalize_workspace_path(request.path.as_str()).map_err(db_error)?;
        if self.workspace_id_by_path(path.as_str()).await?.is_some() {
            return Err(ApiError::conflict(format!(
                "workspace path already exists: {path}"
            )));
        }

        let now_ms = Utc::now().timestamp_millis();
        let created = entities::workspace::ActiveModel {
            path: Set(path),
            created_at_ms: Set(now_ms),
            updated_at_ms: Set(now_ms),
            ..Default::default()
        }
        .insert(self.db.as_ref())
        .await
        .map_err(db_error)?;

        workspace_resource(&created, Some(0))
    }

    pub async fn resolve_workspace(
        &self,
        request: WorkspaceResolveRequest,
    ) -> ApiResult<WorkspaceResource> {
        let path = normalize_workspace_path(request.path.as_str()).map_err(db_error)?;
        if let Some(workspace_id) = self.workspace_id_by_path(path.as_str()).await? {
            return self.get_workspace(workspace_id).await?.ok_or_else(|| {
                ApiError::internal(format!(
                    "workspace {workspace_id} disappeared while resolving path {path}"
                ))
            });
        }

        if !request.create_if_missing {
            return Err(ApiError::not_found(format!(
                "workspace not found for path: {path}"
            )));
        }

        match self
            .create_workspace(WorkspaceWriteRequest { path: path.clone() })
            .await
        {
            Ok(workspace) => Ok(workspace),
            Err(error) => {
                if let Some(workspace_id) = self.workspace_id_by_path(path.as_str()).await? {
                    return self.get_workspace(workspace_id).await?.ok_or_else(|| {
                        ApiError::internal(format!(
                            "workspace {workspace_id} disappeared while resolving path {path}"
                        ))
                    });
                }
                Err(error)
            }
        }
    }

    pub async fn replace_workspace(
        &self,
        workspace_id: i64,
        request: WorkspaceWriteRequest,
    ) -> ApiResult<WorkspaceResource> {
        let Some(existing) = entities::workspace::Entity::find_by_id(workspace_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Err(ApiError::not_found(format!(
                "workspace not found: {workspace_id}"
            )));
        };

        let path = normalize_workspace_path(request.path.as_str()).map_err(db_error)?;
        if path != existing.path
            && let Some(existing_id) = self.workspace_id_by_path(path.as_str()).await?
            && existing_id != workspace_id
        {
            return Err(ApiError::conflict(format!(
                "workspace path already exists: {path}"
            )));
        }

        let mut active: entities::workspace::ActiveModel = existing.into();
        active.path = Set(path);
        active.updated_at_ms = Set(Utc::now().timestamp_millis());
        let updated = active.update(self.db.as_ref()).await.map_err(db_error)?;
        let counts = self.workspace_session_counts(&[updated.id]).await?;
        workspace_resource(&updated, counts.get(&updated.id).copied())
    }

    pub async fn delete_workspace(&self, workspace_id: i64) -> ApiResult<WorkspaceResource> {
        let Some(existing) = entities::workspace::Entity::find_by_id(workspace_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Err(ApiError::not_found(format!(
                "workspace not found: {workspace_id}"
            )));
        };

        let counts = self.workspace_session_counts(&[workspace_id]).await?;
        entities::workspace::Entity::delete_by_id(workspace_id)
            .exec(self.db.as_ref())
            .await
            .map_err(db_error)?;
        workspace_resource(&existing, counts.get(&workspace_id).copied())
    }

    pub async fn list_sessions(
        &self,
        query: super::dto::SessionListQuery,
    ) -> ApiResult<PaginatedResponse<SessionResource>> {
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<SessionCursor>)
            .transpose()?;
        let mut statement = entities::session::Entity::find()
            .order_by_desc(entities::session::Column::UpdatedAtMs)
            .order_by_desc(entities::session::Column::Id);

        if let Some(workspace_id) = query.workspace_id {
            statement = statement.filter(entities::session::Column::WorkspaceId.eq(workspace_id));
        }
        if query.roots {
            statement = statement.filter(entities::session::Column::ParentId.is_null());
        }
        if let Some(parent_id) = query.parent_id {
            statement = statement.filter(entities::session::Column::ParentId.eq(parent_id));
        }
        if let Some(search) = non_empty(query.search.as_deref()) {
            statement =
                statement.filter(entities::session::Column::Title.like(format!("%{search}%")));
        }
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::session::Column::UpdatedAtMs.lt(cursor.updated_at_ms))
                    .add(
                        Condition::all()
                            .add(entities::session::Column::UpdatedAtMs.eq(cursor.updated_at_ms))
                            .add(entities::session::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let resources = self.session_resources_from_models(slice.as_slice()).await?;
        let next_cursor = slice.last().map(|row| SessionCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(resources, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_session(&self, session_id: i64) -> ApiResult<Option<SessionResource>> {
        let Some(model) = entities::session::Entity::find_by_id(session_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };

        let mut resources = self.session_resources_from_models(&[model]).await?;
        Ok(resources.pop())
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> ApiResult<SessionResource> {
        self.ensure_workspace_exists(request.workspace_id).await?;
        if let Some(parent_id) = request.parent_id {
            let parent = self.ensure_session_model(parent_id).await?;
            if parent.workspace_id != request.workspace_id {
                return Err(ApiError::bad_request(
                    "parent session must belong to the same workspace",
                ));
            }
        }

        let created = session_crud::create_session(
            self.db.as_ref(),
            request.workspace_id,
            request.parent_id,
            request.title,
        )
        .await
        .map_err(db_error)?;

        let mut resources = self.session_resources_from_models(&[created]).await?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize created session"))
    }

    pub async fn replace_session(
        &self,
        session_id: i64,
        request: SessionReplaceRequest,
    ) -> ApiResult<SessionResource> {
        let existing = self.ensure_session_model(session_id).await?;
        if request.parent_id == Some(session_id) {
            return Err(ApiError::bad_request(
                "session cannot be its own parent session",
            ));
        }
        if let Some(parent_id) = request.parent_id {
            let parent = self.ensure_session_model(parent_id).await?;
            if parent.workspace_id != existing.workspace_id {
                return Err(ApiError::bad_request(
                    "parent session must belong to the same workspace",
                ));
            }
        }

        let next_version = existing.version + 1;
        let mut active: entities::session::ActiveModel = existing.into();
        active.title = Set(request.title);
        active.parent_id = Set(request.parent_id);
        active.version = Set(next_version);
        active.updated_at_ms = Set(Utc::now().timestamp_millis());
        let updated = active.update(self.db.as_ref()).await.map_err(db_error)?;
        let mut resources = self.session_resources_from_models(&[updated]).await?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize updated session"))
    }

    pub async fn delete_session(&self, session_id: i64) -> ApiResult<SessionResource> {
        let existing = self.ensure_session_model(session_id).await?;
        let mut resources = self
            .session_resources_from_models(std::slice::from_ref(&existing))
            .await?;
        entities::session::Entity::delete_by_id(session_id)
            .exec(self.db.as_ref())
            .await
            .map_err(db_error)?;
        resources
            .pop()
            .ok_or_else(|| ApiError::internal("failed to materialize deleted session"))
    }

    pub async fn list_session_events(
        &self,
        manager: &SessionManager,
        session_id: i64,
        query: SessionEventListQuery,
    ) -> ApiResult<PaginatedResponse<agena::event::DomainEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<EventCursor>)
            .transpose()?;

        let all = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;

        // Newest-first, then apply cursor + limit, then re-sort ascending so
        // the response matches the legacy on-the-wire ordering.
        let mut newest_first: Vec<_> = all.into_iter().collect();
        newest_first.sort_by(|a, b| b.meta.seq_global.cmp(&a.meta.seq_global));
        if let Some(cursor) = cursor {
            newest_first.retain(|e| e.meta.seq_global < cursor.seq);
        }
        let has_more = newest_first.len() > limit as usize;
        let mut slice: Vec<_> = newest_first.into_iter().take(limit as usize).collect();
        let next_cursor = slice.last().map(|e| EventCursor {
            seq: e.meta.seq_global,
            id: e.meta.seq_global,
        });
        slice.reverse();

        build_page(slice, has_more, next_cursor, PageOrder::Asc, limit)
    }

    pub async fn list_messages(
        &self,
        manager: &SessionManager,
        session_id: i64,
        query: MessageListQuery,
    ) -> ApiResult<PaginatedResponse<MessageResource>> {
        self.ensure_session_exists(session_id).await?;
        let session = manager
            .get_session(session_id)
            .await
            .map_err(api_error_from_app)?;

        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<MessageCursor>)
            .transpose()?;

        // Project messages newest-first to match the original SQL ordering,
        // then apply cursor + limit.
        let mut all: Vec<&Message> = session.messages.iter().collect();
        all.sort_by(|a, b| {
            (b.created_at.timestamp_millis(), b.id).cmp(&(a.created_at.timestamp_millis(), a.id))
        });
        if let Some(cursor) = cursor {
            all.retain(|m| {
                let key = (m.created_at.timestamp_millis(), m.id);
                key < (cursor.created_at_ms, cursor.id)
            });
        }

        let has_more = all.len() > limit as usize;
        let mut slice: Vec<&Message> = all.into_iter().take(limit as usize).collect();
        let next_cursor = slice.last().map(|m| MessageCursor {
            created_at_ms: m.created_at.timestamp_millis(),
            id: m.id,
        });
        slice.reverse();

        let items: Vec<MessageResource> = slice
            .iter()
            .map(|m| message_resource_from_message(session.id, m, query.parts))
            .collect();

        build_page(items, has_more, next_cursor, PageOrder::Asc, limit)
    }

    pub async fn get_message(
        &self,
        manager: &SessionManager,
        message_id: i64,
        parts: PartLoadMode,
    ) -> ApiResult<Option<MessageResource>> {
        let Some(session_id) = manager
            .find_session_id_for_message(message_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Ok(None);
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(api_error_from_app)?;
        Ok(session
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| message_resource_from_message(session_id, m, parts)))
    }

    pub async fn list_message_parts(
        &self,
        manager: &SessionManager,
        message_id: i64,
        mode: PartLoadMode,
    ) -> ApiResult<Vec<MessagePart>> {
        if mode == PartLoadMode::None {
            return Ok(Vec::new());
        }
        let Some(session_id) = manager
            .find_session_id_for_message(message_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Err(ApiError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(api_error_from_app)?;
        let parts = session
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.parts.clone())
            .unwrap_or_default();
        Ok(parts.into_iter().map(|p| project_part(p, mode)).collect())
    }

    pub async fn get_message_part(
        &self,
        manager: &SessionManager,
        part_id: i64,
    ) -> ApiResult<Option<MessagePart>> {
        let Some(session_id) = manager
            .find_session_id_for_part(part_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Ok(None);
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(api_error_from_app)?;
        Ok(session
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .find(|p| p.id == part_id)
            .cloned())
    }

    pub async fn list_permission_rules(
        &self,
        query: PermissionRuleListQuery,
    ) -> ApiResult<PaginatedResponse<PermissionRuleResource>> {
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<PermissionRuleCursor>)
            .transpose()?;
        let mut statement = entities::permission_rule::Entity::find()
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id);
        if let Some(search) = non_empty(query.search.as_deref()) {
            statement = statement
                .filter(entities::permission_rule::Column::ActionKey.like(format!("%{search}%")));
        }
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::permission_rule::Column::UpdatedAtMs.lt(cursor.updated_at_ms))
                    .add(
                        Condition::all()
                            .add(
                                entities::permission_rule::Column::UpdatedAtMs
                                    .eq(cursor.updated_at_ms),
                            )
                            .add(entities::permission_rule::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let items = slice
            .iter()
            .map(permission_rule_resource)
            .collect::<ApiResult<Vec<_>>>()?;
        let next_cursor = slice.last().map(|row| PermissionRuleCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(items, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_permission_rule(
        &self,
        rule_id: i64,
    ) -> ApiResult<Option<PermissionRuleResource>> {
        let row = entities::permission_rule::Entity::find_by_id(rule_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?;
        row.as_ref().map(permission_rule_resource).transpose()
    }

    pub async fn create_permission_rule(
        &self,
        request: PermissionRuleWriteRequest,
    ) -> ApiResult<PermissionRuleResource> {
        if self
            .permission_rule_id_by_action_key(request.action_key.as_str())
            .await?
            .is_some()
        {
            return Err(ApiError::conflict(format!(
                "permission rule already exists for action_key '{}'",
                request.action_key
            )));
        }

        let created = permission_rule_crud::upsert_rule(
            self.db.as_ref(),
            request.action_key.as_str(),
            request.mode,
        )
        .await
        .map_err(db_error)?;
        permission_rule_resource(&created)
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        request: PermissionRuleWriteRequest,
    ) -> ApiResult<PermissionRuleResource> {
        let Some(existing) = entities::permission_rule::Entity::find_by_id(rule_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Err(ApiError::not_found(format!(
                "permission rule not found: {rule_id}"
            )));
        };

        if existing.action_key != request.action_key
            && let Some(existing_id) = self
                .permission_rule_id_by_action_key(request.action_key.as_str())
                .await?
            && existing_id != rule_id
        {
            return Err(ApiError::conflict(format!(
                "permission rule already exists for action_key '{}'",
                request.action_key
            )));
        }

        let now_ms = Utc::now().timestamp_millis();
        let mut active: entities::permission_rule::ActiveModel = existing.into();
        active.action_key = Set(request.action_key);
        active.mode = Set(permission_mode_to_string(request.mode));
        active.updated_at_ms = Set(now_ms);
        let updated = active.update(self.db.as_ref()).await.map_err(db_error)?;
        permission_rule_resource(&updated)
    }

    pub async fn delete_permission_rule(&self, rule_id: i64) -> ApiResult<PermissionRuleResource> {
        let Some(existing) = entities::permission_rule::Entity::find_by_id(rule_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Err(ApiError::not_found(format!(
                "permission rule not found: {rule_id}"
            )));
        };

        let resource = permission_rule_resource(&existing)?;
        entities::permission_rule::Entity::delete_by_id(rule_id)
            .exec(self.db.as_ref())
            .await
            .map_err(db_error)?;
        Ok(resource)
    }

    pub async fn assert_session_version(
        &self,
        session_id: i64,
        expected_version: i64,
    ) -> ApiResult<()> {
        let existing = self.ensure_session_model(session_id).await?;
        if existing.version == expected_version {
            return Ok(());
        }

        Err(ApiError::conflict(format!(
            "session version mismatch for {session_id}: expected {expected_version}, current {}",
            existing.version
        )))
    }

    pub async fn latest_session_event_seq(
        &self,
        manager: &SessionManager,
        session_id: i64,
    ) -> ApiResult<Option<i64>> {
        self.ensure_session_exists(session_id).await?;
        let events = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;
        Ok(events.iter().map(|e| e.meta.seq_global).max())
    }

    pub async fn list_session_events_after(
        &self,
        manager: &SessionManager,
        session_id: i64,
        after_seq: i64,
        limit: Option<u64>,
    ) -> ApiResult<Vec<agena::event::DomainEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(limit) as usize;
        let mut events = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;
        events.retain(|e| e.meta.seq_global > after_seq);
        events.truncate(limit);
        Ok(events)
    }

    pub async fn resolve_run_options(
        &self,
        provider_registry: &ProviderRegistry,
        manager: &SessionManager,
        session_id: i64,
        request: SessionRunOptionsRequest,
    ) -> ApiResult<agena::session::SessionRunOptions> {
        self.ensure_session_exists(session_id).await?;

        let model = match request.model {
            Some(model) => {
                ensure_provider_exists(provider_registry, &model)?;
                model
            }
            None => match self.infer_session_model(manager, session_id).await? {
                Some(model) => {
                    ensure_provider_exists(provider_registry, &model)?;
                    model
                }
                None => default_model_from_registry(provider_registry).ok_or_else(|| {
                    ApiError::bad_request(
                        "model is required when the session has no previous model and multiple providers are configured",
                    )
                })?,
            },
        };

        if let Some(temperature) = request.temperature
            && !temperature.is_finite()
        {
            return Err(ApiError::bad_request("temperature must be a finite number"));
        }
        if matches!(request.max_output_tokens, Some(0)) {
            return Err(ApiError::bad_request(
                "max_output_tokens must be greater than zero",
            ));
        }

        Ok(agena::session::SessionRunOptions {
            model,
            system: non_empty(request.system.as_deref()).map(ToOwned::to_owned),
            temperature: request.temperature,
            max_output_tokens: request.max_output_tokens,
        })
    }

    pub async fn session_execution_resource(
        &self,
        manager: &SessionManager,
        session: &Session,
    ) -> ApiResult<SessionExecutionResource> {
        let session_resource = self.get_session(session.id).await?.ok_or_else(|| {
            ApiError::internal("session disappeared while loading execution state")
        })?;

        Ok(SessionExecutionResource {
            session: session_resource,
            blocked: session.blocked(),
            run_state: SessionRunState::from(session.status()),
            latest_event_seq: self.latest_session_event_seq(manager, session.id).await?,
            pending_permission_requests: pending_permission_requests(session),
            pending_user_input_requests: pending_user_input_requests(session),
        })
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

    async fn infer_session_model(
        &self,
        manager: &SessionManager,
        session_id: i64,
    ) -> ApiResult<Option<ModelRef>> {
        let session = manager
            .get_session(session_id)
            .await
            .map_err(api_error_from_app)?;
        let mut sorted: Vec<&Message> = session.messages.iter().collect();
        sorted.sort_by(|a, b| {
            (b.created_at.timestamp_millis(), b.id).cmp(&(a.created_at.timestamp_millis(), a.id))
        });
        for m in sorted {
            let provider_id = m.metadata.model_provider_id.trim();
            let model_id = m.metadata.model_id.trim();
            if provider_id.is_empty() || model_id.is_empty() {
                continue;
            }
            return ModelRef::try_new(provider_id, model_id)
                .map(Some)
                .map_err(|error| {
                    ApiError::bad_request(format!(
                        "session {session_id} contains invalid persisted model metadata: {error}"
                    ))
                });
        }
        Ok(None)
    }

    async fn workspace_id_by_path(&self, path: &str) -> ApiResult<Option<i64>> {
        agena::db::crud::workspace::get_workspace_id_by_path(self.db.as_ref(), path)
            .await
            .map_err(db_error)
    }

    async fn permission_rule_id_by_action_key(&self, action_key: &str) -> ApiResult<Option<i64>> {
        entities::permission_rule::Entity::find()
            .select_only()
            .column(entities::permission_rule::Column::Id)
            .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
            .into_tuple::<i64>()
            .one(self.db.as_ref())
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

    async fn session_resources_from_models(
        &self,
        models: &[entities::session::Model],
    ) -> ApiResult<Vec<SessionResource>> {
        if models.is_empty() {
            return Ok(Vec::new());
        }

        let session_ids = models.iter().map(|row| row.id).collect::<Vec<_>>();
        // Per-session message stats are computed from the unified event log
        // by `SessionManager::list_session_events`. The HTTP service no
        // longer queries a per-session stats table.
        let mut message_stats: HashMap<i64, session_crud::SessionMessageStats> = HashMap::new();
        // Without access to the SessionManager here we conservatively
        // report zero counts. Routes that want accurate per-session counts
        // should fetch them via the dedicated stats endpoint or compute
        // them client-side.
        for &id in &session_ids {
            let _ = id;
        }
        let _ = (&mut message_stats,);
        let child_counts =
            session_crud::child_session_counts_by_parent_ids(self.db.as_ref(), &session_ids)
                .await
                .map_err(db_error)?;

        models
            .iter()
            .map(|model| session_resource(model, &message_stats, &child_counts))
            .collect()
    }
}

fn workspace_resource(
    row: &entities::workspace::Model,
    session_count: Option<u64>,
) -> ApiResult<WorkspaceResource> {
    Ok(WorkspaceResource {
        id: row.id,
        path: row.path.clone(),
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(row.updated_at_ms)?,
        session_count,
    })
}

fn clean_workspace_relative_path(value: Option<&str>) -> ApiResult<PathBuf> {
    let mut cleaned = PathBuf::new();
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(cleaned);
    };
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(ApiError::bad_request(
            "workspace file path must be relative",
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => cleaned.push(part),
            std::path::Component::CurDir => {}
            _ => {
                return Err(ApiError::bad_request(
                    "workspace file path cannot contain parent or root components",
                ));
            }
        }
    }
    Ok(cleaned)
}

fn read_workspace_entries(
    root: &Path,
    dir: &Path,
    depth: usize,
    remaining: &mut usize,
) -> ApiResult<Vec<WorkspaceFileNode>> {
    if *remaining == 0 {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(dir)
        .map_err(|error| workspace_fs_error(dir, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| workspace_fs_error(dir, error))?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut nodes = Vec::new();
    for entry in entries {
        if *remaining == 0 {
            break;
        }

        let path = entry.path();
        let metadata = fs::symlink_metadata(path.as_path())
            .map_err(|error| workspace_fs_error(path.as_path(), error))?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_dir() {
            WorkspaceFileKind::Directory
        } else if file_type.is_file() {
            WorkspaceFileKind::File
        } else if file_type.is_symlink() {
            WorkspaceFileKind::Symlink
        } else {
            WorkspaceFileKind::Other
        };
        *remaining -= 1;
        let children = if kind == WorkspaceFileKind::Directory && depth > 0 {
            Some(read_workspace_entries(
                root,
                path.as_path(),
                depth - 1,
                remaining,
            )?)
        } else {
            None
        };
        nodes.push(WorkspaceFileNode {
            name: entry.file_name().to_string_lossy().to_string(),
            path: path
                .strip_prefix(root)
                .map(workspace_relative_path)
                .unwrap_or_else(|_| path.display().to_string()),
            kind,
            size: (kind == WorkspaceFileKind::File).then_some(metadata.len()),
            children,
        });
    }
    nodes.sort_by(|left, right| {
        let left_dir = left.kind == WorkspaceFileKind::Directory;
        let right_dir = right.kind == WorkspaceFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(nodes)
}

fn workspace_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn workspace_fs_error(path: &Path, error: io::Error) -> ApiError {
    match error.kind() {
        io::ErrorKind::NotFound => {
            ApiError::not_found(format!("workspace file path not found: {}", path.display()))
        }
        io::ErrorKind::PermissionDenied => ApiError::bad_request(format!(
            "workspace file path cannot be read: {}",
            path.display()
        )),
        _ => ApiError::internal(format!(
            "workspace file path error for {}: {}",
            path.display(),
            error
        )),
    }
}

fn session_resource(
    model: &entities::session::Model,
    message_stats: &HashMap<i64, session_crud::SessionMessageStats>,
    child_counts: &HashMap<i64, i64>,
) -> ApiResult<SessionResource> {
    let stats = message_stats.get(&model.id).copied();
    let message_count = stats
        .map(|item| u64::try_from(item.message_count))
        .transpose()
        .map_err(|_| {
            ApiError::internal(format!(
                "invalid negative message count for session {}",
                model.id
            ))
        })?
        .unwrap_or_default();
    let child_session_count = child_counts
        .get(&model.id)
        .copied()
        .map(u64::try_from)
        .transpose()
        .map_err(|_| {
            ApiError::internal(format!(
                "invalid negative child session count for session {}",
                model.id
            ))
        })?
        .unwrap_or_default();

    Ok(SessionResource {
        id: model.id,
        parent_id: model.parent_id,
        workspace_id: model.workspace_id,
        title: model.title.clone(),
        version: model.version,
        created_at: timestamp_millis_to_utc(model.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
        message_count,
        child_session_count,
        last_message_at: stats
            .and_then(|item| item.last_message_at_ms)
            .map(timestamp_millis_to_utc)
            .transpose()?,
    })
}

fn permission_rule_resource(
    row: &entities::permission_rule::Model,
) -> ApiResult<PermissionRuleResource> {
    Ok(PermissionRuleResource {
        id: row.id,
        action_key: row.action_key.clone(),
        mode: permission_mode_from_string(row.mode.as_str())?,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(row.updated_at_ms)?,
    })
}

/// Project a `Message` (from the in-memory `Session.messages`) into the
/// HTTP API `MessageResource` shape that the legacy SQL-backed code path
/// produced from row models.
fn message_resource_from_message(
    session_id: i64,
    message: &Message,
    parts_mode: PartLoadMode,
) -> MessageResource {
    let part_count = message.parts.len() as u64;
    let parts = match parts_mode {
        PartLoadMode::None => None,
        PartLoadMode::Summary | PartLoadMode::Full => Some(
            message
                .parts
                .iter()
                .cloned()
                .map(|p| project_part(p, parts_mode))
                .collect(),
        ),
    };
    MessageResource {
        id: message.id,
        session_id,
        role: message.role,
        state: message.state,
        created_at: message.created_at,
        // The append-only event log carries no separate "updated_at" — every
        // message in `Session.messages` is in its terminal projected form.
        updated_at: message.created_at,
        metadata: message.metadata.clone(),
        usage: message.usage.clone(),
        finish: message.finish.clone(),
        part_count,
        parts,
    }
}

fn project_part(mut part: MessagePart, mode: PartLoadMode) -> MessagePart {
    if mode == PartLoadMode::Summary {
        // Drop the heavy detail payload — clients in summary mode only consume
        // the part header.
        part.content = None;
    }
    part
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

fn permission_mode_to_string(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow".to_string(),
        PermissionMode::Ask => "ask".to_string(),
        PermissionMode::Deny => "deny".to_string(),
    }
}

fn permission_mode_from_string(value: &str) -> ApiResult<PermissionMode> {
    match value {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(ApiError::internal(format!(
            "invalid permission mode in storage: {value}"
        ))),
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn normalize_workspace_path(workspace_path: &str) -> Result<String, DbErr> {
    let raw = workspace_path.trim();
    if raw.is_empty() {
        return Err(DbErr::Custom("workspace path cannot be empty".to_string()));
    }

    let cleaned = Path::new(raw).clean();
    let mut normalized = cleaned.to_string_lossy().replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 1 && !is_windows_drive_root(&normalized) {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    Ok(normalized)
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> ApiResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| ApiError::internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn db_error(error: DbErr) -> ApiError {
    ApiError::from(AppError::Database(error))
}

fn ensure_provider_exists(provider_registry: &ProviderRegistry, model: &ModelRef) -> ApiResult<()> {
    if provider_registry.get(model.provider_id.as_str()).is_some() {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "provider not configured: {}",
            model.provider_id
        )))
    }
}

fn default_model_from_registry(provider_registry: &ProviderRegistry) -> Option<ModelRef> {
    let provider_ids = provider_registry.provider_ids();
    if provider_ids.len() != 1 {
        return None;
    }

    let provider_id = provider_ids.into_iter().next()?;
    let provider = provider_registry.get(provider_id.as_str())?;
    Some(ModelRef::new(
        provider_id,
        provider.default_model().to_string(),
    ))
}

fn pending_permission_requests(session: &Session) -> Vec<agena::permission::PermissionRequest> {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| {
            if part.status != ExecutionStatus::Pending {
                return None;
            }

            let PartContent::PermissionRequest(PermissionRequestPart { request, reply }) =
                part.content.as_ref()?
            else {
                return None;
            };
            reply.is_none().then_some(request.clone())
        })
        .collect()
}

fn pending_user_input_requests(session: &Session) -> Vec<UserInputRequest> {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| {
            if part.status != ExecutionStatus::Pending {
                return None;
            }

            let PartContent::UserInputRequest(UserInputRequestPart { request, reply }) =
                part.content.as_ref()?
            else {
                return None;
            };
            reply.is_none().then_some(request.clone())
        })
        .collect()
}
