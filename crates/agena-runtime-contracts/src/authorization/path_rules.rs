//! Compile the public concrete-path language into deterministic runtime rules.

use agena_domain::{AccessSelector, PathAccessModes, PathPermissionConfig, PathPermissionMode};

use crate::permission::{PermissionConfigError, PermissionPolicy};

pub(super) fn apply(
    value: &PathPermissionConfig,
    mut base: PermissionPolicy,
) -> Result<PermissionPolicy, PermissionConfigError> {
    if let Some(workspace) = &value.workspace {
        validate_mode("permission.path.workspace", workspace)?;
        base.workspace_read_default = workspace.read.expect("validated read mode");
        base.workspace_write_default = workspace.write.expect("validated write mode");
    }
    if let Some(external) = &value.external {
        validate_mode("permission.path.external", external)?;
        base.external_read_default = external.read.expect("validated read mode");
        base.external_write_default = external.write.expect("validated write mode");
    }

    let mut normalized = value.clone();
    normalized
        .normalize_rules()
        .map_err(|reason| PermissionConfigError::InvalidPathRule {
            pattern: "permission.path.rules".into(),
            reason,
        })?;
    let mut rules = normalized
        .rules
        .into_iter()
        .map(|(path, modes)| {
            let mode = validate_mode(&path, &modes)?;
            Ok((path, mode))
        })
        .collect::<Result<Vec<_>, PermissionConfigError>>()?;
    rules.sort_by(|(left, _), (right, _)| left.len().cmp(&right.len()).then(left.cmp(right)));

    for (index, (parent, parent_mode)) in rules.iter().enumerate() {
        for (child, child_mode) in rules.iter().skip(index + 1) {
            if !child.starts_with(&format!("{}/", parent.trim_end_matches('/')))
                || parent_mode == child_mode
            {
                continue;
            }
            if *parent_mode == PathPermissionMode::Deny {
                return Err(PermissionConfigError::InvalidPathRule {
                    pattern: child.clone(),
                    reason: format!(
                        "an explicitly denied directory cannot contain a granted child: conflicts with {parent:?}"
                    ),
                });
            }
        }
    }

    // The public language contains literal paths only. Internally emit the
    // root and its complete subtree so snapshots and OS projection use the
    // same boundaries. A regular file has no descendants; no is_dir probe can
    // accidentally broaden or shrink its permission when compiling settings.
    for (path, mode) in rules {
        let access = PathAccessModes::from_mode(mode);
        for pattern in [path.clone(), format!("{}/**", path.trim_end_matches('/'))] {
            base.add_path_pattern_rule(
                AccessSelector::Read,
                access.read.expect("concrete read mode"),
                pattern.clone(),
            )?;
            base.add_path_pattern_rule(
                AccessSelector::Write,
                access.write.expect("concrete write mode"),
                pattern,
            )?;
        }
    }
    Ok(base)
}

fn validate_mode(
    path: &str,
    modes: &PathAccessModes,
) -> Result<PathPermissionMode, PermissionConfigError> {
    modes
        .sandbox_mode()
        .map_err(|reason| PermissionConfigError::InvalidPathRule {
            pattern: path.into(),
            reason: reason.into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{ExecutionPrincipal, PermissionConfig, validate_permission_config};
    use crate::permission::ToolPermissionPolicy;
    use agena_domain::{AccessKind, PermissionDecision, PermissionMode};
    use std::path::PathBuf;

    fn root() -> PathBuf {
        std::env::temp_dir().join("agena-concrete-path-rules")
    }

    fn principal(config: &PermissionConfig) -> ExecutionPrincipal {
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(config)
        .unwrap()
    }

    #[test]
    fn concrete_directory_permissions_match_the_projected_boundary() {
        let config: PermissionConfig = serde_json::from_value(serde_json::json!({
            "path": {
                "workspace":"deny",
                "external":"deny",
                "rules":{
                    "<workspace>/input":"readonly",
                    "<workspace>/output":"readwrite",
                    "<workspace>/secrets":"deny"
                }
            }
        }))
        .unwrap();
        validate_permission_config(&config).unwrap();
        let root = root();
        let principal = principal(&config);
        let snapshot = principal.sandbox_permission_snapshot(&root);
        for (access, relative, allowed) in [
            (AccessKind::Read, "input/data.txt", true),
            (AccessKind::Read, "input/new/deep/file.txt", true),
            (AccessKind::Write, "input/data.txt", false),
            (AccessKind::Write, "output/new/deep/file.txt", true),
            (AccessKind::Read, "input-other/data.txt", false),
            (AccessKind::Read, "secrets/token", false),
            (AccessKind::Read, "other.txt", false),
        ] {
            let path = root.join(relative);
            assert_eq!(
                matches!(
                    principal.authorize_path_access(access, &root, &path),
                    PermissionDecision::Allow
                ),
                allowed,
                "{relative}",
            );
            assert_eq!(
                matches!(
                    snapshot.authorize_path_access(access, &path),
                    PermissionDecision::Allow
                ),
                allowed,
                "{relative}",
            );
        }
        let restrictions = snapshot
            .filesystem_restrictions(&[root.join("input")], &[root.join("output")])
            .unwrap();
        assert!(restrictions.denied_paths.contains(&root.join("secrets")));
        assert!(!restrictions.denied_paths.contains(&root));
    }

    #[test]
    fn workspace_default_deny_does_not_deny_an_explicit_single_file() {
        let config: PermissionConfig = serde_json::from_value(serde_json::json!({
            "path":{"workspace":"deny","external":"deny","rules":{
                "<workspace>/result.txt":"readwrite"
            }}
        }))
        .unwrap();
        let root = root();
        let snapshot = principal(&config).sandbox_permission_snapshot(&root);
        let file = root.join("result.txt");
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Write, &file),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Read, &root.join("other.txt")),
            PermissionDecision::Deny { .. }
        ));
        let restrictions = snapshot.filesystem_restrictions(&[], &[file]).unwrap();
        assert!(restrictions.denied_paths.is_empty());
    }

    #[test]
    fn json_order_cannot_reopen_a_denied_subtree() {
        for rules in [
            serde_json::json!({"<workspace>/secrets":"deny","<workspace>/secrets/public":"readonly"}),
            serde_json::json!({"<workspace>/secrets/public":"readonly","<workspace>/secrets":"deny"}),
        ] {
            let config = serde_json::from_value::<PermissionConfig>(
                serde_json::json!({"path":{"rules":rules}}),
            );
            assert!(config.is_err() || validate_permission_config(&config.unwrap()).is_err());
        }
        let one: PermissionConfig = serde_json::from_str(r#"{"path":{"workspace":"deny","rules":{"<workspace>":"readwrite","<workspace>/secrets":"deny"}}}"#).unwrap();
        let two: PermissionConfig = serde_json::from_str(r#"{"path":{"workspace":"deny","rules":{"<workspace>/secrets":"deny","<workspace>":"readwrite"}}}"#).unwrap();
        let root = root();
        assert_eq!(
            principal(&one).sandbox_permission_snapshot(&root),
            principal(&two).sandbox_permission_snapshot(&root),
        );
    }

    #[test]
    fn nested_readonly_and_readwrite_follow_the_most_specific_rule() {
        let root = root();
        for (parent, child) in [("readonly", "readwrite"), ("readwrite", "readonly")] {
            for rules in [
                serde_json::json!({"<workspace>":parent,"<workspace>/output":child}),
                serde_json::json!({"<workspace>/output":child,"<workspace>":parent}),
            ] {
                let config: PermissionConfig = serde_json::from_value(serde_json::json!({
                    "path":{"workspace":"deny","external":"deny","rules":rules}
                }))
                .unwrap();
                validate_permission_config(&config).unwrap();
                let snapshot = principal(&config).sandbox_permission_snapshot(&root);
                for (path, mode) in [
                    (root.join("file.txt"), parent),
                    (root.join("output/file.txt"), child),
                ] {
                    assert!(matches!(
                        snapshot.authorize_path_access(AccessKind::Read, &path),
                        PermissionDecision::Allow
                    ));
                    assert_eq!(
                        matches!(
                            snapshot.authorize_path_access(AccessKind::Write, &path),
                            PermissionDecision::Allow
                        ),
                        mode == "readwrite"
                    );
                }
            }
        }
    }

    #[test]
    fn typed_callers_cannot_bypass_mode_or_pattern_validation() {
        for modes in [
            PathAccessModes {
                read: Some(PermissionMode::Deny),
                write: Some(PermissionMode::Allow),
            },
            PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Ask),
            },
            PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: None,
            },
        ] {
            let config = PermissionConfig {
                path: Some(PathPermissionConfig {
                    workspace: Some(modes),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(validate_permission_config(&config).is_err());
        }
        for path in [
            "<workspace>/**/*.key",
            "<unknown>/file",
            "<workspace>/../escape",
        ] {
            let config = PermissionConfig {
                path: Some(PathPermissionConfig {
                    rules: [(
                        path.into(),
                        PathAccessModes::from_mode(PathPermissionMode::Deny),
                    )]
                    .into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(validate_permission_config(&config).is_err(), "{path}");
        }
    }

    #[test]
    fn shipped_resource_defaults_are_concrete_and_do_not_approve_external_access() {
        let config = PermissionConfig::global_default();
        validate_permission_config(&config).unwrap();
        serde_json::to_value(&config).unwrap();
        let principal = principal(&config);
        // Keep both paths on the same volume without depending on real files.
        let executable = std::env::current_exe().unwrap();
        let volume = executable.ancestors().last().unwrap();
        let root = volume.join("agena-default-policy-workspace");
        assert!(matches!(
            principal.authorize_path_access(AccessKind::Read, &root, &root.join("src/main.rs")),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            principal.authorize_path_access(AccessKind::Write, &root, &root.join("src/main.rs")),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            principal.authorize_path_access(
                AccessKind::Read,
                &root,
                &volume.join("agena-default-policy-external/data")
            ),
            PermissionDecision::Deny { .. }
        ));
    }
}
