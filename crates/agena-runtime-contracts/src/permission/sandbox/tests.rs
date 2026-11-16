use super::*;
use crate::authorization::{ExecutionPrincipal, PermissionConfig};

fn root() -> PathBuf {
    std::env::temp_dir().join("agena-sandbox-policy-tests")
}

// These low-level snapshot regressions also exercise captured approvals and
// legacy in-memory patterns. Build that internal state explicitly: public JSON
// acceptance/migration is tested in authorization::path_rules and agena-domain.
fn principal(mut value: serde_json::Value) -> ExecutionPrincipal {
    let mut policy = PermissionPolicy::allow_all();
    if let Some(path) = value.as_object_mut().unwrap().remove("path") {
        let mode = |value: &serde_json::Value| {
            serde_json::from_value::<PermissionMode>(value.clone()).unwrap()
        };
        if let Some(workspace) = path.get("workspace") {
            if let Some(read) = workspace.get("read") {
                policy.workspace_read_default = mode(read);
            }
            if let Some(write) = workspace.get("write") {
                policy.workspace_write_default = mode(write);
            }
        }
        if let Some(external) = path.get("external") {
            if let Some(read) = external.get("read") {
                policy.external_read_default = mode(read);
            }
            if let Some(write) = external.get("write") {
                policy.external_write_default = mode(write);
            }
        }
        if let Some(rules) = path.get("rules").and_then(serde_json::Value::as_object) {
            for (pattern, modes) in rules {
                if let Some(read) = modes.get("read") {
                    policy
                        .add_path_pattern_rule(AccessSelector::Read, mode(read), pattern)
                        .unwrap();
                }
                if let Some(write) = modes.get("write") {
                    policy
                        .add_path_pattern_rule(AccessSelector::Write, mode(write), pattern)
                        .unwrap();
                }
            }
        }
    }
    let network = value
        .as_object_mut()
        .unwrap()
        .remove("network")
        .map(|value| NetworkPermissionConfig::from_legacy_value(value).unwrap());
    let mut config: PermissionConfig = serde_json::from_value(value).unwrap();
    config.network = network;
    ExecutionPrincipal::new(policy, ToolPermissionPolicy::allow_all())
        .try_apply_permission_config(&config)
        .unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn final_native_network_policy_rejects_combined_platform_budget_before_host_io() {
    let deny_ports = |start: u16, end: u16| {
        let mut network = agena_domain::NetworkPermissionConfig::allow_all();
        network.egress.as_mut().unwrap().deny = Some(vec![NetworkRuleConfig {
            to: None,
            ports: Some(
                (start..=end)
                    .map(|port| NetworkPortConfig {
                        protocol: Some(NetworkProtocol::Tcp),
                        port: Some(port),
                        end_port: None,
                    })
                    .collect(),
            ),
        }]);
        network.validate_for_platform("linux").unwrap();
        PermissionConfig {
            network: Some(network),
            ..Default::default()
        }
    };
    let base = deny_ports(1, 16_384);
    let ceiling = deny_ports(16_385, 32_768);
    let snapshot = ExecutionPrincipal::new(
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
    )
    .try_apply_permission_config(&base)
    .unwrap()
    .try_apply_permission_ceiling(&ceiling)
    .unwrap()
    .sandbox_permission_snapshot(&root());
    // Each input expands to 32768 entries plus the gateway deny; their union
    // expands to 65536 plus the gateway and must fail for host callbacks too.
    let error = snapshot
        .native_network_policy(None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("65536 firewall rules"), "{error}");
    assert!(matches!(
        snapshot.authorize_network_connect(&"udp://203.0.113.7:443".parse().unwrap()),
        PermissionDecision::Deny { reason } if reason.contains("65536 firewall rules")
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn final_native_network_policy_rejects_unsupported_seatbelt_rules_before_host_io() {
    let mut snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
        .sandbox_permission_snapshot(&root());
    snapshot.network_layers.push(
        serde_json::from_value(serde_json::json!({
            "default":"deny","allow":[{"to":"192.0.2.0/24"}]
        }))
        .unwrap(),
    );
    let error = snapshot
        .native_network_policy(None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Seatbelt"), "{error}");
    assert!(matches!(
        snapshot.authorize_network_connect(&"192.0.2.7:443".parse().unwrap()),
        PermissionDecision::Deny { reason } if reason.contains("Seatbelt")
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn serialization_preserves_all_ceiling_layers_and_network_rules() {
    let root = root();
    let parent = principal(serde_json::json!({
        "path":{"rules":{"<workspace>/secret/**":{"read":"deny"}}},
        "network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":443}]}]}}
    }));
    let ceiling: PermissionConfig = serde_json::from_value(serde_json::json!({
        "path":{"rules":{"<workspace>/locked":"readonly"}},
        "network":{"default":"allow","deny":[{"to":"192.0.2.13"}]}
    }))
    .unwrap();
    let later: PermissionConfig = serde_json::from_value(serde_json::json!({
        "path":{"workspace":{"read":"allow","write":"allow"}},
        "network":{"default":"allow"}
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
        "192.0.2.13:443",
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
        snapshot.authorize_network_connect(&"192.0.2.14:443".parse().unwrap()),
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

#[cfg(not(target_os = "macos"))]
#[test]
fn resolved_hostnames_use_only_numeric_native_grants() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{
        "default":"deny",
        "allow":[{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":443}]}]
    }}}))
    .sandbox_permission_snapshot(&root());
    assert!(matches!(
        snapshot.authorize_network_connect(&"example.com:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"192.0.2.7:443".parse().unwrap()
        ),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"198.51.100.7:443".parse().unwrap()
        ),
        PermissionDecision::Deny { .. }
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn resolved_native_denials_from_each_ceiling_survive_later_broad_allow() {
    for cidr in ["192.0.2.7/32", "192.0.2.0/24", "192.0.0.0/16"] {
        let base = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}));
        let ceiling: PermissionConfig = serde_json::from_value(serde_json::json!({
            "network":{
                "default":"allow",
                "deny":[{"to":cidr,"protocol":"tcp","ports":[443]}]
            }
        }))
        .unwrap();
        let later: PermissionConfig = serde_json::from_value(serde_json::json!({
            "network":{"default":"allow"}
        }))
        .unwrap();
        let snapshot = base
            .try_apply_permission_ceiling(&ceiling)
            .unwrap()
            .try_apply_permission_ceiling(&later)
            .unwrap()
            .sandbox_permission_snapshot(&root());
        let compiled =
            NetworkPermissionPolicy::from_config(&snapshot.native_network_policy(None).unwrap())
                .unwrap();
        for target in ["192.0.2.7:443", "[::ffff:192.0.2.7]:443"] {
            assert!(
                matches!(
                    snapshot.authorize_resolved_network_address(
                        &"example.com:443".parse().unwrap(),
                        &target.parse().unwrap()
                    ),
                    PermissionDecision::Deny { .. }
                ),
                "{cidr}: {target}"
            );
            assert!(
                matches!(
                    compiled.check_connect(&target.parse().unwrap()),
                    PermissionDecision::Deny { .. }
                ),
                "{cidr}: {target}"
            );
        }
        assert!(matches!(
            compiled.check_connect(&"198.51.100.7:443".parse().unwrap()),
            PermissionDecision::Allow
        ));
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn resolved_private_addresses_require_an_explicit_native_grant() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{
        "default":"deny","allow":[{"to":[{"cidr":"192.0.2.0/24"}]}]
    }}}))
    .sandbox_permission_snapshot(&root());
    for target in ["10.0.0.2:443", "127.0.0.1:443", "[::ffff:127.0.0.1]:443"] {
        assert!(
            matches!(
                snapshot.authorize_resolved_network_address(
                    &"example.com:443".parse().unwrap(),
                    &target.parse().unwrap()
                ),
                PermissionDecision::Deny { .. }
            ),
            "{target}"
        );
    }
    let snapshot = principal(serde_json::json!({"network":{"egress":{
        "default":"deny","allow":[{
            "to":[{"cidr":"10.0.0.2/32"}],
            "ports":[{"protocol":"tcp","port":443}]
        }]
    }}}))
    .sandbox_permission_snapshot(&root());
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"10.0.0.2:443".parse().unwrap()
        ),
        PermissionDecision::Allow
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn persisted_numeric_denials_are_compiled_and_latest_decision_is_preserved() {
    let action = PermissionAction::NetworkAccess {
        target: "https://192.0.2.7/".into(),
        host: "192.0.2.7".into(),
        port: Some(443),
    };
    let mut snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
        .sandbox_permission_snapshot(&root())
        .with_persisted_rules(vec![SandboxPersistedRule {
            action: action.clone(),
            mode: PermissionMode::Deny,
        }]);
    let host = "example.com:443".parse().unwrap();
    let ip = "192.0.2.7:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &ip),
        PermissionDecision::Deny { .. }
    ));
    let compiled =
        NetworkPermissionPolicy::from_config(&snapshot.native_network_policy(None).unwrap())
            .unwrap();
    assert!(matches!(
        compiled.check_connect(&ip),
        PermissionDecision::Deny { .. }
    ));
    snapshot.persisted_rules.push(SandboxPersistedRule {
        action,
        mode: PermissionMode::Allow,
    });
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&host, &ip),
        PermissionDecision::Allow
    ));
    let compiled =
        NetworkPermissionPolicy::from_config(&snapshot.native_network_policy(None).unwrap())
            .unwrap();
    assert!(matches!(
        compiled.check_connect(&ip),
        PermissionDecision::Allow
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn persisted_allow_cannot_reopen_a_native_denial() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{
        "default":"allow","deny":[{"to":[{"cidr":"192.0.2.0/24"}]}]
    }}}))
    .sandbox_permission_snapshot(&root())
    .with_persisted_rules(vec![SandboxPersistedRule {
        action: PermissionAction::NetworkAccess {
            target: "192.0.2.7:443".into(),
            host: "192.0.2.7".into(),
            port: Some(443),
        },
        mode: PermissionMode::Allow,
    }]);
    let target = "192.0.2.7:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_network_connect(&target),
        PermissionDecision::Deny { .. }
    ));
    let compiled =
        NetworkPermissionPolicy::from_config(&snapshot.native_network_policy(None).unwrap())
            .unwrap();
    assert!(matches!(
        compiled.check_connect(&target),
        PermissionDecision::Deny { .. }
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn persisted_network_denials_preserve_transport_even_without_a_port() {
    for (target, port, denied_protocol, allowed_protocol) in [
        (
            "udp://192.0.2.53:53",
            Some(53),
            NetworkProtocol::Udp,
            NetworkProtocol::Tcp,
        ),
        (
            "tcp://192.0.2.53",
            None,
            NetworkProtocol::Tcp,
            NetworkProtocol::Udp,
        ),
        (
            "icmp://192.0.2.53",
            None,
            NetworkProtocol::Icmp,
            NetworkProtocol::Tcp,
        ),
    ] {
        let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
            .sandbox_permission_snapshot(&root())
            .with_persisted_rules(vec![SandboxPersistedRule {
                action: PermissionAction::NetworkAccess {
                    target: target.into(),
                    host: "192.0.2.53".into(),
                    port,
                },
                mode: PermissionMode::Deny,
            }]);
        let compiled =
            NetworkPermissionPolicy::from_config(&snapshot.native_network_policy(None).unwrap())
                .unwrap();
        let address = "192.0.2.53".parse().unwrap();
        assert!(
            matches!(
                compiled.check_endpoint(address, denied_protocol, port),
                PermissionDecision::Deny { .. }
            ),
            "{target}"
        );
        assert!(
            matches!(
                compiled.check_endpoint(address, allowed_protocol, Some(53)),
                PermissionDecision::Allow
            ),
            "{target}"
        );
    }
}

#[test]
fn effective_stored_network_approval_modes_fail_closed() {
    for mode in [PermissionMode::Ask, PermissionMode::Auto] {
        let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
            .sandbox_permission_snapshot(&root())
            .with_persisted_rules(vec![SandboxPersistedRule {
                action: PermissionAction::NetworkAccess {
                    target: "192.0.2.7:443".into(),
                    host: "192.0.2.7".into(),
                    port: Some(443),
                },
                mode,
            }]);
        let error = snapshot
            .native_network_policy(None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ask/auto"), "{error}");
        assert!(error.contains("replace or remove"), "{error}");
        assert!(matches!(
            snapshot.authorize_network_connect(&"192.0.2.7:443".parse().unwrap()),
            PermissionDecision::Deny { .. }
        ));
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn later_concrete_decisions_override_historical_approval_modes_for_the_same_action() {
    let action = PermissionAction::NetworkAccess {
        target: "192.0.2.7:443".into(),
        host: "192.0.2.7".into(),
        port: Some(443),
    };
    for historical in [PermissionMode::Ask, PermissionMode::Auto] {
        for current in [PermissionMode::Allow, PermissionMode::Deny] {
            let rules = vec![
                SandboxPersistedRule {
                    action: action.clone(),
                    mode: historical,
                },
                SandboxPersistedRule {
                    action: action.clone(),
                    mode: current,
                },
            ];
            let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
                .sandbox_permission_snapshot(&root())
                .with_persisted_rules(rules.clone());
            let compiled = NetworkPermissionPolicy::from_config(
                &snapshot.native_network_policy(None).unwrap(),
            )
            .unwrap();
            assert_eq!(
                matches!(
                    compiled.check_connect(&"192.0.2.7:443".parse().unwrap()),
                    PermissionDecision::Allow
                ),
                current == PermissionMode::Allow
            );
            let mut reversed = rules;
            reversed.reverse();
            let mut snapshot = snapshot;
            snapshot.persisted_rules = reversed;
            assert!(
                snapshot
                    .native_network_policy(None)
                    .unwrap_err()
                    .to_string()
                    .contains("ask/auto")
            );
        }
    }
}

#[test]
fn malformed_stored_network_identities_fail_even_when_the_decision_is_allow() {
    for action in [
        PermissionAction::NetworkAccess {
            target: "example.com:443".into(),
            host: "example.com".into(),
            port: Some(443),
        },
        PermissionAction::NetworkAccess {
            target: "https://192.0.2.7/private".into(),
            host: "192.0.2.7".into(),
            port: Some(443),
        },
        PermissionAction::NetworkAccess {
            target: "192.0.2.7:443".into(),
            host: "192.0.2.8".into(),
            port: Some(443),
        },
        PermissionAction::NetworkAccess {
            target: "192.0.2.7:443".into(),
            host: "192.0.2.7".into(),
            port: Some(80),
        },
    ] {
        for mode in [PermissionMode::Allow, PermissionMode::Deny] {
            let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
                .sandbox_permission_snapshot(&root())
                .with_persisted_rules(vec![SandboxPersistedRule {
                    action: action.clone(),
                    mode,
                }]);
            let error = snapshot
                .native_network_policy(None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("stored network decision"), "{error}");
            assert!(error.contains("replace or remove"), "{error}");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_rejects_effective_stored_numeric_decisions_including_allows() {
    for mode in [PermissionMode::Allow, PermissionMode::Deny] {
        let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
            .sandbox_permission_snapshot(&root())
            .with_persisted_rules(vec![SandboxPersistedRule {
                action: PermissionAction::NetworkAccess {
                    target: "192.0.2.7:443".into(),
                    host: "192.0.2.7".into(),
                    port: Some(443),
                },
                mode,
            }]);
        assert!(
            snapshot
                .native_network_policy(None)
                .unwrap_err()
                .to_string()
                .contains("Seatbelt")
        );
    }
}

#[test]
fn persisted_hostname_denials_fail_native_compilation_with_a_migration_error() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
        .sandbox_permission_snapshot(&root())
        .with_persisted_rules(vec![SandboxPersistedRule {
            action: PermissionAction::NetworkAccess {
                target: "example.com:443".into(),
                host: "example.com".into(),
                port: Some(443),
            },
            mode: PermissionMode::Deny,
        }]);
    let error = snapshot
        .native_network_policy(None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stored network decision"), "{error}");
    assert!(error.contains("literal IP"), "{error}");
    assert!(error.contains("replace or remove"), "{error}");
}

#[test]
fn resolved_network_targets_keep_port_and_numeric_address_identity() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{"default":"allow"}}}))
        .sandbox_permission_snapshot(&root());
    let host = "example.com:443".parse().unwrap();
    for resolved in ["192.0.2.7:80", "other.com:443"] {
        assert!(matches!(
            snapshot.authorize_resolved_network_address(&host, &resolved.parse().unwrap()),
            PermissionDecision::Deny { .. }
        ));
    }
    let ip = "192.0.2.7:443".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&ip, &"192.0.2.8:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        snapshot.authorize_resolved_network_address(&ip, &ip),
        PermissionDecision::Allow
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn resolved_network_targets_preserve_the_declared_transport_protocol() {
    let snapshot = principal(serde_json::json!({"network":{"egress":{
        "default":"deny","allow":[{
            "to":[{"cidr":"192.0.2.53/32"}],
            "ports":[{"protocol":"udp","port":53}]
        }]
    }}}))
    .sandbox_permission_snapshot(&root());
    let resolved = "192.0.2.53:53".parse().unwrap();
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"udp://example.com:53".parse().unwrap(),
            &resolved
        ),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"tcp://example.com:53".parse().unwrap(),
            &resolved
        ),
        PermissionDecision::Deny { .. }
    ));
}
