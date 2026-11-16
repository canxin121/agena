use super::*;
use crate::authorization::ExecutionPrincipal;

fn root() -> PathBuf {
    PathBuf::from(normalize_path_string(
        &std::env::temp_dir().join("agena-scoped-sandbox-policy"),
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

fn writable(snapshot: &SandboxPermissionSnapshot, target: &Path, writes: &[PathBuf]) -> bool {
    matches!(
        snapshot.check_granted_path(AccessKind::Write, target, &[], writes),
        PermissionDecision::Allow
    )
}

#[test]
fn approved_writable_child_of_readonly_parent_has_no_ancestor_restriction() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path": {
        "workspace":"deny", "external":"deny", "rules": {
            "<workspace>":"readonly", "<workspace>/output":"readwrite"
        }
    }}))
    .sandbox_permission_snapshot(&root);
    for grant in [root.join("output"), root.join("output/new.txt")] {
        let restricted = snapshot
            .filesystem_restrictions(&[], &[grant.clone()])
            .unwrap();
        assert!(restricted.readonly_paths.is_empty());
        assert!(restricted.readwrite_paths.is_empty());
        assert!(restricted.denied_paths.is_empty());
        assert!(writable(&snapshot, &grant, &[grant.clone()]));
        assert!(!writable(&snapshot, &root.join("outside.txt"), &[grant]));
    }
}

#[test]
fn approved_parent_keeps_readonly_and_explicit_denied_children() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path": {
        "workspace":"readwrite", "external":"deny", "rules": {
            "<workspace>/reference":"readonly", "<workspace>/secrets":"deny"
        }
    }}))
    .sandbox_permission_snapshot(&root);
    let writes = [root.clone()];
    let restricted = snapshot.filesystem_restrictions(&[], &writes).unwrap();
    assert_eq!(restricted.readonly_paths, [root.join("reference")]);
    assert_eq!(restricted.denied_paths, [root.join("secrets")]);
    assert!(restricted.readwrite_paths.is_empty());
    assert!(!writable(&snapshot, &root.join("reference/file"), &writes));
    assert!(!writable(&snapshot, &root.join("secrets/file"), &writes));
    assert!(writable(&snapshot, &root.join("output/file"), &writes));
}

#[test]
fn alternating_modes_restore_only_authorized_descendants_of_approved_writes() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path": {
        "workspace":"readwrite", "external":"deny", "rules": {
            "<workspace>/data":"readonly",
            "<workspace>/data/generated":"readwrite",
            "<workspace>/data/generated/reference":"readonly",
            "<workspace>/data/generated/reference/work":"readwrite"
        }
    }}))
    .sandbox_permission_snapshot(&root);
    let writes = [root.clone()];
    let restricted = snapshot.filesystem_restrictions(&[], &writes).unwrap();
    assert_eq!(
        restricted.readonly_paths,
        [root.join("data"), root.join("data/generated/reference")]
    );
    assert_eq!(
        restricted.readwrite_paths,
        [
            root.join("data/generated"),
            root.join("data/generated/reference/work")
        ]
    );
    for (path, allowed) in [
        ("file", true),
        ("data/file", false),
        ("data/generated/file", true),
        ("data/generated/reference/file", false),
        ("data/generated/reference/work/file", true),
    ] {
        assert_eq!(
            writable(&snapshot, &root.join(path), &writes),
            allowed,
            "{path}"
        );
    }
    let narrower = [root.join("data/generated")];
    let restricted = snapshot.filesystem_restrictions(&[], &narrower).unwrap();
    assert_eq!(
        restricted.readonly_paths,
        [root.join("data/generated/reference")]
    );
    assert_eq!(
        restricted.readwrite_paths,
        [root.join("data/generated/reference/work")]
    );
    assert!(!writable(&snapshot, &root.join("file"), &narrower));
    let read_only_invocation = snapshot.filesystem_restrictions(&[root], &[]).unwrap();
    assert!(read_only_invocation.readwrite_paths.is_empty());
}

#[test]
fn independent_ceiling_limits_reopened_write_boundaries() {
    let root = root();
    let snapshot = principal(serde_json::json!({"path": {
        "workspace":"readwrite", "external":"deny", "rules": {
            "<workspace>/data":"readonly", "<workspace>/data/generated":"readwrite"
        }
    }}))
    .try_apply_permission_ceiling(
        &serde_json::from_value(serde_json::json!({"path": {
            "workspace":"readwrite", "external":"deny", "rules": {
                "<workspace>/data/generated":"readonly",
                "<workspace>/data/generated/output":"readwrite",
                "<workspace>/data/generated/secrets":"deny"
            }
        }}))
        .unwrap(),
    )
    .unwrap()
    .sandbox_permission_snapshot(&root);
    let writes = [root.clone()];
    let restricted = snapshot.filesystem_restrictions(&[], &writes).unwrap();
    assert_eq!(
        restricted.readonly_paths,
        [root.join("data"), root.join("data/generated")]
    );
    assert_eq!(
        restricted.readwrite_paths,
        [root.join("data/generated/output")]
    );
    assert_eq!(
        restricted.denied_paths,
        [root.join("data/generated/secrets")]
    );
    assert!(!writable(
        &snapshot,
        &root.join("data/generated/file"),
        &writes
    ));
    assert!(writable(
        &snapshot,
        &root.join("data/generated/output/file"),
        &writes
    ));
    assert!(!writable(
        &snapshot,
        &root.join("data/generated/secrets/file"),
        &writes
    ));
}

#[test]
fn persisted_denial_cannot_be_reopened_by_a_static_write_grant() {
    let root = root();
    let base = principal(serde_json::json!({"path": {
        "workspace":"readwrite", "external":"deny", "rules": {
            "<workspace>/data/generated":"readwrite"
        }
    }}))
    .sandbox_permission_snapshot(&root);
    for access in ["read", "write"] {
        let snapshot = base
            .clone()
            .with_persisted_rules(vec![SandboxPersistedRule {
                action: PermissionAction::PathAccess {
                    access_kind: access.into(),
                    workspace_root: normalize_path_string(&root),
                    target_path: normalize_path_string(&root.join("data")),
                },
                mode: PermissionMode::Deny,
            }]);
        let restricted = snapshot
            .filesystem_restrictions(&[], &[root.clone()])
            .unwrap();
        assert!(restricted.readwrite_paths.is_empty());
        assert!(!writable(
            &snapshot,
            &root.join("data/generated/file"),
            &[root.clone()]
        ));
        assert!(
            snapshot
                .filesystem_restrictions(&[], &[root.join("data/generated")])
                .is_err()
        );
    }
}

#[test]
fn approval_for_one_child_does_not_approve_its_ask_siblings() {
    let root = root();
    let mut policy = PermissionPolicy::allow_all();
    policy.workspace_write_default = PermissionMode::Auto;
    for pattern in ["<workspace>/private", "<workspace>/private/**"] {
        policy
            .add_path_pattern_rule(AccessSelector::Write, PermissionMode::Ask, pattern)
            .unwrap();
    }
    let snapshot = ExecutionPrincipal::new(policy, ToolPermissionPolicy::allow_all())
        .sandbox_permission_snapshot(&root);
    let writes = [root.clone(), root.join("private/approved")];
    let restricted = snapshot.filesystem_restrictions(&[], &writes).unwrap();
    assert_eq!(restricted.readonly_paths, [root.join("private")]);
    assert_eq!(restricted.readwrite_paths, [root.join("private/approved")]);
    assert!(writable(&snapshot, &root.join("ordinary/file"), &writes));
    assert!(writable(
        &snapshot,
        &root.join("private/approved/file"),
        &writes
    ));
    assert!(!writable(
        &snapshot,
        &root.join("private/sibling/file"),
        &writes
    ));
}
