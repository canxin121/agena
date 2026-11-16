//! Compile configured roots without requiring model resource declarations.

use super::*;

impl SandboxPermissionSnapshot {
    /// Grant exactly the roots allowed by the effective static policy, all
    /// independent authority ceilings, and persisted decisions. Ask/Auto never
    /// becomes an implicit grant when there is no declared resource to approve.
    pub fn filesystem_policy(&self) -> Result<SandboxFilesystemPolicy, SandboxPolicyError> {
        if self.blocked {
            return Err(SandboxPolicyError::Blocked);
        }
        if self
            .path_layers
            .iter()
            .all(|layer| layer.external_read_default == PermissionMode::Allow)
        {
            return Err(SandboxPolicyError::Unsupported {
                pattern: "permission.path.external".into(),
                reason: "an allowed external default grants the entire host filesystem, including protected sandbox and shared temporary paths; use external: deny and grant concrete external files or directories, or disable the sandbox".into(),
            });
        }

        let mut candidates = Vec::new();
        if !self.workspace_root.as_os_str().is_empty() {
            candidates.push(self.workspace_root.clone());
        }
        for layer in &self.path_layers {
            for rule in &layer.rules {
                for pattern in &rule.patterns {
                    if let Some((path, _)) = pattern.boundary(&[pattern.filesystem_root()])? {
                        candidates.push(path);
                    }
                }
            }
        }
        for rule in &self.persisted_rules {
            if let PermissionAction::PathAccess {
                workspace_root,
                target_path,
                ..
            } = &rule.action
                && path_identity::equal(Path::new(workspace_root), &self.workspace_root)
            {
                candidates.push(PathBuf::from(target_path));
            }
        }
        let mut candidates = normalized_paths(&candidates)?;
        candidates.sort();
        candidates.dedup_by(|left, right| path_identity::equal(left, right));
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        for path in candidates {
            let read = matches!(
                self.authorize_path_access(AccessKind::Read, &path),
                PermissionDecision::Allow
            );
            let write = matches!(
                self.authorize_path_access(AccessKind::Write, &path),
                PermissionDecision::Allow
            );
            if read {
                reads.push(path.clone());
            }
            // A persisted read denial inside a writable tree closes that
            // subtree completely. It must not become a write-only mount.
            if read && write {
                writes.push(path);
            }
        }
        remove_covered_roots(&mut reads);
        remove_covered_roots(&mut writes);
        let restrictions = self.filesystem_restrictions(&reads, &writes)?;
        reads.retain(|read| !writes.iter().any(|write| within(read, write)));
        reads.extend(restrictions.readonly_paths);
        remove_covered_roots(&mut reads);
        for read in &reads {
            if writes
                .iter()
                .any(|write| within(write, read) || within(read, write))
            {
                return Err(SandboxPolicyError::Unsupported {
                    pattern: read.to_string_lossy().into_owned(),
                    reason: "Agena's portable MXC policy requires separate readonly/readwrite roots; nested readonly/readwrite grants, including workspace defaults and permission ceilings, are not accepted. Use workspace: deny and grant each file or directory separately".into(),
                });
            }
        }
        Ok(SandboxFilesystemPolicy {
            readonly_paths: reads,
            readwrite_paths: writes,
            denied_paths: restrictions.denied_paths,
        })
    }
}

fn remove_covered_roots(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then(left.cmp(right))
    });
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in paths.drain(..) {
        if !roots.iter().any(|root| within(&path, root)) {
            roots.push(path);
        }
    }
    *paths = roots;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;

    fn root() -> PathBuf {
        std::env::temp_dir().join("agena-configured-sandbox-policy")
    }

    fn principal(value: serde_json::Value) -> ExecutionPrincipal {
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&serde_json::from_value(value).unwrap())
        .unwrap()
    }

    #[test]
    fn configured_roots_follow_defaults_and_preserve_denied_descendants() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                "<workspace>/secrets":"deny"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(scope.readwrite_paths, [root.clone()]);
        assert!(scope.readonly_paths.is_empty());
        assert_eq!(scope.denied_paths, [root.join("secrets")]);
    }

    #[test]
    fn deny_default_allows_exact_file_without_granting_its_parent() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"deny", "rules": {
                "<workspace>/output/result.txt":"readwrite",
                "<workspace>/input":"readonly"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(scope.readwrite_paths, [root.join("output/result.txt")]);
        assert_eq!(scope.readonly_paths, [root.join("input")]);
        assert!(!scope.readwrite_paths.contains(&root.join("output")));
    }

    #[test]
    fn independent_ceiling_intersects_roots_and_cannot_grant_its_own_scope() {
        let root = root();
        let principal = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny"
        }}))
        .try_apply_permission_ceiling(
            &serde_json::from_value(serde_json::json!({"path": {
                "workspace":"deny", "external":"deny", "rules": {
                    "<workspace>/input":"readonly", "<workspace>/output":"readwrite"
                }
            }}))
            .unwrap(),
        )
        .unwrap();
        let scope = principal
            .sandbox_permission_snapshot(&root)
            .filesystem_policy()
            .unwrap();
        assert_eq!(scope.readonly_paths, [root.join("input")]);
        assert_eq!(scope.readwrite_paths, [root.join("output")]);
    }

    #[test]
    fn persisted_denial_remains_inside_a_configured_grant() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny"
        }}))
        .sandbox_permission_snapshot(&root)
        .with_persisted_rules(vec![SandboxPersistedRule {
            action: PermissionAction::PathAccess {
                access_kind: "read".into(),
                workspace_root: normalize_path_string(&root),
                target_path: normalize_path_string(&root.join("secret.txt")),
            },
            mode: PermissionMode::Deny,
        }]);
        assert_eq!(
            snapshot.filesystem_policy().unwrap().denied_paths,
            [root.join("secret.txt")]
        );
    }

    #[test]
    fn global_sessions_use_concrete_external_roots_without_an_implicit_workspace() {
        let external = root().join("external");
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                external.to_string_lossy().to_string():"readonly"
            }
        }}))
        .sandbox_permission_snapshot(Path::new(""));
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(scope.readonly_paths, [external]);
        assert!(scope.readwrite_paths.is_empty());
    }

    #[test]
    fn explicit_runtime_denials_outside_grants_survive_configured_policy_projection() {
        let workspace = root().join("project");
        let runtime = root().join("external-runtime");
        let base = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"deny", "rules": {
                "<workspace>/input":"readonly"
            }
        }}));
        let denied = serde_json::from_value(serde_json::json!({"path": {
            "rules": {(runtime.to_string_lossy().to_string()):"deny"}
        }}))
        .unwrap();
        let configured = base
            .clone()
            .try_apply_permission_config(&denied)
            .unwrap()
            .sandbox_permission_snapshot(&workspace);
        let ceiling = base
            .clone()
            .try_apply_permission_ceiling(&denied)
            .unwrap()
            .sandbox_permission_snapshot(&workspace);
        let persisted = base
            .sandbox_permission_snapshot(&workspace)
            .with_persisted_rules(vec![SandboxPersistedRule {
                action: PermissionAction::PathAccess {
                    access_kind: "read".into(),
                    workspace_root: normalize_path_string(&workspace),
                    target_path: normalize_path_string(&runtime),
                },
                mode: PermissionMode::Deny,
            }]);
        for snapshot in [configured, ceiling, persisted] {
            let scope = snapshot.filesystem_policy().unwrap();
            assert_eq!(scope.readonly_paths, [workspace.join("input")]);
            assert!(scope.readwrite_paths.is_empty());
            assert_eq!(scope.denied_paths, [runtime.clone()]);
            // Class defaults never become a denied ancestor that would block input.
            assert!(!scope.denied_paths.contains(&workspace));
        }
    }

    #[test]
    fn superseded_persisted_runtime_denial_does_not_override_the_effective_decision() {
        let workspace = root().join("project");
        let runtime = root().join("external-runtime");
        let base = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"deny", "rules": {
                "<workspace>/input":"readonly",
                (runtime.to_string_lossy().to_string()):"readonly"
            }
        }}));
        let action = PermissionAction::PathAccess {
            access_kind: "read".into(),
            workspace_root: normalize_path_string(&workspace),
            target_path: normalize_path_string(&runtime),
        };
        let snapshot = base
            .sandbox_permission_snapshot(&workspace)
            .with_persisted_rules(vec![
                SandboxPersistedRule {
                    action: action.clone(),
                    mode: PermissionMode::Deny,
                },
                SandboxPersistedRule {
                    action,
                    mode: PermissionMode::Allow,
                },
            ]);
        let scope = snapshot.filesystem_policy().unwrap();
        assert!(scope.readonly_paths.contains(&runtime));
        assert!(scope.denied_paths.is_empty());
    }

    #[test]
    fn broad_external_defaults_and_nested_writes_fail_without_partial_grants() {
        let external = principal(serde_json::json!({"path": {
            "workspace":"readonly", "external":"readonly"
        }}))
        .sandbox_permission_snapshot(&root());
        assert!(
            external
                .filesystem_policy()
                .unwrap_err()
                .to_string()
                .contains("entire host filesystem")
        );
        let nested = principal(serde_json::json!({"path": {
            "workspace":"readonly", "external":"deny", "rules": {"<workspace>/output":"readwrite"}
        }}))
        .sandbox_permission_snapshot(&root());
        assert!(
            nested
                .filesystem_policy()
                .unwrap_err()
                .to_string()
                .contains("nested readonly/readwrite")
        );
    }

    #[test]
    fn workspace_readwrite_default_cannot_hide_nested_readonly_grants() {
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                "<workspace>/reference":"readonly"
            }
        }}))
        .sandbox_permission_snapshot(&root());
        let error = snapshot.filesystem_policy().unwrap_err().to_string();
        assert!(error.contains("nested readonly/readwrite"), "{error}");
        assert!(error.contains("workspace: deny"), "{error}");
    }

    #[test]
    fn independent_ceiling_cannot_create_nested_readonly_and_readwrite_mounts() {
        let principal = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny"
        }}))
        .try_apply_permission_ceiling(
            &serde_json::from_value(serde_json::json!({"path": {
                "workspace":"readwrite", "external":"deny", "rules": {
                    "<workspace>/reference":"readonly"
                }
            }}))
            .unwrap(),
        )
        .unwrap();
        let error = principal
            .sandbox_permission_snapshot(&root())
            .filesystem_policy()
            .unwrap_err()
            .to_string();
        assert!(error.contains("nested readonly/readwrite"), "{error}");
    }
}
