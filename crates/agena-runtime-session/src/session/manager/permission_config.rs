//! Validate permission writes in the same scope used by session execution.
use super::*;
use agena_domain::PermissionConfig;
use agena_runtime_contracts::authorization::{
    validate_permission_config, validate_permission_config_for_workspace,
};

impl SessionManager {
    pub(super) async fn validate_session_permission_configuration(
        &self,
        workspace_id: Option<i64>,
        configuration: &SessionConfiguration,
    ) -> Result<(), AppError> {
        let state = if workspace_id.is_some() {
            self.state.load_full()
        } else {
            self.global_state.load_full()
        };
        let workspace = match (
            workspace_id,
            configuration.effective_workspace_root.as_ref(),
        ) {
            (Some(_), Some(root)) => Some(root.clone()),
            (Some(id), None) => Some(self.workspace_root_by_id(id).await?),
            (None, _) => None,
        };
        validate_configuration_permissions(
            &state.config.permission,
            configuration,
            workspace.as_deref(),
            state.config.sandbox_enabled,
        )
        .map_err(AppError::Config)
    }

    pub(super) async fn validate_existing_session_permissions(
        &self,
        session: SessionId,
    ) -> Result<(), AppError> {
        let metadata = self
            .kernel()
            .reader
            .metadata(session)
            .await
            .map_err(kernel_error)?;
        let configuration = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        self.validate_session_permission_configuration(metadata.workspace_id, &configuration)
            .await
    }

    pub(super) async fn session_archive_permission_validator(
        &self,
        workspace_id: i64,
    ) -> Result<SessionImportValidator, AppError> {
        let workspace = self.workspace_root_by_id(workspace_id).await?;
        let state = self.state.load_full();
        let base = state.config.permission.clone();
        let sandbox_enabled = state.config.sandbox_enabled;
        Ok(Arc::new(move |configuration| {
            let root = configuration
                .effective_workspace_root
                .as_deref()
                .unwrap_or(&workspace);
            validate_configuration_permissions(&base, configuration, Some(root), sandbox_enabled)
        }))
    }
}

fn validate_configuration_permissions(
    base: &PermissionConfig,
    configuration: &SessionConfiguration,
    workspace: Option<&std::path::Path>,
    sandbox_enabled: bool,
) -> Result<(), String> {
    let effective = base.merged_with(&configuration.permission);
    for (field, permission) in [
        ("session.permission", &configuration.permission),
        (
            "session.permission_ceiling",
            &configuration.permission_ceiling,
        ),
        ("effective session.permission", &effective),
    ] {
        validate_for_scope(permission, workspace).map_err(|reason| format!("{field}: {reason}"))?;
    }
    if workspace.is_none() {
        validate_global_scope_overrides(
            &configuration.permission,
            &configuration.permission_ceiling,
        )
        .map_err(|reason| format!("session.permission: {reason}"))?;
    }
    if sandbox_enabled {
        use agena_runtime_contracts::{
            authorization::ExecutionPrincipal,
            permission::{PermissionPolicy, ToolPermissionPolicy},
        };
        // Compile the same full policy used for shell/process execution. The
        // ceiling remains an independent intersection, including on imports.
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&effective)
        .and_then(|principal| {
            principal.try_apply_permission_ceiling(&configuration.permission_ceiling)
        })
        .map_err(|error| format!("session sandbox permission: {error}"))?;
        principal
            .sandbox_permission_snapshot(workspace.unwrap_or_else(|| std::path::Path::new("")))
            .filesystem_policy()
            .map_err(|error| format!("session sandbox permission: {error}"))?;
    }
    Ok(())
}

fn validate_for_scope(
    permission: &PermissionConfig,
    workspace: Option<&std::path::Path>,
) -> Result<(), String> {
    if let Some(workspace) = workspace {
        validate_permission_config_for_workspace(permission, workspace)
            .map_err(|error| error.to_string())
    } else {
        validate_permission_config(permission).map_err(|error| error.to_string())?;
        // Global executors clear workspace_root. The matcher and sandbox
        // snapshot therefore leave workspace/state templates inactive; they
        // must never be resolved against the server's project or instance root.
        // Keep the stored templates, but validate every active host resource.
        let mut active = permission.clone();
        if let Some(path) = &mut active.path {
            path.rules
                .retain(|pattern, _| inactive_global_alias(pattern).is_none());
        }
        validate_permission_config_for_workspace(&active, std::path::Path::new(""))
            .map_err(|error| error.to_string())
    }
}

fn inactive_global_alias(pattern: &str) -> Option<&'static str> {
    let canonical = agena_domain::canonical_permission_path(pattern).ok()?;
    ["<workspace>", "<state>"].into_iter().find(|alias| {
        canonical == *alias
            || canonical
                .strip_prefix(*alias)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn validate_global_scope_overrides(
    permission: &PermissionConfig,
    ceiling: &PermissionConfig,
) -> Result<(), String> {
    let Some(path) = &permission.path else {
        return Ok(());
    };
    for (pattern, mode) in &path.rules {
        let Some(alias) = inactive_global_alias(pattern) else {
            continue;
        };
        let canonical = agena_domain::canonical_permission_path(pattern)?;
        // Delegated and managed global sessions copy the parent's effective
        // policy together with its immutable ceiling. Preserve those templates,
        // while rejecting a newly authored rule that could never take effect.
        let inherited = ceiling.path.as_ref().is_some_and(|path| {
            path.rules
                .iter()
                .any(|(inherited_pattern, inherited_mode)| {
                    inherited_mode == mode
                        && agena_domain::canonical_permission_path(inherited_pattern)
                            .is_ok_and(|inherited| inherited == canonical)
                })
        });
        if !inherited {
            return Err(format!(
                "global sessions have no {alias}; {pattern:?} requires an owning workspace; use an absolute path, <home>, or a task-specific <tmp> directory"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod sandbox_mode_tests {
    use super::*;

    fn workspace() -> std::path::PathBuf {
        // Saving validates prospective roots without creating them.
        std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("agena-session-mode-test-{}", std::process::id()))
    }

    fn configuration(permission: serde_json::Value) -> SessionConfiguration {
        SessionConfiguration {
            permission: serde_json::from_value(permission).unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn broad_defaults_and_nested_modes_validate_with_or_without_sandbox() {
        let workspace = workspace();
        let base = PermissionConfig::global_default();
        for permission in [
            serde_json::json!({"path": {"workspace":"readwrite", "external":"readonly"}}),
            serde_json::json!({"path": {"rules": {"<workspace>/output":"readwrite"}}}),
        ] {
            let configuration = configuration(permission);
            for sandbox_enabled in [true, false] {
                validate_configuration_permissions(
                    &base,
                    &configuration,
                    Some(&workspace),
                    sandbox_enabled,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn separate_roots_compile_with_an_independent_session_ceiling() {
        let workspace = workspace();
        let base = PermissionConfig::global_default();
        let mut configuration = configuration(serde_json::json!({
            "path": {
                "workspace": "deny",
                "external": "deny",
                "rules": {
                    "<workspace>/input": "readonly",
                    "<workspace>/output": "readwrite"
                }
            }
        }));
        configuration.permission_ceiling = serde_json::from_value(serde_json::json!({
            "path": {"rules": {"<workspace>/output": "deny"}}
        }))
        .unwrap();
        validate_configuration_permissions(&base, &configuration, Some(&workspace), true).unwrap();
    }
}
