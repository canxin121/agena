use super::*;
use crate::authorization::{ExecutionPrincipal, PermissionConfig};

fn root() -> PathBuf {
    std::env::temp_dir().join("agena-sandbox-policy-tests")
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
fn serialization_preserves_all_ceiling_layers_and_network_rules() {
    let root = root();
    let parent = principal(serde_json::json!({
        "path":{"rules":{"<workspace>/secret/**":{"read":"deny"}}},
        "network":{"internet":"allow","private":"deny","loopback":"deny"}
    }));
    let ceiling: PermissionConfig = serde_json::from_value(serde_json::json!({
        "path":{"rules":{"<workspace>/locked/**":{"write":"deny"}}},
        "network":{"rules":{"*.example.test:443":"deny"}}
    }))
    .unwrap();
    let later: PermissionConfig = serde_json::from_value(serde_json::json!({
        "path":{"workspace":{"read":"allow","write":"allow"}},
        "network":{"internet":"allow","private":"allow","loopback":"allow"}
    }))
    .unwrap();
    let principal = parent
        .try_apply_permission_ceiling(&ceiling)
        .unwrap()
        .try_apply_permission_ceiling(&later)
        .unwrap();
    let snapshot = principal.sandbox_permission_snapshot(&root);
    let snapshot: SandboxPermissionSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(snapshot.path_layers.len(), 3);
    assert_eq!(snapshot.network_layers.len(), 3);
    for (access, path) in [
        (AccessKind::Read, root.join("secret/token")),
        (AccessKind::Write, root.join("locked/file")),
    ] {
        assert!(matches!(
            snapshot.authorize_path_access(access, &path),
            PermissionDecision::Deny { .. }
        ));
    }
    for raw in [
        "foo.example.test:443",
        "127.0.0.1:8080",
        "10.2.3.4:443",
        "[::ffff:127.0.0.1]:8080",
        "[::ffff:10.2.3.4]:443",
    ] {
        assert!(
            matches!(
                snapshot.authorize_network_connect(&raw.parse().unwrap()),
                PermissionDecision::Deny { .. }
            ),
            "{raw}"
        );
    }
    assert!(matches!(
        snapshot.authorize_network_connect(&"other.test:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
}

#[test]
fn subtree_grants_preserve_secret_and_readonly_children() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path":{"rules":{
        "<workspace>/secret/**":{"read":"deny","write":"deny"},
        "<workspace>/locked/**":{"write":"deny"}
    }}}))
    .sandbox_permission_snapshot(&root);
    let projected = snapshot
        .filesystem_restrictions(&[root.clone()], &[root.clone()])
        .unwrap();
    assert!(projected.denied_paths.contains(&root.join("secret")));
    assert!(projected.readonly_paths.contains(&root.join("locked")));
    assert!(!projected.readonly_paths.contains(&root.join("secret")));
    assert!(matches!(
        snapshot.check_granted_path(
            AccessKind::Read,
            &root.join("secret/key"),
            &[root.clone()],
            &[]
        ),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        snapshot.check_granted_path(
            AccessKind::Read,
            &root.join("locked/file"),
            &[root.clone()],
            &[]
        ),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        snapshot.check_granted_path(
            AccessKind::Write,
            &root.join("locked/file"),
            &[root.clone()],
            &[root]
        ),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn root_approval_covers_default_auto_but_not_more_specific_ask() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path":{
        "workspace":{"read":"auto","write":"auto"},
        "rules":{"<workspace>/private/**":{"read":"ask"}}
    }}))
    .sandbox_permission_snapshot(&root);
    assert!(matches!(
        snapshot.check_granted_path(
            AccessKind::Read,
            &root.join("ordinary/file"),
            &[root.clone()],
            &[]
        ),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        snapshot.check_granted_path(
            AccessKind::Read,
            &root.join("private/file"),
            &[root.clone()],
            &[]
        ),
        PermissionDecision::Deny { .. }
    ));
    let child = root.join("private");
    assert!(matches!(
        snapshot.check_granted_path(AccessKind::Read, &child.join("file"), &[child], &[]),
        PermissionDecision::Allow
    ));
}

#[test]
fn unsupported_globs_do_not_use_current_files_as_a_policy() {
    let root = root();
    for pattern in [
        "<workspace>/**/*.key",
        "<workspace>/secret?.txt",
        "<workspace>/prefix*",
    ] {
        let snapshot = principal(serde_json::json!({"path":{"rules":{pattern:{"read":"deny"}}}}))
            .sandbox_permission_snapshot(&root);
        assert!(
            matches!(
                snapshot.filesystem_restrictions(&[root.clone()], &[]),
                Err(SandboxPolicyError::Unsupported { .. })
            ),
            "{pattern}"
        );
    }
}

#[test]
fn single_allowed_probe_name_does_not_erase_subtree_deny() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path":{"rules":{
        "<workspace>/secret/**":{"read":"deny"},
        "<workspace>/secret/.agena-sandbox-policy-descendant":{"read":"allow"}
    }}}))
    .sandbox_permission_snapshot(&root);
    let restrictions = snapshot
        .filesystem_restrictions(&[root.clone()], &[])
        .unwrap();
    assert!(restrictions.denied_paths.contains(&root.join("secret")));
}

#[test]
fn later_whole_subtree_allow_preserves_rule_order() {
    let root = root();
    let mut policy = PermissionPolicy::allow_all();
    policy
        .add_path_pattern_rule(
            AccessSelector::Read,
            PermissionMode::Deny,
            "<workspace>/secret/**",
        )
        .unwrap();
    policy
        .add_path_pattern_rule(
            AccessSelector::Read,
            PermissionMode::Allow,
            "<workspace>/secret/*",
        )
        .unwrap();
    let snapshot = ExecutionPrincipal::new(policy, ToolPermissionPolicy::allow_all())
        .sandbox_permission_snapshot(&root);
    assert!(
        snapshot
            .filesystem_restrictions(&[root], &[])
            .unwrap()
            .denied_paths
            .is_empty()
    );
}

#[test]
fn write_grants_do_not_silently_grant_unapproved_reads() {
    let root = root();
    let snapshot =
        principal(serde_json::json!({"path":{"workspace":{"read":"ask","write":"allow"}}}))
            .sandbox_permission_snapshot(&root);
    assert!(matches!(
        snapshot.filesystem_restrictions(&[], &[root.clone()]),
        Err(SandboxPolicyError::ReadApprovalRequired { .. })
    ));
    assert!(
        snapshot
            .filesystem_restrictions(&[root.clone()], &[root])
            .is_ok()
    );
}

#[test]
fn persisted_denials_cannot_be_overridden_by_static_or_later_ceiling_allow() {
    let root = root();
    let target = root.join("saved-secret");
    let snapshot = principal(serde_json::json!({}))
        .sandbox_permission_snapshot(&root)
        .with_persisted_rules(vec![SandboxPersistedRule {
            action: PermissionAction::PathAccess {
                access_kind: "read".into(),
                workspace_root: normalize_path_string(&root),
                target_path: normalize_path_string(&target),
            },
            mode: PermissionMode::Deny,
        }]);
    assert!(
        snapshot
            .filesystem_restrictions(&[root], &[])
            .unwrap()
            .denied_paths
            .contains(&target)
    );
}

#[test]
fn aliases_are_captured_before_child_environment_changes() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path":{"rules":{
        "<tmp>/**":{"read":"deny"},
        "<state>/secret/**":{"read":"deny"},
        "<cwd>/private/**":{"read":"deny"}
    }}}))
    .sandbox_permission_snapshot(&root);
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains("<tmp>"));
    assert!(!json.contains("<state>"));
    assert!(!json.contains("<cwd>"));
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Read, &std::env::temp_dir().join("other-file")),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn literal_directory_allow_does_not_grant_denied_descendants() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path":{
        "workspace":{"read":"deny"},
        "rules":{"<workspace>/input":{"read":"allow"}}
    }}))
    .sandbox_permission_snapshot(&root);
    assert!(matches!(
        snapshot.filesystem_restrictions(&[root.join("input")], &[]),
        Err(SandboxPolicyError::Unsupported { .. })
    ));
}

#[test]
fn explicit_denials_outside_grants_protect_platform_baseline_access() {
    let root = root();
    let outside = root.with_file_name("agena-sandbox-outside-secret");
    let snapshot = principal(serde_json::json!({"path":{"rules":{
        outside.to_string_lossy():{"read":"deny"}
    }}}))
    .sandbox_permission_snapshot(&root);
    assert!(
        snapshot
            .filesystem_restrictions(&[root], &[])
            .unwrap()
            .denied_paths
            .contains(&outside)
    );
}

#[test]
fn resolved_public_address_inherits_approved_domain_in_deny_default_policy() {
    let snapshot = principal(serde_json::json!({"network":{
        "internet":"deny","private":"deny","loopback":"deny",
        "rules":{"*":"deny","example.com:443":"allow"}
    }}))
    .sandbox_permission_snapshot(&root());
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"8.8.8.8:443".parse().unwrap()
        ),
        PermissionDecision::Allow
    ));
    let automatic = principal(serde_json::json!({"network":{"internet":"auto"}}))
        .sandbox_permission_snapshot(&root());
    assert!(matches!(
        automatic.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"8.8.8.8:443".parse().unwrap()
        ),
        PermissionDecision::Allow
    ));
}

#[test]
fn resolved_ip_restrictions_from_each_ceiling_override_domain_approval() {
    for pattern in ["8.8.8.8:443", "8.8.8.0/24:443", "8.8.*:443"] {
        let base = principal(
            serde_json::json!({"network":{"internet":"deny","rules":{"example.com:443":"allow"}}}),
        );
        let ceiling: PermissionConfig =
            serde_json::from_value(serde_json::json!({"network":{"rules":{pattern:"deny"}}}))
                .unwrap();
        let snapshot = base
            .try_apply_permission_ceiling(&ceiling)
            .unwrap()
            .sandbox_permission_snapshot(&root());
        for target in ["8.8.8.8:443", "[::ffff:8.8.8.8]:443"] {
            assert!(
                matches!(
                    snapshot.authorize_resolved_network_address(
                        &"example.com:443".parse().unwrap(),
                        &target.parse().unwrap()
                    ),
                    PermissionDecision::Deny { .. }
                ),
                "{pattern}: {target}"
            );
        }
    }
}

#[test]
fn cross_class_dns_requires_resolved_ip_allow() {
    for mode in ["deny", "ask", "auto"] {
        let snapshot = principal(serde_json::json!({"network":{
            "internet":"allow","private":mode,"loopback":mode
        }}))
        .sandbox_permission_snapshot(&root());
        for target in ["10.0.0.2:443", "127.0.0.1:443", "[::ffff:127.0.0.1]:443"] {
            assert!(
                !matches!(
                    snapshot.authorize_resolved_network_address(
                        &"example.com:443".parse().unwrap(),
                        &target.parse().unwrap()
                    ),
                    PermissionDecision::Allow
                ),
                "{mode}: {target}"
            );
        }
        assert!(!matches!(
            snapshot.authorize_resolved_network_address(
                &"service.local:443".parse().unwrap(),
                &"127.0.0.1:443".parse().unwrap()
            ),
            PermissionDecision::Allow
        ));
    }
    let snapshot = principal(serde_json::json!({"network":{
        "internet":"allow","private":"deny","rules":{"10.0.0.2:443":"allow"}
    }}))
    .sandbox_permission_snapshot(&root());
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"10.0.0.2:443".parse().unwrap()
        ),
        PermissionDecision::Allow
    ));
}

#[test]
fn resolved_numeric_ip_denial_retains_persisted_scope_overrides() {
    let action = PermissionAction::NetworkAccess {
        target: "https://8.8.8.8/resource".into(),
        host: "8.8.8.8".into(),
        port: Some(443),
    };
    let snapshot = principal(serde_json::json!({"network":{"internet":"allow"}}))
        .sandbox_permission_snapshot(&root())
        .with_persisted_rules(vec![SandboxPersistedRule {
            action: action.clone(),
            mode: PermissionMode::Deny,
        }]);
    let host = "example.com:443".parse().unwrap();
    let ip = "8.8.8.8:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &ip),
        PermissionDecision::Deny { .. }
    ));
    let mut snapshot = snapshot;
    snapshot.persisted_rules.push(SandboxPersistedRule {
        action,
        mode: PermissionMode::Allow,
    });
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &ip),
        PermissionDecision::Allow
    ));
}

#[test]
fn resolved_network_targets_keep_port_and_numeric_address_identity() {
    let snapshot = principal(serde_json::json!({"network":{"internet":"ask"}}))
        .sandbox_permission_snapshot(&root());
    let host = "example.com:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &"8.8.8.8:80".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &"other.com:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    let ip = "8.8.8.8:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&ip, &ip),
        PermissionDecision::Allow
    ));
}
