use super::{ToolError, ToolExecutor};
use agena_domain::PermissionScope;
use agena_runtime_contracts::authorization::{SandboxPermissionSnapshot, SandboxPersistedRule};
use std::sync::Arc;

impl ToolExecutor {
    /// Derive the resource policy from this executor's current scoped
    /// principal. No cached base policy can omit a later permission ceiling.
    pub fn sandbox_permission_snapshot(&self) -> SandboxPermissionSnapshot {
        self.principal
            .sandbox_permission_snapshot(self.workspace_root())
            .with_persisted_rules(self.sandbox_persisted_rules.as_ref().clone())
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
            &serde_json::from_value(serde_json::json!({"path":{"workspace":{"read":"deny"}}}))
                .unwrap(),
        );
        assert!(matches!(
            executor
                .sandbox_permission_snapshot()
                .authorize_path_access(AccessKind::Read, &root),
            PermissionDecision::Deny { .. }
        ));
    }
}
