//! Instance and lifecycle commands exposed to trusted local plugins.

use super::*;
use agena_domain::{PermissionAction, PermissionMode, PermissionScope};
use agena_session_api::commands::{Command, UpsertPermissionRuleParams};
use agena_storage::PersistedPermissionRule;
use agena_storage::kernel::{RequestId, SessionConfiguration, SessionTitleState};

use super::session_queries::{permission_rule_resource, workspace_resource};

impl RuntimeHostClient {
    pub(super) async fn run_management_session_command(
        &self,
        command: Command,
    ) -> Result<serde_json::Value, PluginError> {
        let services = self.runtime()?.application_services();
        let mut session_id = command.session_id();
        let data = match command {
            Command::CreateWorkspace(params) => {
                let repositories = self.application_repositories()?;
                let path = workspace_identity(params.path).await?;
                if repositories
                    .workspace
                    .lookup_id(&path)
                    .await
                    .map_err(plugin_error)?
                    .is_some()
                {
                    return Err(PluginError::invalid_params(
                        "a workspace already uses this path",
                    ));
                }
                let workspace = repositories
                    .workspace
                    .create(path)
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(workspace_resource(&workspace, Some(0))?)
                    .map_err(plugin_error)?
            }
            Command::ResolveWorkspace(params) => {
                let repositories = self.application_repositories()?;
                let path = workspace_identity(params.path).await?;
                let id = match repositories
                    .workspace
                    .lookup_id(&path)
                    .await
                    .map_err(plugin_error)?
                {
                    Some(id) => id,
                    None if params.create_if_missing => repositories
                        .workspace
                        .ensure_id(&path)
                        .await
                        .map_err(plugin_error)?,
                    None => return Err(PluginError::invalid_params("workspace not found")),
                };
                let workspace = repositories
                    .workspace
                    .get(id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                serde_json::to_value(workspace_resource(&workspace, None)?).map_err(plugin_error)?
            }
            Command::UpdateWorkspace(params) => {
                let repositories = self.application_repositories()?;
                let path = workspace_identity(params.path).await?;
                if let Some(id) = repositories
                    .workspace
                    .lookup_id(&path)
                    .await
                    .map_err(plugin_error)?
                    && id != params.workspace_id
                {
                    return Err(PluginError::invalid_params(
                        "a workspace already uses this path",
                    ));
                }
                let workspace = repositories
                    .workspace
                    .update_path(params.workspace_id, path)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                serde_json::to_value(workspace_resource(&workspace, None)?).map_err(plugin_error)?
            }
            Command::DeleteWorkspace(params) => {
                self.application_repositories()?
                    .workspace
                    .delete(params.workspace_id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                serde_json::json!({ "id": params.workspace_id })
            }
            Command::CreateSession(params) => {
                let workspace = self
                    .application_repositories()?
                    .workspace
                    .get(params.workspace_id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                let manager = self.session_manager()?;
                let configuration = SessionConfiguration {
                    parent_session_id: params.parent_id.map(kernel_session_id).transpose()?,
                    relation_kind: if params.parent_id.is_some() {
                        agena_domain::SessionRelationKind::Child
                    } else {
                        agena_domain::SessionRelationKind::Root
                    },
                    title_state: if params.auto_title {
                        SessionTitleState::automatic()
                    } else {
                        Default::default()
                    },
                    effective_workspace_root: Some(PathBuf::from(workspace.path)),
                    ..Default::default()
                };
                let created = manager
                    .session_kernel()
                    .sessions
                    .create_configured(
                        RequestId::new(),
                        params.workspace_id,
                        params.title,
                        configuration,
                    )
                    .await
                    .map_err(plugin_error)?;
                let id = created.value.id.get();
                session_id = Some(id);
                let session = manager.get_session(id).await.map_err(plugin_error)?;
                serde_json::to_value(session_resource_from_session(&session))
                    .map_err(plugin_error)?
            }
            Command::DeleteSession(params) => {
                let id = kernel_session_id(params.session_id)?;
                let manager = self.session_manager()?;
                if let Some(expected) = params.expected_version {
                    let metadata = manager
                        .session_kernel()
                        .reader
                        .metadata(id)
                        .await
                        .map_err(plugin_error)?;
                    if metadata.metadata_version.get() != expected {
                        return Err(PluginError::invalid_params(format!(
                            "session version mismatch: expected {expected}, current {}",
                            metadata.metadata_version.get()
                        )));
                    }
                }
                manager
                    .delete_session(params.session_id)
                    .await
                    .map_err(plugin_error)?;
                serde_json::json!({ "id": params.session_id })
            }
            Command::UpsertPermissionRule(params) => {
                let rule = self.permission_rule(params).await?;
                let (stored, _) = self
                    .application_repositories()?
                    .permission_rules
                    .upsert(&rule)
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(permission_rule_resource(&stored)?).map_err(plugin_error)?
            }
            Command::ReplacePermissionRule(params) => {
                let rule = self.permission_rule(params.rule).await?;
                let stored = self
                    .application_repositories()?
                    .permission_rules
                    .replace(params.rule_id, &rule)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("permission rule not found"))?;
                serde_json::to_value(permission_rule_resource(&stored)?).map_err(plugin_error)?
            }
            Command::RevokePermissionRule(params) => {
                let stored = self
                    .application_repositories()?
                    .permission_rules
                    .revoke(params.rule_id, params.reason, plugin_operator())
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("permission rule not found"))?;
                serde_json::to_value(permission_rule_resource(&stored)?).map_err(plugin_error)?
            }
            Command::DeletePermissionRule(params) => {
                self.application_repositories()?
                    .permission_rules
                    .delete(params.rule_id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| PluginError::invalid_params("permission rule not found"))?;
                serde_json::json!({ "id": params.rule_id })
            }
            Command::StopActivity(params) => {
                let activity = services
                    .activities
                    .ok_or_else(|| host_unavailable("background activities are not enabled"))?
                    .stop_activity(&params.activity_id)
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(
                    agena_session_api::activity::BackgroundActivityResource::from(&activity),
                )
                .map_err(plugin_error)?
            }
            Command::DismissActivity(params) => {
                let activity = services
                    .activities
                    .ok_or_else(|| host_unavailable("background activities are not enabled"))?
                    .dismiss_activity(&params.activity_id)
                    .map_err(plugin_error)?;
                serde_json::json!({ "id": activity.id })
            }
            Command::ClearFinishedActivities => {
                let count = services
                    .activities
                    .ok_or_else(|| host_unavailable("background activities are not enabled"))?
                    .clear_finished()
                    .await
                    .map_err(plugin_error)?;
                serde_json::json!({ "count": count })
            }
            _ => return Err(PluginError::invalid_params("expected a management command")),
        };
        Ok(serde_json::json!({
            "session_id": session_id,
            "execution_ids": [],
            "data": data,
        }))
    }

    fn application_repositories(
        &self,
    ) -> Result<crate::RuntimeApplicationRepositories, PluginError> {
        self.runtime()?
            .application_services()
            .repositories
            .ok_or_else(|| host_unavailable("application repositories are not enabled"))
    }

    async fn permission_rule(
        &self,
        request: UpsertPermissionRuleParams,
    ) -> Result<PersistedPermissionRule, PluginError> {
        let workspace_root = self
            .runtime()?
            .workspace_root()
            .to_string_lossy()
            .into_owned();
        let scope = match request.scope.as_deref().unwrap_or("workspace") {
            "session" => PermissionScope::Session,
            "workspace" => PermissionScope::Workspace,
            "global" => PermissionScope::Global,
            other => {
                return Err(PluginError::invalid_params(format!(
                    "unsupported permission scope: {other}"
                )));
            }
        };
        let session_id = if scope == PermissionScope::Session {
            Some(
                kernel_session_id(request.session_id.ok_or_else(|| {
                    PluginError::invalid_params("session scope requires a session_id")
                })?)?
                .get(),
            )
        } else {
            None
        };
        let workspace_id = if scope == PermissionScope::Workspace {
            Some(
                self.application_repositories()?
                    .workspace
                    .ensure_id(&workspace_root)
                    .await
                    .map_err(plugin_error)?,
            )
        } else {
            None
        };
        let action = permission_action(&request, &workspace_root)?;
        let mode = match request.mode {
            agena_session_api::resource::PermissionMode::Allow => PermissionMode::Allow,
            agena_session_api::resource::PermissionMode::Auto => PermissionMode::Auto,
            agena_session_api::resource::PermissionMode::Ask => PermissionMode::Ask,
            agena_session_api::resource::PermissionMode::Deny => PermissionMode::Deny,
        };
        Ok(PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key: serde_json::to_string(&action).map_err(plugin_error)?,
            mode,
            scope,
            session_id,
            workspace_id,
            source: "plugin".into(),
            reason: None,
            operator: plugin_operator(),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        })
    }
}

fn plugin_operator() -> Option<String> {
    current_host_callback_context().and_then(|context| context.plugin_id)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn required(value: Option<&str>, name: &str) -> Result<String, PluginError> {
    non_empty(value)
        .map(str::to_owned)
        .ok_or_else(|| PluginError::invalid_params(format!("{name} is required")))
}

fn permission_action(
    request: &UpsertPermissionRuleParams,
    workspace_root: &str,
) -> Result<PermissionAction, PluginError> {
    if let Some(action_key) = non_empty(request.action_key.as_deref()) {
        return serde_json::from_str(action_key)
            .map_err(|error| PluginError::invalid_params_error(&error));
    }
    match request.subject_kind.as_deref() {
        Some("tool") => Ok(PermissionAction::Tool {
            tool_name: required(request.tool_name.as_deref(), "tool_name")?,
            qualifier: non_empty(request.qualifier.as_deref()).map(str::to_owned),
        }),
        Some("path_access") => Ok(PermissionAction::PathAccess {
            access_kind: required(request.path_access_kind.as_deref(), "path_access_kind")?,
            workspace_root: non_empty(request.workspace_root.as_deref())
                .unwrap_or(workspace_root)
                .to_owned(),
            target_path: required(request.target_path.as_deref(), "target_path")?,
        }),
        Some("network_access") => {
            let target = required(
                request
                    .network_target
                    .as_deref()
                    .or(request.network_host.as_deref()),
                "network_target or network_host",
            )?;
            let address = request
                .network_port
                .map(|port| format!("{target}:{port}"))
                .unwrap_or_else(|| target.clone());
            let parsed: agena_domain::NetworkTarget =
                address.parse().map_err(PluginError::invalid_params)?;
            Ok(PermissionAction::NetworkAccess {
                target,
                host: parsed.host().to_owned(),
                port: parsed.port(),
            })
        }
        Some(kind) => Err(PluginError::invalid_params(format!(
            "unsupported permission subject_kind: {kind}"
        ))),
        None => Err(PluginError::invalid_params(
            "permission rule requires either action_key or structured subject fields",
        )),
    }
}

/// Resolve aliases for existing paths and normalize paths registered for later use.
async fn workspace_identity(path: String) -> Result<String, PluginError> {
    use std::path::Component;
    let path = path.trim();
    if path.is_empty() {
        return Err(PluginError::invalid_params(
            "workspace path cannot be empty",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    if normalized.as_os_str().is_empty() {
        normalized.push(".");
    }
    let canonical = tokio::fs::canonicalize(&normalized)
        .await
        .unwrap_or(normalized);
    let mut identity = canonical.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    Ok(identity)
}
