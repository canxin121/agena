use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
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
        crud::{
            permission_rule as permission_rule_crud, session as session_crud,
            workspace as workspace_crud,
        },
        entities,
    },
    event::{EventKind, EventPublisher, PermissionRuleEvent, PublishContext},
    message::{
        ExecutionStatus, Message, MessagePart, PartContent, PermissionRequestPart,
        UserInputRequest, UserInputRequestPart,
    },
    model::ModelRef,
    permission::{PermissionAction, PermissionMode, PermissionScope, PersistedPermissionRule},
    provider::ProviderRegistry,
    session::{Session, SessionGoal, SessionManager},
};

use super::{
    dto::{
        GitStatusResource, MessageListQuery, MessageResource, PartLoadMode,
        PermissionRuleListQuery, PermissionRuleResource, PermissionRuleWriteRequest,
        ScheduledJobResource, ScheduledJobRunResource, SessionAutomationResource,
        SessionCreateRequest, SessionEventListQuery, SessionExecutionContextResource,
        SessionExecutionResource, SessionGoalResource, SessionReplaceRequest, SessionResource,
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

#[cfg(test)]
use agena::message::MessageStatus;

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
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<MessageCursor>)
            .transpose()?;
        let visible =
            load_visible_message_projection(manager, session_id, query.parts == PartLoadMode::Full)
                .await?;
        let (messages, has_more, next_cursor) =
            paginate_visible_messages(visible.messages.as_slice(), cursor, limit);
        let items: Vec<MessageResource> = messages
            .iter()
            .map(|message| message_resource_from_message(session_id, message, query.parts))
            .collect();

        build_page(
            items,
            has_more,
            next_cursor.map(|(created_at_ms, id)| MessageCursor { created_at_ms, id }),
            PageOrder::Asc,
            limit,
        )
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
        let visible =
            load_visible_message_projection(manager, session_id, parts == PartLoadMode::Full)
                .await?;
        Ok(visible
            .find_message(message_id)
            .map(|message| message_resource_from_message(session_id, message, parts)))
    }

    pub async fn list_message_parts(
        &self,
        manager: &SessionManager,
        message_id: i64,
        mode: PartLoadMode,
    ) -> ApiResult<Vec<MessagePart>> {
        let Some(session_id) = manager
            .find_session_id_for_message(message_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Err(ApiError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        if mode == PartLoadMode::None {
            return Ok(Vec::new());
        }
        let visible =
            load_visible_message_projection(manager, session_id, mode == PartLoadMode::Full)
                .await?;
        let Some(message) = visible.find_message(message_id) else {
            return Err(ApiError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        Ok(message
            .parts
            .iter()
            .cloned()
            .map(|part| project_part(part, mode))
            .collect())
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
        let visible = load_visible_message_projection(manager, session_id, true).await?;
        Ok(visible.find_part(part_id))
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
        let workspace_id =
            workspace_crud::ensure_workspace_id(self.db.as_ref(), self.workspace_root.as_str())
                .await
                .map_err(db_error)?;
        let scope = permission_scope_from_request(request.scope.as_deref())?;
        let action = permission_action_from_write_request(&request, self.workspace_root.as_str())?;
        let action_key = serde_json::to_string(&action)
            .map_err(AppError::from)
            .map_err(api_error_from_app)?;
        let rule = PersistedPermissionRule {
            action_key,
            mode: request.mode,
            scope,
            session_id: match scope {
                PermissionScope::Session => request.session_id,
                PermissionScope::Workspace | PermissionScope::Global => None,
            },
            workspace_id: match scope {
                PermissionScope::Session | PermissionScope::Global => None,
                PermissionScope::Workspace => Some(workspace_id),
            },
            source: "api".to_string(),
            reason: None,
            operator: Some("http_api".to_string()),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        };

        let (created, is_new) = permission_rule_crud::upsert_rule(self.db.as_ref(), &rule)
            .await
            .map_err(db_error)?;
        let resource = permission_rule_resource(&created)?;
        self.publish_permission_rule_event(if is_new {
            EventKind::PermissionRuleCreated(permission_rule_event(&created))
        } else {
            EventKind::PermissionRuleUpdated(permission_rule_event(&created))
        })
        .await?;
        Ok(resource)
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

        let workspace_id =
            workspace_crud::ensure_workspace_id(self.db.as_ref(), self.workspace_root.as_str())
                .await
                .map_err(db_error)?;
        let scope = permission_scope_from_request(request.scope.as_deref())?;
        let action = permission_action_from_write_request(&request, self.workspace_root.as_str())?;
        let action_key = serde_json::to_string(&action)
            .map_err(AppError::from)
            .map_err(api_error_from_app)?;
        let now_ms = Utc::now().timestamp_millis();
        let mut active: entities::permission_rule::ActiveModel = existing.into();
        active.action_key = Set(action_key);
        active.mode = Set(permission_mode_to_string(request.mode));
        active.scope = Set(permission_scope_to_string(scope));
        active.session_id = Set(match scope {
            PermissionScope::Session => request.session_id,
            PermissionScope::Workspace | PermissionScope::Global => None,
        });
        active.workspace_id = Set(match scope {
            PermissionScope::Session | PermissionScope::Global => None,
            PermissionScope::Workspace => Some(workspace_id),
        });
        active.source = Set("api".to_string());
        active.operator = Set(Some("http_api".to_string()));
        active.updated_at_ms = Set(now_ms);
        let updated = active.update(self.db.as_ref()).await.map_err(db_error)?;
        let resource = permission_rule_resource(&updated)?;
        self.publish_permission_rule_event(EventKind::PermissionRuleUpdated(
            permission_rule_event(&updated),
        ))
        .await?;
        Ok(resource)
    }

    pub async fn revoke_permission_rule(
        &self,
        rule_id: i64,
        reason: Option<String>,
    ) -> ApiResult<PermissionRuleResource> {
        let Some(updated) = permission_rule_crud::revoke_rule(
            self.db.as_ref(),
            rule_id,
            reason,
            Some("http_api".to_string()),
        )
        .await
        .map_err(db_error)?
        else {
            return Err(ApiError::not_found(format!(
                "permission rule not found: {rule_id}"
            )));
        };
        let resource = permission_rule_resource(&updated)?;
        self.publish_permission_rule_event(EventKind::PermissionRuleRevoked(
            permission_rule_event(&updated),
        ))
        .await?;
        Ok(resource)
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

    pub async fn git_status(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> ApiResult<GitStatusResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let git_available = command_available("git");
        let gh_available = command_available("gh");

        let Some(manager) = runtime.session_manager() else {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions: 0,
                worktree_managed_dirs: 0,
            });
        };

        let executor = manager.tool_executor();
        let (worktree_active_sessions, worktree_managed_dirs) = match executor.worktree_registry() {
            Some(registry) => (
                agena::tool::worktree_list_active(registry).len() as u64,
                agena::tool::worktree_list_managed(&workspace_root, registry).len() as u64,
            ),
            None => (0, 0),
        };

        if !git_available {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let branch = git_output(&workspace_root, ["branch", "--show-current"])?;
        let upstream = git_output(
            &workspace_root,
            [
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .and_then(|value| non_empty(Some(value.as_str())).map(ToOwned::to_owned));
        let ahead_behind = upstream.as_ref().and_then(|_| {
            git_output(
                &workspace_root,
                ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .ok()
        });
        let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
        let status = git_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, unstaged_files, untracked_files, changed_files) =
            summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            workspace_root: workspace_root.display().to_string(),
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            upstream,
            ahead,
            behind,
            staged_files,
            unstaged_files,
            untracked_files,
            changed_files,
            clean: changed_files == 0,
            worktree_active_sessions,
            worktree_managed_dirs,
        })
    }

    pub async fn git_init(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> ApiResult<GitStatusResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        if !command_available("git") {
            return Err(ApiError::bad_request(
                "git is not available on PATH; cannot initialize a repository",
            ));
        }

        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]) {
            let output = Command::new("git")
                .args(["init"])
                .current_dir(&workspace_root)
                .output()
                .map_err(|error| {
                    ApiError::internal(format!("failed to execute git init: {error}"))
                })?;
            if !output.status.success() {
                return Err(ApiError::internal(format!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }

        self.git_status(runtime).await
    }

    pub async fn vcs_diff_raw(&self, runtime: &agena::runtime::AgenaRuntime) -> ApiResult<String> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        if !command_available("git") {
            return Ok(String::new());
        }
        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]) {
            return Ok(String::new());
        }

        let mut chunks = Vec::<String>::new();
        if git_success(&workspace_root, ["rev-parse", "--verify", "HEAD"]) {
            let tracked = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
                &[0],
            )?;
            if !tracked.trim().is_empty() {
                chunks.push(tracked);
            }
        } else {
            let staged = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "--cached", "--"],
                &[0],
            )?;
            if !staged.trim().is_empty() {
                chunks.push(staged);
            }
        }

        let status = git_output(&workspace_root, ["status", "--porcelain"])?;
        for file in untracked_files_from_status(status.as_str()) {
            let patch = git_untracked_patch(&workspace_root, file.as_str())?;
            if !patch.trim().is_empty() {
                chunks.push(patch);
            }
        }

        Ok(chunks.join("\n"))
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
        let variant = non_empty(request.variant.as_deref()).map(ToOwned::to_owned);
        let thinking = if let Some(variant_name) = variant.as_deref() {
            let variants = provider_registry
                .model_variants(&model)
                .map_err(api_error_from_app)?;
            let variant = variants.get(variant_name).ok_or_else(|| {
                ApiError::bad_request(format!("model `{}` has no variant `{variant_name}`", model))
            })?;
            variant.thinking.clone()
        } else {
            None
        };

        Ok(agena::session::SessionRunOptions {
            model,
            variant,
            thinking,
            system: non_empty(request.system.as_deref()).map(ToOwned::to_owned),
            temperature: request.temperature,
            max_output_tokens: request.max_output_tokens,
            agent_profile: non_empty(request.agent_profile.as_deref()).map(ToOwned::to_owned),
            max_turn_loops: request.max_turn_loops,
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

        let scheduler_jobs = list_scheduled_jobs(manager).await;

        Ok(SessionExecutionResource {
            session: session_resource,
            blocked: session.blocked(),
            run_state: SessionRunState::from(session.status()),
            latest_event_seq: self.latest_session_event_seq(manager, session.id).await?,
            automation: session_automation_resource(&scheduler_jobs, session.id),
            execution: SessionExecutionContextResource {
                agent_profile: session.runtime().execution.agent_profile.clone(),
                agent_mode: session.runtime().execution.agent_mode,
                agent_hidden: session.runtime().execution.agent_hidden,
                agent_color: session.runtime().execution.agent_color.clone(),
                active_skill_name: session.runtime().execution.active_skill_name.clone(),
                system_prompt_override: session.runtime().execution.system_prompt_override.clone(),
                allowed_tools: session.runtime().execution.allowed_tools.clone(),
                agent_permission: session.runtime().execution.agent_permission.clone(),
                model_provider_id: session.runtime().execution.model_provider_id.clone(),
                model_id: session.runtime().execution.model_id.clone(),
                model_variant: session.runtime().execution.model_variant.clone(),
                agent_run: session.runtime().execution.agent_run.clone(),
                effective_workspace_root: session
                    .runtime()
                    .effective_workspace_root()
                    .map(|path| path.display().to_string()),
                task_id: session.runtime().execution.task_id.clone(),
            },
            pending_permission_requests: pending_permission_requests(session),
            pending_user_input_requests: pending_user_input_requests(session),
            goal: match session.goal.as_ref() {
                Some(goal) => Some(self.session_goal_resource(manager, session, goal).await?),
                None => None,
            },
        })
    }

    pub async fn session_goal_resource(
        &self,
        _manager: &SessionManager,
        _session: &Session,
        goal: &SessionGoal,
    ) -> ApiResult<SessionGoalResource> {
        Ok(SessionGoalResource {
            id: goal.id,
            session_id: goal.session_id,
            objective: goal.objective.clone(),
            status: goal.status,
            token_budget: goal.token_budget,
            tokens_used: goal.tokens_used,
            time_used_seconds: goal.time_used_seconds,
            created_at: goal.created_at,
            updated_at: goal.updated_at,
            completed_at: goal.completed_at,
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
        depth: model.depth,
        root_id: model.root_id,
        workspace_id: model.workspace_id,
        title: model.title.clone(),
        version: model.version,
        is_subagent: model.is_subagent,
        created_at: timestamp_millis_to_utc(model.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
        message_count,
        child_session_count,
        last_message_at: stats
            .and_then(|item| item.last_message_at_ms)
            .map(timestamp_millis_to_utc)
            .transpose()?,
        goal: None,
    })
}

fn permission_rule_resource(
    row: &entities::permission_rule::Model,
) -> ApiResult<PermissionRuleResource> {
    let action: PermissionAction = serde_json::from_str(row.action_key.as_str())
        .map_err(AppError::from)
        .map_err(api_error_from_app)?;
    let (
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
    ) = match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => (
            "tool".to_string(),
            Some(tool_name),
            qualifier,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => (
            "path_access".to_string(),
            None,
            None,
            Some(access_kind),
            Some(workspace_root),
            Some(target_path),
            None,
            None,
            None,
        ),
        PermissionAction::NetworkAccess { target, host, port } => (
            "network_access".to_string(),
            None,
            None,
            None,
            None,
            None,
            Some(target),
            Some(host),
            port,
        ),
    };
    Ok(PermissionRuleResource {
        id: row.id,
        action_key: row.action_key.clone(),
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
        mode: permission_mode_from_string(row.mode.as_str())?,
        scope: row.scope.clone(),
        session_id: row.session_id,
        workspace_id: row.workspace_id,
        source: row.source.clone(),
        reason: row.reason.clone(),
        operator: row.operator.clone(),
        revoked_at: row.revoked_at_ms.map(timestamp_millis_to_utc).transpose()?,
        revoked_reason: row.revoked_reason.clone(),
        revoked_by: row.revoked_by.clone(),
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(row.updated_at_ms)?,
    })
}

impl ApiService {
    async fn publish_permission_rule_event(&self, kind: EventKind) -> ApiResult<()> {
        let Some(publisher) = self.publisher.as_ref() else {
            return Ok(());
        };
        publisher
            .publish(PublishContext::default(), kind)
            .await
            .map_err(|err| {
                ApiError::internal(format!("publish permission rule event failed: {err}"))
            })?;
        Ok(())
    }
}

fn permission_rule_event(row: &entities::permission_rule::Model) -> PermissionRuleEvent {
    PermissionRuleEvent {
        session_id: row.session_id,
        rule_id: row.id,
        action_key: row.action_key.clone(),
        mode: row.mode.clone(),
        scope: row.scope.clone(),
        source: row.source.clone(),
        reason: row.reason.clone(),
        operator: row.operator.clone(),
        revoked_reason: row.revoked_reason.clone(),
        revoked_by: row.revoked_by.clone(),
        ts_ms: Utc::now().timestamp_millis(),
    }
}

fn permission_scope_from_request(value: Option<&str>) -> ApiResult<PermissionScope> {
    match value.unwrap_or("workspace") {
        "session" => Ok(PermissionScope::Session),
        "workspace" => Ok(PermissionScope::Workspace),
        "global" => Ok(PermissionScope::Global),
        other => Err(ApiError::bad_request(format!(
            "unsupported permission scope: {other}"
        ))),
    }
}

fn permission_scope_to_string(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
}

fn permission_action_from_write_request(
    request: &PermissionRuleWriteRequest,
    workspace_root: &str,
) -> ApiResult<PermissionAction> {
    if let Some(action_key) = request.action_key.as_deref()
        && !action_key.trim().is_empty()
    {
        return serde_json::from_str(action_key)
            .map_err(AppError::from)
            .map_err(api_error_from_app);
    }

    match request.subject_kind.as_deref() {
        Some("tool") => {
            let tool_name = request
                .tool_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request("tool_name is required for tool rule")
                })?
                .to_string();
            Ok(PermissionAction::Tool {
                tool_name,
                qualifier: request
                    .qualifier
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string),
            })
        }
        Some("path_access") => {
            let access_kind = request
                .path_access_kind
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request("path_access_kind is required for path_access rule")
                })?
                .to_string();
            let target_path = request
                .target_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request("target_path is required for path_access rule")
                })?
                .to_string();
            let workspace_root = request
                .workspace_root
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(workspace_root)
                .to_string();
            Ok(PermissionAction::PathAccess {
                access_kind,
                workspace_root,
                target_path,
            })
        }
        Some("network_access") => {
            let target = request
                .network_target
                .as_deref()
                .or(request.network_host.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request(
                        "network_target or network_host is required for network_access rule",
                    )
                })?
                .to_string();
            let parsed = agena::permission::NetworkTarget::parse(
                request
                    .network_port
                    .map(|port| format!("{target}:{port}"))
                    .unwrap_or_else(|| target.clone()),
            )
            .map_err(|err| ApiError::bad_request(format!("invalid network target: {err}")))?;
            Ok(PermissionAction::NetworkAccess {
                target,
                host: parsed.host().to_string(),
                port: parsed.port(),
            })
        }
        Some(other) => Err(ApiError::bad_request(format!(
            "unsupported permission subject_kind: {other}"
        ))),
        None => Err(ApiError::bad_request(
            "permission rule requires either action_key or structured subject fields",
        )),
    }
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
        role: visible_message_role(message.role),
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

#[derive(Debug, Clone)]
struct VisibleMessageProjection {
    messages: Vec<Message>,
    hidden_message_aliases: HashMap<i64, i64>,
}

impl VisibleMessageProjection {
    fn find_message(&self, message_id: i64) -> Option<&Message> {
        let visible_id = self
            .hidden_message_aliases
            .get(&message_id)
            .copied()
            .unwrap_or(message_id);
        self.messages
            .iter()
            .find(|message| message.id == visible_id)
    }

    fn find_part(&self, part_id: i64) -> Option<MessagePart> {
        self.messages.iter().find_map(|message| {
            message
                .parts
                .iter()
                .find(|part| part.id == part_id)
                .cloned()
        })
    }
}

fn visible_message_role(role: agena::role::Role) -> agena_api::resource::MessageRole {
    match role {
        agena::role::Role::User => agena_api::resource::MessageRole::User,
        agena::role::Role::Assistant | agena::role::Role::Tool => {
            agena_api::resource::MessageRole::Assistant
        }
        agena::role::Role::System => agena_api::resource::MessageRole::System,
    }
}

async fn load_visible_message_projection(
    manager: &SessionManager,
    session_id: i64,
    include_full_parts: bool,
) -> ApiResult<VisibleMessageProjection> {
    let messages = manager
        .list_projected_messages(session_id, include_full_parts)
        .await
        .map_err(api_error_from_app)?;
    Ok(project_visible_messages(messages))
}

fn project_visible_messages(messages: Vec<Message>) -> VisibleMessageProjection {
    let mut visible = Vec::with_capacity(messages.len());
    let mut hidden_message_aliases = HashMap::new();
    let mut assistant_indices_by_id = HashMap::<i64, usize>::new();
    let mut assistant_indices_by_operation = HashMap::<String, usize>::new();

    for mut message in messages {
        if message.role != agena::role::Role::Tool {
            normalize_message_parts(&mut message);
            let visible_index = visible.len();
            if message.role == agena::role::Role::Assistant {
                assistant_indices_by_id.insert(message.id, visible_index);
                index_assistant_operations(
                    &message,
                    visible_index,
                    &mut assistant_indices_by_operation,
                );
            }
            visible.push(message);
            continue;
        }

        let target_index = visible_tool_parent_index(
            &message,
            visible.as_slice(),
            &assistant_indices_by_id,
            &assistant_indices_by_operation,
        );

        let Some(target_index) = target_index else {
            message.role = agena::role::Role::Assistant;
            normalize_message_parts(&mut message);
            let visible_index = visible.len();
            assistant_indices_by_id.insert(message.id, visible_index);
            index_assistant_operations(
                &message,
                visible_index,
                &mut assistant_indices_by_operation,
            );
            visible.push(message);
            continue;
        };

        let target_message_id = visible[target_index].id;
        hidden_message_aliases.insert(message.id, target_message_id);

        for mut part in message.parts {
            part.message_id = target_message_id;
            part.part_index = visible[target_index].parts.len() as i32;
            if let Some(operation_id) = part.operation_id.clone() {
                assistant_indices_by_operation.insert(operation_id, target_index);
            }
            visible[target_index].parts.push(part);
        }
    }

    VisibleMessageProjection {
        messages: visible,
        hidden_message_aliases,
    }
}

fn normalize_message_parts(message: &mut Message) {
    for (index, part) in message.parts.iter_mut().enumerate() {
        part.message_id = message.id;
        part.part_index = index as i32;
    }
}

fn index_assistant_operations(
    message: &Message,
    visible_index: usize,
    assistant_indices_by_operation: &mut HashMap<String, usize>,
) {
    for operation_id in message
        .parts
        .iter()
        .filter_map(|part| part.operation_id.as_ref())
    {
        assistant_indices_by_operation.insert(operation_id.clone(), visible_index);
    }
}

fn visible_tool_parent_index(
    tool_message: &Message,
    visible: &[Message],
    assistant_indices_by_id: &HashMap<i64, usize>,
    assistant_indices_by_operation: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(parent_message_id) = tool_message.metadata.parent_message_id {
        if let Some(index) = assistant_indices_by_id.get(&parent_message_id).copied() {
            return Some(index);
        }
    }

    for operation_id in tool_message
        .parts
        .iter()
        .filter_map(|part| part.operation_id.as_deref())
    {
        if let Some(index) = assistant_indices_by_operation.get(operation_id).copied() {
            return Some(index);
        }
    }

    visible
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role == agena::role::Role::Assistant)
        .map(|(index, _)| index)
}

fn paginate_visible_messages(
    messages: &[Message],
    cursor: Option<MessageCursor>,
    limit: u64,
) -> (Vec<Message>, bool, Option<(i64, i64)>) {
    let mut filtered = messages
        .iter()
        .filter(|message| match cursor {
            Some(cursor) => {
                let key = (message.created_at.timestamp_millis(), message.id);
                key > (cursor.created_at_ms, cursor.id)
            }
            None => true,
        })
        .cloned()
        .collect::<Vec<_>>();

    let has_more = filtered.len() > limit as usize;
    filtered.truncate(limit as usize);
    let next_cursor = if has_more {
        filtered
            .last()
            .map(|message| (message.created_at.timestamp_millis(), message.id))
    } else {
        None
    };

    (filtered, has_more, next_cursor)
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

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod visible_message_projection_tests {
    use super::*;
    use agena::message::{MessageMetadata, ToolExecutionPart, ToolInvocation};
    use agena::role::Role;
    use chrono::Utc;

    fn assistant_with_tool_call(message_id: i64, operation_id: &str) -> Message {
        let created_at = Utc::now();
        let mut part = MessagePart::with_content(
            10,
            message_id,
            created_at,
            ExecutionStatus::Pending,
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: 0,
                invocation: ToolInvocation::new("read", Default::default()),
                title: "read".to_string(),
                lifecycle: Default::default(),
            }),
        );
        part.operation_id = Some(operation_id.to_string());
        Message {
            id: message_id,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts: vec![part],
            created_at,
            metadata: MessageMetadata::default(),
            usage: None,
            finish: Some("tool_calls".to_string()),
        }
    }

    fn tool_result_message(
        message_id: i64,
        parent_message_id: i64,
        operation_id: &str,
        output_text: &str,
    ) -> Message {
        let created_at = Utc::now();
        let mut part = MessagePart::with_content(
            20,
            message_id,
            created_at,
            ExecutionStatus::Completed,
            PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 0,
                invocation: ToolInvocation::new("read", Default::default()),
                output_text: output_text.to_string(),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: Default::default(),
                lifecycle: Default::default(),
            }),
        );
        part.operation_id = Some(operation_id.to_string());
        Message {
            id: message_id,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![part],
            created_at,
            metadata: MessageMetadata {
                parent_message_id: Some(parent_message_id),
                ..MessageMetadata::default()
            },
            usage: None,
            finish: None,
        }
    }

    #[test]
    fn visible_projection_merges_tool_messages_into_assistant() {
        let projection = project_visible_messages(vec![
            assistant_with_tool_call(1, "call_read_1"),
            tool_result_message(2, 1, "call_read_1", "README body"),
        ]);

        assert_eq!(projection.messages.len(), 1);
        assert_eq!(projection.hidden_message_aliases.get(&2), Some(&1));

        let message = &projection.messages[0];
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.parts.len(), 2);
        assert_eq!(message.parts[1].message_id, 1);
        assert_eq!(
            message.parts[1].operation_id.as_deref(),
            Some("call_read_1")
        );
        assert!(matches!(
            message.parts[1].content.as_ref(),
            Some(PartContent::ToolExecution(ToolExecutionPart::Completed { output_text, .. }))
                if output_text == "README body"
        ));
    }
}

fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> ApiResult<String> {
    Ok(git_output_with_status(workspace_root, args, &[0])?
        .trim()
        .to_string())
}

fn git_output_with_status<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
    ok_statuses: &[i32],
) -> ApiResult<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .map_err(|error| {
            ApiError::internal(format!("failed to execute git {:?}: {}", args, error))
        })?;
    let code = output.status.code().unwrap_or_default();
    if !ok_statuses.contains(&code) {
        return Err(ApiError::internal(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn untracked_files_from_status(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| line.strip_prefix("?? ").map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn git_untracked_patch(workspace_root: &Path, file: &str) -> ApiResult<String> {
    #[cfg(windows)]
    let null_path = "NUL";
    #[cfg(not(windows))]
    let null_path = "/dev/null";

    git_output_with_status(
        workspace_root,
        [
            "diff",
            "--no-index",
            "--binary",
            "--no-ext-diff",
            "--",
            null_path,
            file,
        ],
        &[0, 1],
    )
}

fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
    (ahead, behind)
}

fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
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

pub async fn list_scheduled_jobs(manager: &SessionManager) -> Vec<agena_scheduler::ScheduledJob> {
    let executor = manager.tool_executor();
    let Some(scheduler) = executor.scheduler().cloned() else {
        return Vec::new();
    };
    scheduler.list().await
}

fn session_automation_resource(
    jobs: &[agena_scheduler::ScheduledJob],
    session_id: i64,
) -> Option<SessionAutomationResource> {
    let mut jobs = jobs
        .iter()
        .filter(|job| job.owner_session_id == Some(session_id))
        .cloned()
        .collect::<Vec<_>>();
    if jobs.is_empty() {
        return None;
    }
    sort_jobs_for_display(&mut jobs);
    Some(SessionAutomationResource {
        job_count: jobs.len(),
        latest_job: jobs.into_iter().next().map(scheduled_job_resource),
    })
}

pub fn sort_jobs_for_display(jobs: &mut [agena_scheduler::ScheduledJob]) {
    jobs.sort_by(|left, right| {
        let left_last_run = left
            .last_run
            .as_ref()
            .map(|run| run.triggered_at.timestamp_millis());
        let right_last_run = right
            .last_run
            .as_ref()
            .map(|run| run.triggered_at.timestamp_millis());
        right_last_run
            .cmp(&left_last_run)
            .then_with(|| left.next_fire_at.cmp(&right.next_fire_at))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub fn scheduled_job_resource(job: agena_scheduler::ScheduledJob) -> ScheduledJobResource {
    let (kind, expression, at) = match job.kind {
        agena_scheduler::JobKind::Cron { expression, .. } => {
            ("cron".to_string(), Some(expression), None)
        }
        agena_scheduler::JobKind::Once { at } => ("once".to_string(), None, Some(at)),
    };
    ScheduledJobResource {
        id: job.id.to_string(),
        kind,
        expression,
        at,
        prompt: job.prompt,
        owner_session_id: job.owner_session_id,
        next_fire_at: job.next_fire_at,
        last_fired_at: job.last_fired_at,
        last_run: job.last_run.map(scheduled_job_run_resource),
    }
}

fn scheduled_job_run_resource(run: agena_scheduler::JobRunRecord) -> ScheduledJobRunResource {
    ScheduledJobRunResource {
        triggered_at: run.triggered_at,
        finished_at: run.finished_at,
        status: run.status,
        session_id: run.session_id,
        error_message: run.error_message,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ahead_behind_interprets_git_rev_list_counts() {
        assert_eq!(parse_ahead_behind(Some("0\t0")), (Some(0), Some(0)));
        assert_eq!(parse_ahead_behind(Some("2 5")), (Some(5), Some(2)));
        assert_eq!(parse_ahead_behind(None), (None, None));
    }

    #[test]
    fn summarize_git_status_counts_porcelain_entries() {
        let status = "M  staged.txt\n M unstaged.txt\nMM both.txt\n?? new.txt\n";
        assert_eq!(summarize_git_status(status), (2, 2, 1, 4));
    }
}
