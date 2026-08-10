use agena_storage::PermissionRuleListQuery;

use super::{
    ApplicationError, ApplicationResult, ApplicationService, PageOrder, PaginatedResponse,
    PermissionAction, PermissionRuleCursor, PermissionRuleResource, PermissionRuleWriteCommand,
    PermissionRuleWriteRequest, PermissionScope, PersistedPermissionRule, SearchPaginationQuery,
    api_error_from_app, build_page, decode_cursor, non_empty, normalize_limit,
    timestamp_millis_to_utc, trim_page,
};

impl ApplicationService {
    pub async fn list_permission_rules(
        &self,
        query: SearchPaginationQuery,
    ) -> ApplicationResult<PaginatedResponse<PermissionRuleResource>> {
        let limit = normalize_limit(query.limit());
        let cursor = query
            .cursor()
            .map(decode_cursor::<PermissionRuleCursor>)
            .transpose()?;
        let rows = self
            .permission_rule_repository
            .list(PermissionRuleListQuery {
                search: non_empty(query.search()).map(ToString::to_string),
                before_updated_at_ms: cursor.map(|value| value.updated_at_ms),
                before_id: cursor.map(|value| value.id),
                limit: limit + 1,
            })
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let items = slice
            .iter()
            .map(permission_rule_record_resource)
            .collect::<ApplicationResult<Vec<_>>>()?;
        let next_cursor = slice.last().map(|row| PermissionRuleCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(items, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_permission_rule(
        &self,
        rule_id: i64,
    ) -> ApplicationResult<Option<PermissionRuleResource>> {
        let row = self
            .permission_rule_repository
            .get(rule_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        row.as_ref()
            .map(permission_rule_record_resource)
            .transpose()
    }

    pub async fn create_permission_rule(
        &self,
        request: PermissionRuleWriteRequest,
    ) -> ApplicationResult<PermissionRuleResource> {
        self.create_permission_rule_command(permission_rule_write_command_from_request(
            request,
            "api",
            Some("http_api".to_string()),
            self.workspace_root.as_str(),
        )?)
        .await
    }

    pub async fn create_permission_rule_command(
        &self,
        command: PermissionRuleWriteCommand,
    ) -> ApplicationResult<PermissionRuleResource> {
        let rule = self.persisted_permission_rule_from_command(command).await?;

        let (created, _is_new) = self
            .permission_rule_repository
            .upsert(&rule)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        permission_rule_record_resource(&created)
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        request: PermissionRuleWriteRequest,
    ) -> ApplicationResult<PermissionRuleResource> {
        self.replace_permission_rule_command(
            rule_id,
            permission_rule_write_command_from_request(
                request,
                "api",
                Some("http_api".to_string()),
                self.workspace_root.as_str(),
            )?,
        )
        .await
    }

    pub async fn replace_permission_rule_command(
        &self,
        rule_id: i64,
        command: PermissionRuleWriteCommand,
    ) -> ApplicationResult<PermissionRuleResource> {
        let rule = self.persisted_permission_rule_from_command(command).await?;
        let Some(updated) = self
            .permission_rule_repository
            .replace(rule_id, &rule)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The permission rule was not found.",
                format!("permission rule not found: {rule_id}"),
            ));
        };
        permission_rule_record_resource(&updated)
    }

    pub async fn revoke_permission_rule(
        &self,
        rule_id: i64,
        reason: Option<String>,
    ) -> ApplicationResult<PermissionRuleResource> {
        self.revoke_permission_rule_as(rule_id, reason, Some("http_api".to_string()))
            .await
    }

    pub async fn revoke_permission_rule_as(
        &self,
        rule_id: i64,
        reason: Option<String>,
        operator: Option<String>,
    ) -> ApplicationResult<PermissionRuleResource> {
        let Some(updated) = self
            .permission_rule_repository
            .revoke(rule_id, reason, operator)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The permission rule was not found.",
                format!("permission rule not found: {rule_id}"),
            ));
        };
        permission_rule_record_resource(&updated)
    }

    async fn persisted_permission_rule_from_command(
        &self,
        command: PermissionRuleWriteCommand,
    ) -> ApplicationResult<PersistedPermissionRule> {
        if command.scope == PermissionScope::Session && command.session_id.is_none() {
            return Err(ApplicationError::bad_request(
                "session scope requires a session_id",
            ));
        }
        let workspace_id = if command.scope == PermissionScope::Workspace {
            Some(
                self.workspace_repository
                    .ensure_id(self.workspace_root.as_str())
                    .await
                    .map_err(|error| ApplicationError::internal(error.to_string()))?,
            )
        } else {
            None
        };
        Ok(PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key: serde_json::to_string(&command.action).map_err(api_error_from_app)?,
            mode: command.mode,
            scope: command.scope,
            session_id: (command.scope == PermissionScope::Session)
                .then_some(command.session_id)
                .flatten(),
            workspace_id,
            source: command.source,
            reason: None,
            operator: command.operator,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        })
    }

    pub async fn delete_permission_rule(
        &self,
        rule_id: i64,
    ) -> ApplicationResult<PermissionRuleResource> {
        let Some(existing) = self
            .permission_rule_repository
            .delete(rule_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The permission rule was not found.",
                format!("permission rule not found: {rule_id}"),
            ));
        };

        permission_rule_record_resource(&existing)
    }
}

fn permission_rule_resource(
    row: &agena_storage::PermissionRuleRecord,
) -> ApplicationResult<PermissionRuleResource> {
    let action: PermissionAction =
        serde_json::from_str(row.action_key.as_str()).map_err(api_error_from_app)?;
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
        mode: permission_mode_to_resource(permission_mode_from_string(row.mode.as_str())?),
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

fn permission_rule_record_resource(
    row: &agena_storage::PermissionRuleRecord,
) -> ApplicationResult<PermissionRuleResource> {
    permission_rule_resource(row)
}

fn permission_scope_from_request(value: Option<&str>) -> ApplicationResult<PermissionScope> {
    match value.unwrap_or("workspace") {
        "session" => Ok(PermissionScope::Session),
        "workspace" => Ok(PermissionScope::Workspace),
        "global" => Ok(PermissionScope::Global),
        other => Err(ApplicationError::bad_request_with_diagnostic(
            "The permission scope is not supported.",
            format!("unsupported permission scope: {other}"),
        )),
    }
}

fn permission_action_from_write_request(
    request: &PermissionRuleWriteRequest,
    workspace_root: &str,
) -> ApplicationResult<PermissionAction> {
    if let Some(action_key) = request.action_key.as_deref()
        && !action_key.trim().is_empty()
    {
        return serde_json::from_str(action_key).map_err(api_error_from_app);
    }

    match request.subject_kind.as_deref() {
        Some("tool") => {
            let tool_name = request
                .tool_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApplicationError::bad_request("tool_name is required for tool rule")
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
                    ApplicationError::bad_request(
                        "path_access_kind is required for path_access rule",
                    )
                })?
                .to_string();
            let target_path = request
                .target_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApplicationError::bad_request("target_path is required for path_access rule")
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
                    ApplicationError::bad_request(
                        "network_target or network_host is required for network_access rule",
                    )
                })?
                .to_string();
            let parsed: agena_domain::NetworkTarget = request
                .network_port
                .map(|port| format!("{target}:{port}"))
                .unwrap_or_else(|| target.clone())
                .parse()
                .map_err(|err| {
                    ApplicationError::bad_request_with_diagnostic(
                        "The network target is invalid.",
                        err,
                    )
                })?;
            Ok(PermissionAction::NetworkAccess {
                target,
                host: parsed.host().to_string(),
                port: parsed.port(),
            })
        }
        Some(other) => Err(ApplicationError::bad_request_with_diagnostic(
            "The permission subject type is not supported.",
            format!("unsupported permission subject_kind: {other}"),
        )),
        None => Err(ApplicationError::bad_request(
            "permission rule requires either action_key or structured subject fields",
        )),
    }
}

fn permission_rule_write_command_from_request(
    request: PermissionRuleWriteRequest,
    source: &str,
    operator: Option<String>,
    workspace_root: &str,
) -> ApplicationResult<PermissionRuleWriteCommand> {
    let scope = permission_scope_from_request(request.scope.as_deref())?;
    let action = permission_action_from_write_request(&request, workspace_root)?;
    Ok(PermissionRuleWriteCommand {
        action,
        mode: permission_mode_to_domain(request.mode),
        scope,
        session_id: request.session_id,
        source: source.to_owned(),
        operator,
    })
}

fn permission_mode_from_string(value: &str) -> ApplicationResult<agena_domain::PermissionMode> {
    match value {
        "allow" => Ok(agena_domain::PermissionMode::Allow),
        "auto" => Ok(agena_domain::PermissionMode::Auto),
        "ask" => Ok(agena_domain::PermissionMode::Ask),
        "deny" => Ok(agena_domain::PermissionMode::Deny),
        _ => Err(ApplicationError::internal(format!(
            "invalid permission mode in storage: {value}"
        ))),
    }
}

const fn permission_mode_to_domain(
    mode: agena_api::resource::PermissionMode,
) -> agena_domain::PermissionMode {
    match mode {
        agena_api::resource::PermissionMode::Allow => agena_domain::PermissionMode::Allow,
        agena_api::resource::PermissionMode::Auto => agena_domain::PermissionMode::Auto,
        agena_api::resource::PermissionMode::Ask => agena_domain::PermissionMode::Ask,
        agena_api::resource::PermissionMode::Deny => agena_domain::PermissionMode::Deny,
    }
}

const fn permission_mode_to_resource(
    mode: agena_domain::PermissionMode,
) -> agena_api::resource::PermissionMode {
    match mode {
        agena_domain::PermissionMode::Allow => agena_api::resource::PermissionMode::Allow,
        agena_domain::PermissionMode::Auto => agena_api::resource::PermissionMode::Auto,
        agena_domain::PermissionMode::Ask => agena_api::resource::PermissionMode::Ask,
        agena_domain::PermissionMode::Deny => agena_api::resource::PermissionMode::Deny,
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use agena_domain::PermissionMode;
    use agena_storage::MemoryStore;
    use agena_storage::store::{SessionFacade, SessionStore};
    use agena_storage_sqlite::{SeaPermissionRuleRepository, SeaWorkspaceRepository, SqliteEngine};
    use sea_orm::Database;

    use super::{
        ApplicationService, PermissionAction, PermissionRuleWriteCommand, PermissionScope,
    };

    async fn service() -> ApplicationService {
        let database = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("open in-memory SQLite database"),
        );
        agena_storage_sqlite::initialize_schema(database.as_ref())
            .await
            .expect("initialize test schema");
        let facade: Arc<dyn SessionStore> = Arc::new(SessionFacade::new(
            SqliteEngine::new(Arc::clone(&database)),
            "permissions-test",
            64,
        ));
        ApplicationService::new(
            "/test/workspace",
            Arc::new(MemoryStore::for_workspace(Path::new("/test/workspace"))),
            Arc::new(SeaWorkspaceRepository::new(Arc::clone(&database))),
            Arc::new(SeaPermissionRuleRepository::new(database)),
            facade,
        )
    }

    #[tokio::test]
    async fn transport_neutral_command_preserves_cli_audit_actor() {
        let service = service().await;
        let created = service
            .create_permission_rule_command(PermissionRuleWriteCommand {
                action: PermissionAction::Tool {
                    tool_name: "shell".to_owned(),
                    qualifier: Some("git status".to_owned()),
                },
                mode: PermissionMode::Allow,
                scope: PermissionScope::Workspace,
                session_id: None,
                source: "cli".to_owned(),
                operator: Some("cli".to_owned()),
            })
            .await
            .expect("create CLI permission rule through application service");

        assert_eq!(created.source, "cli");
        assert_eq!(created.operator.as_deref(), Some("cli"));
        assert_eq!(created.mode, PermissionMode::Allow);
        assert!(created.workspace_id.is_some());
    }

    #[tokio::test]
    async fn session_scoped_command_requires_a_session_id_before_writing() {
        let service = service().await;
        let error = service
            .create_permission_rule_command(PermissionRuleWriteCommand {
                action: PermissionAction::Tool {
                    tool_name: "shell".to_owned(),
                    qualifier: None,
                },
                mode: PermissionMode::Ask,
                scope: PermissionScope::Session,
                session_id: None,
                source: "cli".to_owned(),
                operator: Some("cli".to_owned()),
            })
            .await
            .expect_err("session-scoped command without session id must fail");
        assert!(
            error
                .to_string()
                .contains("session scope requires a session_id")
        );
    }
}
