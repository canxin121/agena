use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

impl ApiService {
    pub async fn list_permission_rules(
        &self,
        query: SearchPaginationQuery,
    ) -> ApiResult<PaginatedResponse<PermissionRuleResource>> {
        let limit = normalize_limit(query.limit());
        let cursor = query
            .cursor()
            .map(decode_cursor::<PermissionRuleCursor>)
            .transpose()?;
        let mut statement = entities::permission_rule::Entity::find()
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id);
        if let Some(search) = non_empty(query.search()) {
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
        active.mode = Set(request.mode.as_str().to_owned());
        active.scope = Set(scope.as_str().to_owned());
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
                .ok_or_else(|| ApiError::bad_request("tool_name is required for tool rule"))?
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
            let parsed: agena::permission::NetworkTarget = request
                .network_port
                .map(|port| format!("{target}:{port}"))
                .unwrap_or_else(|| target.clone())
                .parse()
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
use super::{
    ApiError, ApiResult, ApiService, AppError, Condition, EventKind, PageOrder, PaginatedResponse,
    PermissionAction, PermissionMode, PermissionRuleCursor, PermissionRuleEvent,
    PermissionRuleResource, PermissionRuleWriteRequest, PermissionScope, PersistedPermissionRule,
    PublishContext, SearchPaginationQuery, Set, Utc, api_error_from_app, build_page, db_error,
    decode_cursor, entities, non_empty, normalize_limit, permission_rule_crud,
    timestamp_millis_to_utc, trim_page, workspace_crud,
};
