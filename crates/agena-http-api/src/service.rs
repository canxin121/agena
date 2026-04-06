use std::{collections::HashMap, path::Path, sync::Arc};

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
        ExecutionStatus, MessageMetadata, MessagePart, MessagePartSummary, PartContent,
        PermissionRequestPart, UserInputRequest, UserInputRequestPart,
    },
    model::ModelRef,
    permission::PermissionMode,
    provider::ProviderRegistry,
    session::{Session, SessionEventRecord},
};

#[cfg(test)]
use agena::{
    db::{crud::message as message_crud, tx::with_transaction_and_effects},
    message::{Message, MessageStatus},
};

use super::{
    dto::{
        MessageListQuery, MessageResource, PartLoadMode, PermissionRuleListQuery,
        PermissionRuleResource, PermissionRuleWriteRequest, SessionCreateRequest,
        SessionEventListQuery, SessionExecutionResource, SessionReplaceRequest, SessionResource,
        SessionRunOptionsRequest, SessionRunState, WorkspaceListQuery, WorkspaceResolveRequest,
        WorkspaceResource, WorkspaceWriteRequest,
    },
    error::ApiError,
    pagination::{
        PageInfo, PageOrder, PaginatedResponse, decode_cursor, encode_cursor, normalize_limit,
    },
};

#[cfg(test)]
use super::dto::{MessagePartWriteRequest, MessageWriteRequest};

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

#[derive(Debug, Clone, FromQueryResult)]
struct MessagePartCountRow {
    message_id: i64,
    part_count: i64,
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

        Ok(build_page(
            items,
            has_more,
            next_cursor,
            PageOrder::Desc,
            limit,
        )?)
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

        Ok(build_page(
            resources,
            has_more,
            next_cursor,
            PageOrder::Desc,
            limit,
        )?)
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
            .session_resources_from_models(&[existing.clone()])
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
        session_id: i64,
        query: SessionEventListQuery,
    ) -> ApiResult<PaginatedResponse<SessionEventRecord>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_cursor::<EventCursor>)
            .transpose()?;
        let mut statement = entities::session_event::Entity::find()
            .filter(entities::session_event::Column::SessionId.eq(session_id))
            .order_by_desc(entities::session_event::Column::Seq)
            .order_by_desc(entities::session_event::Column::Id);
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::session_event::Column::Seq.lt(cursor.seq))
                    .add(
                        Condition::all()
                            .add(entities::session_event::Column::Seq.eq(cursor.seq))
                            .add(entities::session_event::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (mut slice, has_more) = trim_page(rows, limit)?;
        let next_cursor = slice.last().map(|row| EventCursor {
            seq: row.seq,
            id: row.id,
        });
        slice.reverse();
        let items = slice
            .into_iter()
            .map(map_session_event_record)
            .collect::<ApiResult<Vec<_>>>()?;

        Ok(build_page(
            items,
            has_more,
            next_cursor,
            PageOrder::Asc,
            limit,
        )?)
    }

    pub async fn list_messages(
        &self,
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
        let mut statement = entities::message::Entity::find()
            .filter(entities::message::Column::SessionId.eq(session_id))
            .order_by_desc(entities::message::Column::CreatedAtMs)
            .order_by_desc(entities::message::Column::Id);
        if let Some(cursor) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(entities::message::Column::CreatedAtMs.lt(cursor.created_at_ms))
                    .add(
                        Condition::all()
                            .add(entities::message::Column::CreatedAtMs.eq(cursor.created_at_ms))
                            .add(entities::message::Column::Id.lt(cursor.id)),
                    ),
            );
        }

        let rows = statement
            .limit(limit + 1)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;
        let (mut slice, has_more) = trim_page(rows, limit)?;
        let next_cursor = slice.last().map(|row| MessageCursor {
            created_at_ms: row.created_at_ms,
            id: row.id,
        });
        slice.reverse();
        let items = self
            .message_resources_from_models(slice.as_slice(), query.parts)
            .await?;

        Ok(build_page(
            items,
            has_more,
            next_cursor,
            PageOrder::Asc,
            limit,
        )?)
    }

    pub async fn get_message(
        &self,
        message_id: i64,
        parts: PartLoadMode,
    ) -> ApiResult<Option<MessageResource>> {
        let Some(row) = entities::message::Entity::find_by_id(message_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };
        let mut items = self.message_resources_from_models(&[row], parts).await?;
        Ok(items.pop())
    }

    #[cfg(test)]
    pub async fn create_message(
        &self,
        session_id: i64,
        request: MessageWriteRequest,
    ) -> ApiResult<MessageResource> {
        self.ensure_session_exists(session_id).await?;
        let message = build_message_from_request(0, request);
        let message_id = with_transaction_and_effects(self.db.as_ref(), |txn, _effects| {
            let message = message.clone();
            Box::pin(async move {
                let persisted =
                    message_crud::insert_message_with_parts(txn, session_id, &message).await?;
                let session_runtime = session_crud::get_session_by_id(txn, session_id)
                    .await?
                    .and_then(|session| session.runtime_state)
                    .unwrap_or_default();
                session_crud::touch_session_updated_at(txn, session_id, session_runtime).await?;
                Ok(persisted.id)
            })
        })
        .await
        .map_err(db_error)?;

        self.get_message(message_id, PartLoadMode::Full)
            .await?
            .ok_or_else(|| ApiError::internal("created message could not be loaded"))
    }

    pub async fn list_message_parts(
        &self,
        message_id: i64,
        mode: PartLoadMode,
    ) -> ApiResult<Vec<MessagePart>> {
        let _ = self.ensure_message_model(message_id).await?;
        self.load_parts_for_message_ids(&[message_id], mode)
            .await
            .map(|mut map| map.remove(&message_id).unwrap_or_default())
    }

    pub async fn get_message_part(&self, part_id: i64) -> ApiResult<Option<MessagePart>> {
        agena::db::crud::message_part::get_message_part_with_detail(self.db.as_ref(), part_id)
            .await
            .map_err(db_error)
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

        Ok(build_page(
            items,
            has_more,
            next_cursor,
            PageOrder::Desc,
            limit,
        )?)
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

    pub async fn latest_session_event_seq(&self, session_id: i64) -> ApiResult<Option<i64>> {
        self.ensure_session_exists(session_id).await?;
        entities::session_event::Entity::find()
            .select_only()
            .column(entities::session_event::Column::Seq)
            .filter(entities::session_event::Column::SessionId.eq(session_id))
            .order_by_desc(entities::session_event::Column::Seq)
            .order_by_desc(entities::session_event::Column::Id)
            .into_tuple::<i64>()
            .one(self.db.as_ref())
            .await
            .map_err(db_error)
    }

    pub async fn list_session_events_after(
        &self,
        session_id: i64,
        after_seq: i64,
        limit: Option<u64>,
    ) -> ApiResult<Vec<SessionEventRecord>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(limit);
        entities::session_event::Entity::find()
            .filter(entities::session_event::Column::SessionId.eq(session_id))
            .filter(entities::session_event::Column::Seq.gt(after_seq))
            .order_by_asc(entities::session_event::Column::Seq)
            .order_by_asc(entities::session_event::Column::Id)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(map_session_event_record)
            .collect()
    }

    pub async fn resolve_run_options(
        &self,
        provider_registry: &ProviderRegistry,
        session_id: i64,
        request: SessionRunOptionsRequest,
    ) -> ApiResult<agena::session::SessionRunOptions> {
        self.ensure_session_exists(session_id).await?;

        let model = match request.model {
            Some(model) => {
                ensure_provider_exists(provider_registry, &model)?;
                model
            }
            None => match self.infer_session_model(session_id).await? {
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
        session: &Session,
    ) -> ApiResult<SessionExecutionResource> {
        let session_resource = self.get_session(session.id).await?.ok_or_else(|| {
            ApiError::internal("session disappeared while loading execution state")
        })?;

        Ok(SessionExecutionResource {
            session: session_resource,
            blocked: session.blocked(),
            run_state: SessionRunState::from(session.status()),
            latest_event_seq: self.latest_session_event_seq(session.id).await?,
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

    async fn ensure_message_model(&self, message_id: i64) -> ApiResult<entities::message::Model> {
        entities::message::Entity::find_by_id(message_id)
            .one(self.db.as_ref())
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found(format!("message not found: {message_id}")))
    }

    async fn infer_session_model(&self, session_id: i64) -> ApiResult<Option<ModelRef>> {
        let rows = entities::message::Entity::find()
            .filter(entities::message::Column::SessionId.eq(session_id))
            .order_by_desc(entities::message::Column::CreatedAtMs)
            .order_by_desc(entities::message::Column::Id)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;

        for row in rows {
            let provider_id = row.model_provider_id.trim();
            let model_id = row.model_id.trim();
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
        let message_stats =
            session_crud::session_message_stats_by_session_ids(self.db.as_ref(), &session_ids)
                .await
                .map_err(db_error)?;
        let child_counts =
            session_crud::child_session_counts_by_parent_ids(self.db.as_ref(), &session_ids)
                .await
                .map_err(db_error)?;

        models
            .iter()
            .map(|model| session_resource(model, &message_stats, &child_counts))
            .collect()
    }

    async fn message_resources_from_models(
        &self,
        models: &[entities::message::Model],
        mode: PartLoadMode,
    ) -> ApiResult<Vec<MessageResource>> {
        if models.is_empty() {
            return Ok(Vec::new());
        }

        let message_ids = models.iter().map(|row| row.id).collect::<Vec<_>>();
        let part_counts = self.message_part_counts(&message_ids).await?;
        let mut parts_by_message = if mode == PartLoadMode::None {
            HashMap::new()
        } else {
            self.load_parts_for_message_ids(&message_ids, mode).await?
        };

        models
            .iter()
            .map(|row| {
                Ok(MessageResource {
                    id: row.id,
                    session_id: row.session_id,
                    role: row.role,
                    state: row.status,
                    created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                    updated_at: timestamp_millis_to_utc(row.updated_at_ms)?,
                    metadata: MessageMetadata {
                        source: row.source,
                        parent_message_id: row.parent_message_id,
                        generated_by_call_id: row.generated_by_call_id,
                        model_provider_id: row.model_provider_id.clone(),
                        model_id: row.model_id.clone(),
                        tags: message_tags_from_json(row.tags.as_ref()),
                    },
                    usage: row.usage.clone(),
                    finish: row.finish.clone(),
                    part_count: part_counts.get(&row.id).copied().unwrap_or_default(),
                    parts: match mode {
                        PartLoadMode::None => None,
                        PartLoadMode::Summary | PartLoadMode::Full => {
                            Some(parts_by_message.remove(&row.id).unwrap_or_default())
                        }
                    },
                })
            })
            .collect()
    }

    async fn message_part_counts(&self, message_ids: &[i64]) -> ApiResult<HashMap<i64, u64>> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }

        entities::message_part::Entity::find()
            .select_only()
            .column_as(entities::message_part::Column::MessageId, "message_id")
            .column_as(entities::message_part::Column::Id.count(), "part_count")
            .filter(entities::message_part::Column::MessageId.is_in(message_ids.iter().copied()))
            .group_by(entities::message_part::Column::MessageId)
            .into_model::<MessagePartCountRow>()
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.message_id,
                    u64::try_from(row.part_count).map_err(|_| {
                        ApiError::internal(format!(
                            "invalid negative part count for message {}",
                            row.message_id
                        ))
                    })?,
                ))
            })
            .collect()
    }

    async fn load_parts_for_message_ids(
        &self,
        message_ids: &[i64],
        mode: PartLoadMode,
    ) -> ApiResult<HashMap<i64, Vec<MessagePart>>> {
        if message_ids.is_empty() || mode == PartLoadMode::None {
            return Ok(HashMap::new());
        }

        let part_rows = entities::message_part::Entity::find()
            .filter(entities::message_part::Column::MessageId.is_in(message_ids.iter().copied()))
            .order_by_asc(entities::message_part::Column::MessageId)
            .order_by_asc(entities::message_part::Column::PartIndex)
            .all(self.db.as_ref())
            .await
            .map_err(db_error)?;

        let detail_ids = if mode == PartLoadMode::Full {
            part_rows
                .iter()
                .filter(|row| row.has_detail)
                .map(|row| row.id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let detail_map = if detail_ids.is_empty() {
            HashMap::new()
        } else {
            entities::message_part_detail::Entity::find()
                .filter(entities::message_part_detail::Column::PartId.is_in(detail_ids))
                .all(self.db.as_ref())
                .await
                .map_err(db_error)?
                .into_iter()
                .map(|row| (row.part_id, row.detail))
                .collect::<HashMap<_, _>>()
        };

        let mut parts_by_message = HashMap::<i64, Vec<MessagePart>>::new();
        for row in part_rows {
            let summary = map_message_part_summary(&row)?;
            let detail = if mode == PartLoadMode::Full && summary.has_detail {
                detail_map.get(&summary.id).cloned()
            } else {
                None
            };
            parts_by_message
                .entry(summary.message_id)
                .or_default()
                .push(MessagePart::from_summary(summary, detail));
        }

        Ok(parts_by_message)
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

fn map_message_part_summary(row: &entities::message_part::Model) -> ApiResult<MessagePartSummary> {
    Ok(MessagePartSummary {
        id: row.id,
        message_id: row.message_id,
        part_index: row.part_index,
        status: row.status,
        kind: row.kind,
        name: row.name.clone(),
        summary: row.summary_text.clone(),
        has_detail: row.has_detail,
        operation_id: row.operation_id.clone(),
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
    })
}

fn map_session_event_record(row: entities::session_event::Model) -> ApiResult<SessionEventRecord> {
    Ok(SessionEventRecord {
        event_id: Some(row.id),
        session_id: row.session_id,
        seq: row.seq,
        event_type: row.event_type,
        payload: row.payload,
        causation_id: row.causation_id,
        correlation_id: row.correlation_id,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
    })
}

#[cfg(test)]
fn build_message_from_request(id: i64, request: MessageWriteRequest) -> Message {
    let created_at = request.created_at.unwrap_or_else(Utc::now);
    Message {
        id,
        role: request.role,
        state: request.state.unwrap_or(MessageStatus::Completed),
        parts: build_message_parts(request.parts, created_at),
        created_at,
        metadata: request.metadata.unwrap_or_default(),
        usage: request.usage,
        finish: request.finish,
    }
}

#[cfg(test)]
fn build_message_parts(
    parts: Vec<MessagePartWriteRequest>,
    default_created_at: DateTime<Utc>,
) -> Vec<MessagePart> {
    parts
        .into_iter()
        .enumerate()
        .map(|(idx, input)| {
            let mut part = MessagePart::with_content(
                0,
                0,
                input.created_at.unwrap_or(default_created_at),
                input
                    .status
                    .unwrap_or(agena::message::ExecutionStatus::Completed),
                input.content,
            );
            part.part_index = idx as i32;
            part.operation_id = input.operation_id;
            part
        })
        .collect()
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

fn message_tags_from_json(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agena::{
        db::init_schema,
        message::{MessageSource, PartContent},
        role::Role,
    };
    use sea_orm::Database;

    use super::*;
    use crate::dto::{MessageWriteRequest, WorkspaceWriteRequest};

    async fn build_service() -> ApiService {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("sqlite memory db should connect");
        init_schema(&db).await.expect("schema should initialize");
        ApiService::new(Arc::new(db))
    }

    #[tokio::test]
    async fn list_messages_preserves_metadata_tags() {
        let service = build_service().await;
        let workspace = service
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/agena-http-api-tags".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = service
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "tags".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        let created = service
            .create_message(
                session.id,
                MessageWriteRequest {
                    role: Role::System,
                    state: None,
                    parts: vec![MessagePartWriteRequest {
                        content: PartContent::text("summary"),
                        status: None,
                        operation_id: None,
                        created_at: None,
                    }],
                    metadata: Some(MessageMetadata {
                        source: MessageSource::System,
                        parent_message_id: None,
                        generated_by_call_id: None,
                        model_provider_id: "openai".to_string(),
                        model_id: "gpt-5".to_string(),
                        tags: vec!["prompt_summary".to_string(), "prompt_compacted".to_string()],
                    }),
                    usage: None,
                    finish: None,
                    created_at: None,
                },
            )
            .await
            .expect("message should be created");

        let page = service
            .list_messages(
                session.id,
                MessageListQuery {
                    cursor: None,
                    limit: Some(10),
                    parts: PartLoadMode::Summary,
                },
            )
            .await
            .expect("messages should load");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, created.id);
        assert_eq!(
            page.items[0].metadata.tags,
            vec!["prompt_summary".to_string(), "prompt_compacted".to_string()]
        );
    }
}
