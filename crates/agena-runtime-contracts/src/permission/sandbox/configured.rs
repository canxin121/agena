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
        let mut candidates = if self
            .path_layers
            .iter()
            .all(|layer| layer.external_read_default == PermissionMode::Allow)
        {
            filesystem_roots()?
        } else {
            Vec::new()
        };
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
        candidates.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then(left.cmp(right))
        });
        candidates.dedup_by(|left, right| path_identity::equal(left, right));

        // With no launch grants, the existing restriction projector returns
        // only effective explicit read denials, including external runtime
        // denials and current persisted decisions. Class defaults stay separate.
        let denied_paths = self.filesystem_restrictions(&[], &[])?.denied_paths;
        let write_denials = self.persisted_write_denials();
        let mut result = SandboxFilesystemPolicy {
            denied_paths,
            ..Default::default()
        };
        let mut boundaries: Vec<(PathBuf, ConfiguredMode)> = Vec::new();
        for path in candidates {
            let hard_deny = result
                .denied_paths
                .iter()
                .any(|denied| within(&path, denied));
            let read = !hard_deny
                && matches!(
                    self.authorize_path_access(AccessKind::Read, &path),
                    PermissionDecision::Allow
                );
            let write = read
                && !write_denials.iter().any(|denied| within(&path, denied))
                && matches!(
                    self.authorize_path_access(AccessKind::Write, &path),
                    PermissionDecision::Allow
                );
            let mode = match (read, write) {
                (true, true) => ConfiguredMode::ReadWrite,
                (true, false) => ConfiguredMode::ReadOnly,
                _ => ConfiguredMode::Deny,
            };
            if read {
                self.validate_configured_subtree(AccessKind::Read, &path)?;
            }
            if write {
                self.validate_configured_subtree(AccessKind::Write, &path)?;
            }
            let parent = boundaries
                .iter()
                .rev()
                .find(|(parent, _)| within(&path, parent))
                .map_or(ConfiguredMode::Deny, |(_, mode)| *mode);
            if mode == ConfiguredMode::Deny && mode == parent {
                continue;
            }
            match mode {
                ConfiguredMode::ReadOnly => result.readonly_paths.push(path.clone()),
                ConfiguredMode::ReadWrite => result.readwrite_paths.push(path.clone()),
                ConfiguredMode::Deny if !hard_deny => {
                    result.default_denied_paths.push(path.clone())
                }
                ConfiguredMode::Deny => {}
            }
            // Keep every concrete grant, including a redundant same-mode child:
            // it can reopen an explicitly selected subtree of the launcher's
            // reserved temporary roots. Ancestry deduplication would also erase
            // the final RO in an RO -> RW -> RO policy.
            boundaries.push((path, mode));
        }
        reject_conflicting_root_identities(&result)?;
        Ok(result)
    }

    fn validate_configured_subtree(
        &self,
        access: AccessKind,
        path: &Path,
    ) -> Result<(), SandboxPolicyError> {
        if path.is_file() {
            return Ok(());
        }
        for layer in &self.path_layers {
            let mode = layer.subtree_default(access, &self.workspace_root, path);
            if mode == PermissionMode::Deny
                || (mode != PermissionMode::Allow
                    && matches!(
                        layer.check(access, &self.workspace_root, path),
                        PermissionDecision::Allow
                    ))
            {
                return Err(SandboxPolicyError::Unsupported {
                    pattern: path.to_string_lossy().into_owned(),
                    reason: "a literal directory grant does not approve its more restrictive descendant default; grant a subtree explicitly".into(),
                });
            }
        }
        Ok(())
    }

    fn persisted_write_denials(&self) -> Vec<PathBuf> {
        self.persisted_rules
            .iter()
            .filter_map(|rule| {
                let PermissionAction::PathAccess {
                    access_kind,
                    workspace_root,
                    target_path,
                } = &rule.action
                else {
                    return None;
                };
                let path = PathBuf::from(normalize_path_string(Path::new(target_path)));
                (rule.mode == PermissionMode::Deny
                    && access_kind == "write"
                    && path_identity::equal(Path::new(workspace_root), &self.workspace_root)
                    && matches!(
                        self.authorize_path_access(AccessKind::Write, &path),
                        PermissionDecision::Deny { .. }
                    ))
                .then_some(path)
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfiguredMode {
    Deny,
    ReadOnly,
    ReadWrite,
}

fn reject_conflicting_root_identities(
    policy: &SandboxFilesystemPolicy,
) -> Result<(), SandboxPolicyError> {
    // Class defaults create roots too. Comparing only explicit settings rules
    // misses a default RW workspace paired with a readonly APFS/symlink view of
    // the same directory. Native identity normalization must not choose RW.
    for readonly in &policy.readonly_paths {
        for readwrite in &policy.readwrite_paths {
            let same = path_identity::equal(readonly, readwrite)
                || crate::authorization::same_existing_file(readonly, readwrite).map_err(
                    |reason| SandboxPolicyError::Unsupported {
                        pattern: readonly.display().to_string(),
                        reason,
                    },
                )?;
            if same {
                return Err(SandboxPolicyError::Unsupported {
                    pattern: readonly.display().to_string(),
                    reason: format!(
                        "readonly root {} and readwrite root {} refer to the same filesystem resource; use one path spelling and one mode for this resource, including its workspace default",
                        readonly.display(),
                        readwrite.display(),
                    ),
                });
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn filesystem_roots() -> Result<Vec<PathBuf>, SandboxPolicyError> {
    Ok(vec![PathBuf::from(std::path::MAIN_SEPARATOR_STR)])
}

#[cfg(windows)]
fn filesystem_roots() -> Result<Vec<PathBuf>, SandboxPolicyError> {
    // The drive mask also includes empty removable drives and offline mapped
    // drives. Defaults cover currently accessible volumes; explicit path rules
    // still report their own unavailable-resource errors at launch.
    let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    if drives == 0 {
        return Err(SandboxPolicyError::Unsupported {
            pattern: "permission.path.external".into(),
            reason: format!(
                "cannot enumerate host filesystem roots: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    Ok((0u8..26)
        .filter(|index| drives & (1u32 << *index) != 0)
        .map(|index| PathBuf::from(format!("{}:/", char::from(b'A' + index))))
        .filter(|root| std::fs::read_dir(root).is_ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;

    #[cfg(any(unix, windows))]
    struct IdentityFixture(PathBuf);

    #[cfg(any(unix, windows))]
    impl IdentityFixture {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "agena-configured-identity-{}-{unique}",
                std::process::id(),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(std::fs::canonicalize(path).unwrap())
        }
    }

    #[cfg(any(unix, windows))]
    impl Drop for IdentityFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn root() -> PathBuf {
        PathBuf::from(normalize_path_string(
            &std::env::temp_dir().join("agena-configured-sandbox-policy"),
        ))
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
    #[cfg(unix)]
    fn workspace_default_cannot_conflict_with_a_readonly_view_of_the_same_directory() {
        let fixture = IdentityFixture::new();
        let workspace = fixture.0.join("workspace");
        let alias = fixture.0.join("alternate-view");
        std::fs::create_dir(&workspace).unwrap();
        std::os::unix::fs::symlink(&workspace, &alias).unwrap();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                alias.to_string_lossy().as_ref():"readonly"
            }
        }}))
        .sandbox_permission_snapshot(&workspace);
        let error = snapshot.filesystem_policy().unwrap_err().to_string();
        assert!(error.contains("same filesystem resource"), "{error}");
        assert!(error.contains("workspace default"), "{error}");
        assert!(error.contains("one path spelling"), "{error}");

        let same_mode = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                alias.to_string_lossy().as_ref():"readwrite"
            }
        }}))
        .sandbox_permission_snapshot(&workspace);
        assert!(same_mode.filesystem_policy().is_ok());
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn hardlink_roots_cannot_have_different_filesystem_modes() {
        let fixture = IdentityFixture::new();
        let source = fixture.0.join("source.txt");
        let alias = fixture.0.join("alias.txt");
        std::fs::write(&source, b"fixture").unwrap();
        std::fs::hard_link(&source, &alias).unwrap();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"deny", "rules": {
                "<workspace>/source.txt":"readwrite",
                "<workspace>/alias.txt":"readonly"
            }
        }}))
        .sandbox_permission_snapshot(&fixture.0);
        let error = snapshot.filesystem_policy().unwrap_err().to_string();
        assert!(error.contains("same filesystem resource"), "{error}");
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
    fn all_workspace_and_external_default_combinations_keep_class_boundaries() {
        let root = root();
        let host_roots = normalized_paths(&filesystem_roots().unwrap()).unwrap();
        for workspace in ["deny", "readonly", "readwrite"] {
            for external in ["deny", "readonly", "readwrite"] {
                let snapshot = principal(serde_json::json!({"path": {
                    "workspace":workspace, "external":external
                }}))
                .sandbox_permission_snapshot(&root);
                let scope = snapshot.filesystem_policy().unwrap();
                let mut readonly = if external == "readonly" {
                    host_roots.clone()
                } else {
                    vec![]
                };
                let mut readwrite = if external == "readwrite" {
                    host_roots.clone()
                } else {
                    vec![]
                };
                match workspace {
                    "readonly" => readonly.push(root.clone()),
                    "readwrite" => readwrite.push(root.clone()),
                    _ => {}
                }
                assert_eq!(
                    scope.readonly_paths, readonly,
                    "workspace={workspace}, external={external}"
                );
                assert_eq!(
                    scope.readwrite_paths, readwrite,
                    "workspace={workspace}, external={external}"
                );
                assert_eq!(
                    scope.default_denied_paths,
                    if workspace == "deny" && external != "deny" {
                        vec![root.clone()]
                    } else {
                        vec![]
                    },
                    "workspace={workspace}, external={external}"
                );
                assert!(scope.denied_paths.is_empty());
            }
        }
    }

    #[test]
    fn workspace_readwrite_keeps_nested_readonly_boundary() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                "<workspace>/reference":"readonly"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(scope.readwrite_paths, [root.clone()]);
        assert_eq!(scope.readonly_paths, [root.join("reference")]);
        assert!(scope.denied_paths.is_empty());
    }

    #[test]
    fn independent_ceiling_preserves_nested_readonly_boundary() {
        let root = root();
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
        let scope = principal
            .sandbox_permission_snapshot(&root)
            .filesystem_policy()
            .unwrap();
        assert_eq!(scope.readwrite_paths, [root.clone()]);
        assert_eq!(scope.readonly_paths, [root.join("reference")]);
    }

    #[test]
    fn alternating_nested_modes_keep_each_override() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readonly", "external":"deny", "rules": {
                "<workspace>/output":"readwrite",
                "<workspace>/output/reference":"readonly",
                "<workspace>/output/reference/generated":"readwrite"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(
            scope.readonly_paths,
            [root.clone(), root.join("output/reference")]
        );
        assert_eq!(
            scope.readwrite_paths,
            [root.join("output"), root.join("output/reference/generated")]
        );
        assert!(scope.default_denied_paths.is_empty());
    }

    #[test]
    fn concrete_same_mode_grants_survive_broad_external_defaults() {
        let root = root();
        for mode in ["readonly", "readwrite"] {
            let snapshot = principal(serde_json::json!({"path": {
                "workspace":mode, "external":mode, "rules": {
                    "<workspace>/cache":mode
                }
            }}))
            .sandbox_permission_snapshot(&root);
            let scope = snapshot.filesystem_policy().unwrap();
            let mut expected = normalized_paths(&filesystem_roots().unwrap()).unwrap();
            expected.extend([root.clone(), root.join("cache")]);
            if mode == "readonly" {
                assert_eq!(scope.readonly_paths, expected);
                assert!(scope.readwrite_paths.is_empty());
            } else {
                assert_eq!(scope.readwrite_paths, expected);
                assert!(scope.readonly_paths.is_empty());
            }
        }
    }

    #[test]
    fn external_default_excludes_a_denied_workspace_and_preserves_its_granted_children() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"readonly", "rules": {
                "<workspace>/input":"readonly",
                "<workspace>/output":"readwrite",
                "<workspace>/secrets":"deny"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        let mut readonly = normalized_paths(&filesystem_roots().unwrap()).unwrap();
        readonly.push(root.join("input"));
        assert_eq!(scope.readonly_paths, readonly);
        assert_eq!(scope.readwrite_paths, [root.join("output")]);
        assert_eq!(scope.default_denied_paths, [root.clone()]);
        assert_eq!(scope.denied_paths, [root.join("secrets")]);
    }

    #[test]
    fn independent_ceiling_intersects_external_defaults_and_workspace_exceptions() {
        let root = root();
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"readonly"
        }}))
        .try_apply_permission_ceiling(
            &serde_json::from_value(serde_json::json!({"path": {
                "workspace":"deny", "external":"readwrite", "rules": {
                    "<workspace>/input":"readonly", "<workspace>/output":"readwrite"
                }
            }}))
            .unwrap(),
        )
        .unwrap()
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        let mut readonly = normalized_paths(&filesystem_roots().unwrap()).unwrap();
        readonly.push(root.join("input"));
        assert_eq!(scope.readonly_paths, readonly);
        assert_eq!(scope.readwrite_paths, [root.join("output")]);
        assert_eq!(scope.default_denied_paths, [root]);
        assert!(scope.denied_paths.is_empty());
    }

    #[test]
    fn persisted_write_denial_caps_descendants_until_the_latest_decision_allows_it() {
        let root = root();
        let action = PermissionAction::PathAccess {
            access_kind: "write".into(),
            workspace_root: normalize_path_string(&root),
            target_path: normalize_path_string(&root.join("output")),
        };
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                "<workspace>/output/generated":"readwrite"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let deny = SandboxPersistedRule {
            action: action.clone(),
            mode: PermissionMode::Deny,
        };
        let restricted = snapshot
            .clone()
            .with_persisted_rules(vec![deny.clone()])
            .filesystem_policy()
            .unwrap();
        assert_eq!(restricted.readwrite_paths, [root.clone()]);
        assert_eq!(
            restricted.readonly_paths,
            [root.join("output"), root.join("output/generated")]
        );
        assert!(restricted.denied_paths.is_empty());
        let allowed = snapshot
            .with_persisted_rules(vec![
                deny,
                SandboxPersistedRule {
                    action,
                    mode: PermissionMode::Allow,
                },
            ])
            .filesystem_policy()
            .unwrap();
        assert_eq!(
            allowed.readwrite_paths,
            [
                root.clone(),
                root.join("output"),
                root.join("output/generated")
            ]
        );
        assert!(allowed.readonly_paths.is_empty());
    }

    #[test]
    fn explicit_filesystem_root_grants_preserve_recursive_roots_and_child_overrides() {
        let root = root();
        let filesystem_root = root.ancestors().last().unwrap().to_path_buf();
        let subtree_rule = format!(
            "{}/**",
            normalize_path_string(&filesystem_root).trim_end_matches('/')
        );
        let snapshot = principal(serde_json::json!({"path": {
            "workspace":"deny", "external":"deny", "rules": {
                subtree_rule:"readonly",
                "<workspace>/output":"readwrite",
                "<workspace>/secrets":"deny"
            }
        }}))
        .sandbox_permission_snapshot(&root);
        let scope = snapshot.filesystem_policy().unwrap();
        assert_eq!(scope.readonly_paths, [filesystem_root, root.clone()]);
        assert_eq!(scope.readwrite_paths, [root.join("output")]);
        assert_eq!(scope.denied_paths, [root.join("secrets")]);
        assert!(scope.default_denied_paths.is_empty());
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Read, &root.join("reference/file")),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Write, &root.join("output/file")),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            snapshot.authorize_path_access(AccessKind::Write, &root.join("reference/file")),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn global_external_default_has_no_implicit_workspace() {
        let scope = principal(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"readonly"
        }}))
        .sandbox_permission_snapshot(Path::new(""))
        .filesystem_policy()
        .unwrap();
        assert_eq!(
            scope.readonly_paths,
            normalized_paths(&filesystem_roots().unwrap()).unwrap()
        );
        assert!(scope.readwrite_paths.is_empty());
        assert!(scope.default_denied_paths.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn windows_filesystem_roots_remain_absolute_after_normalization() {
        for path in [r"C:\", r"\\?\C:\"] {
            let normalized = normalize_path_string(Path::new(path));
            assert_eq!(normalized, "c:/");
            assert!(Path::new(&normalized).is_absolute());
        }
        let roots = normalized_paths(&filesystem_roots().unwrap()).unwrap();
        assert!(!roots.is_empty());
        assert!(roots.iter().all(|path| path.is_absolute()));
        assert!(roots.iter().all(|path| std::fs::read_dir(path).is_ok()));
    }
}
