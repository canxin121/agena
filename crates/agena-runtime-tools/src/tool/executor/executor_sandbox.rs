use super::{ToolError, ToolExecutor};
use agena_domain::PermissionScope;
use agena_runtime_contracts::authorization::{SandboxPermissionSnapshot, SandboxPersistedRule};
use std::sync::Arc;

impl ToolExecutor {
    /// Derive the resource policy from this executor's current scoped
    /// principal. No cached base policy can omit a later permission ceiling.
    pub fn sandbox_permission_snapshot(&self) -> SandboxPermissionSnapshot {
        self.principal
            .sandbox_permission_snapshot(self.permission_workspace_root())
            .with_persisted_rules(self.sandbox_persisted_rules.as_ref().clone())
    }

    /// Long-lived terminals accept new input only under their launch authority.
    /// Read/stop operations remain available after settings change.
    pub(crate) fn execution_policy_identity(&self) -> Result<String, ToolError> {
        let mut resources = self.sandbox_permission_snapshot();
        // Tool and command approvals are checked on every call. Remembering
        // one must not invalidate a terminal's unchanged filesystem/network scope.
        resources.persisted_rules.retain(|rule| {
            matches!(
                rule.action,
                agena_domain::PermissionAction::PathAccess { .. }
                    | agena_domain::PermissionAction::NetworkAccess { .. }
            )
        });
        serde_json::to_string(&(
            self.sandbox_enabled(),
            self.resource_guard_enabled(),
            resources,
        ))
        .map_err(|error| ToolError::invalid_input_error(&error))
    }

    /// The actual working directory remains a resource even when shell effect
    /// declarations are disabled. Validating it never grants its descendants.
    pub(crate) fn validate_sandbox_workdir(&self, cwd: &std::path::Path) -> Result<(), ToolError> {
        if !self.sandbox_enabled() {
            return Ok(());
        }
        let resolved = crate::canonicalize_mutation_path(cwd);
        if !cwd.is_absolute() || !resolved.is_dir() {
            return Err(ToolError::invalid_input(
                "sandbox working directory must be an existing absolute directory",
            ));
        }
        if !matches!(
            self.sandbox_permission_snapshot()
                .authorize_path_access(agena_domain::AccessKind::Read, &resolved),
            agena_domain::PermissionDecision::Allow
        ) {
            return Err(ToolError::invalid_input(format!(
                "sandbox policy does not allow reading the working directory {}; select an allowed directory with workdir or folder",
                resolved.display()
            )));
        }
        Ok(())
    }

    /// Bind the repository's non-revoked session snapshot to this invocation.
    /// Repository snapshots can contain older records for the same scope/key;
    /// keep exactly the latest one, matching `PermissionRuleRepository::resolve`.
    pub fn with_sandbox_persisted_rules(
        mut self,
        mut records: Vec<agena_storage::PersistedPermissionRule>,
    ) -> Result<Self, ToolError> {
        fn scope_order(scope: PermissionScope) -> u8 {
            match scope {
                PermissionScope::Global => 0,
                PermissionScope::Workspace => 1,
                PermissionScope::Session => 2,
            }
        }
        records.retain(|rule| rule.revoked_at_ms.is_none());
        records.sort_by(|left, right| {
            scope_order(left.scope)
                .cmp(&scope_order(right.scope))
                .then_with(|| left.action_key.cmp(&right.action_key))
                .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
                .then_with(|| right.id.cmp(&left.id))
        });
        records.dedup_by(|left, right| {
            left.scope == right.scope && left.action_key == right.action_key
        });
        let rules =
            records
                .into_iter()
                .map(|rule| {
                    serde_json::from_str(&rule.action_key)
                    .map(|action| SandboxPersistedRule { action, mode: rule.mode })
                    .map_err(|error| {
                        ToolError::invalid_input(format!(
                            "cannot construct sandbox permission snapshot from stored rule: {error}"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        self.sandbox_persisted_rules = Arc::new(rules);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{AccessKind, PermissionAction, PermissionDecision, PermissionMode};
    use agena_runtime_contracts::{
        authorization::ExecutionPrincipal,
        permission::{PermissionPolicy, ToolPermissionPolicy},
    };

    fn record(
        scope: PermissionScope,
        revision: i64,
        mode: PermissionMode,
        action: &PermissionAction,
    ) -> agena_storage::PersistedPermissionRule {
        agena_storage::PersistedPermissionRule {
            id: Some(revision),
            created_at_ms: Some(revision),
            updated_at_ms: Some(revision),
            action_key: serde_json::to_string(action).unwrap(),
            mode,
            scope,
            session_id: None,
            workspace_id: None,
            source: "test".into(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        }
    }

    #[test]
    fn newest_persisted_record_per_scope_matches_repository_resolution() {
        let root = std::env::temp_dir().join("agena-sandbox-record-test");
        let target = root.join("file");
        let action = PermissionAction::PathAccess {
            access_kind: "read".into(),
            workspace_root: root.to_string_lossy().replace('\\', "/"),
            target_path: target.to_string_lossy().replace('\\', "/"),
        };
        let executor = ToolExecutor::new(
            root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
        .with_sandbox_persisted_rules(vec![
            record(PermissionScope::Session, 1, PermissionMode::Deny, &action),
            record(PermissionScope::Session, 2, PermissionMode::Ask, &action),
        ])
        .unwrap();
        let snapshot = executor.sandbox_permission_snapshot();
        assert_eq!(snapshot.persisted_rules.len(), 1);
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Read, &target),
            PermissionDecision::Allow
        ));
    }

    #[test]
    fn current_principal_is_projected_after_configuration_changes() {
        let root = std::env::temp_dir().join("agena-sandbox-current-principal-test");
        let executor = ToolExecutor::new(
            root.clone(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
        .with_permission_config(
            &serde_json::from_value(serde_json::json!({"path":{"workspace":"deny"}})).unwrap(),
        );
        assert!(matches!(
            executor
                .sandbox_permission_snapshot()
                .authorize_path_access(AccessKind::Read, &root),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn terminal_identity_ignores_tool_approvals_but_tracks_resource_decisions() {
        let root = std::env::temp_dir().join("agena-terminal-resource-identity");
        let executor = ToolExecutor::new(
            &root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        );
        let identity = executor.execution_policy_identity().unwrap();
        let tool = record(
            PermissionScope::Session,
            1,
            PermissionMode::Allow,
            &PermissionAction::Tool {
                tool_name: "agena.fs.stat".into(),
                qualifier: None,
            },
        );
        let remembered_tool = executor
            .clone()
            .with_sandbox_persisted_rules(vec![tool.clone()])
            .unwrap();
        assert_eq!(
            identity,
            remembered_tool.execution_policy_identity().unwrap()
        );
        for action in [
            PermissionAction::PathAccess {
                access_kind: "read".into(),
                workspace_root: root.to_string_lossy().into_owned(),
                target_path: root.join("secret").to_string_lossy().into_owned(),
            },
            PermissionAction::NetworkAccess {
                target: "192.0.2.1:443".into(),
                host: "192.0.2.1".into(),
                port: Some(443),
            },
        ] {
            let scoped = executor
                .clone()
                .with_sandbox_persisted_rules(vec![
                    tool.clone(),
                    record(PermissionScope::Session, 2, PermissionMode::Deny, &action),
                ])
                .unwrap();
            assert_ne!(identity, scoped.execution_policy_identity().unwrap());
        }
    }

    #[test]
    fn sandbox_workdir_requires_real_read_authority_and_mode_changes_identity() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("input")).unwrap();
        let executor = ToolExecutor::new(
            &root,
            ExecutionPrincipal::new(PermissionPolicy::allow_all(), ToolPermissionPolicy::allow_all()),
            agena_plugin_host::PluginHost::new_empty(), None, None,
        ).with_permission_config(&serde_json::from_value(serde_json::json!({
            "path":{"workspace":"deny", "external":"deny", "rules":{"<workspace>/input":"readonly"}}
        })).unwrap());
        assert!(
            executor
                .validate_sandbox_workdir(&root)
                .unwrap_err()
                .to_string()
                .contains("working directory")
        );
        executor
            .validate_sandbox_workdir(&root.join("input"))
            .unwrap();
        let direct = executor.clone().with_execution_settings(false, false);
        direct.validate_sandbox_workdir(&root).unwrap();
        assert_ne!(
            executor.execution_policy_identity().unwrap(),
            direct.execution_policy_identity().unwrap()
        );
    }
}
