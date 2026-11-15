use super::*;
use agena_plugin_sdk::ToolTag;
use serde_json::json;

fn config(value: serde_json::Value) -> PermissionConfig {
    serde_json::from_value(value).unwrap()
}

fn shell_policy(value: serde_json::Value) -> ToolPermissionPolicy {
    apply_to_tool_permission_policy(&config(value), ToolPermissionPolicy::allow_all()).unwrap()
}

#[test]
fn command_rules_keep_document_order_including_the_catch_all() {
    let policy = shell_policy(json!({"tools": {"rules": {
        "agena.shell.exec": {"*": "allow", "git push *": "deny"}
    }}}));
    assert!(matches!(
        policy.check_tool(
            "agena.shell.exec",
            Some("git push origin main"),
            &[ToolTag::Shell]
        ),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        policy.check_tool("agena.shell.exec", Some("git status"), &[ToolTag::Shell]),
        PermissionDecision::Allow
    ));
    let later_broad = shell_policy(json!({"tools": {"rules": {
        "agena.shell.exec": {"git status": "allow", "git *": "deny"}
    }}}));
    assert!(matches!(
        later_broad.check_tool("agena.shell.exec", Some("git status"), &[ToolTag::Shell]),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn a_named_allow_cannot_swallow_an_explicit_command_deny() {
    let policy = shell_policy(json!({"tools": {
        "names": {"agena.shell.exec": "allow"},
        "rules": {"agena.shell.exec": {"git push *": "deny"}}
    }}));
    assert!(matches!(
        policy.check_tool(
            "agena.shell.exec",
            Some("git push origin main"),
            &[ToolTag::Shell]
        ),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn named_decisions_override_class_defaults() {
    let policy = shell_policy(json!({"tools": {
        "names": {"agena.shell.exec": "ask", "agena.fs.read": "deny"},
        "rules": {
            "agena.shell.exec": {"routine": "allow"},
            "*": {"read-only": "allow"}
        }
    }}));
    assert!(matches!(
        policy.check_tool("agena.shell.exec", Some("git status"), &[ToolTag::Shell]),
        PermissionDecision::Ask { .. }
    ));
    assert!(matches!(
        policy.check_tool("agena.fs.read", None, &[ToolTag::ReadOnly]),
        PermissionDecision::Deny { .. }
    ));
}

#[test]
fn command_allows_do_not_approve_arbitrary_persistent_terminal_input() {
    let policy = shell_policy(json!({"tools": {
        "names": {"agena.shell.write": "ask"},
        "rules": {"agena.shell.write": {"*": "allow"}}
    }}));
    assert!(matches!(
        policy.check_tool(
            "agena.shell.write",
            Some("print(secret)"),
            &[ToolTag::Shell]
        ),
        PermissionDecision::Ask { .. }
    ));
}

#[test]
fn unsupported_tool_conditions_fail_instead_of_being_ignored() {
    for value in [
        json!({"tools": {"rules": {"agena.fs.read": {"**/secret/**": "deny"}}}}),
        json!({"tools": {"rules": {"*": "deny"}}}),
        json!({"tools": {"rules": {"*": {"routine": "allow"}}}}),
        json!({"tools": {"rules": {"agena.shell.exec": {"": "deny"}}}}),
        json!({"tools": {"rules": {"agena.shell.exec": {"123": "deny"}}}}),
        json!({"tools": {"names": {"agena.fs.*": "allow"}}}),
    ] {
        let error =
            apply_to_tool_permission_policy(&config(value), ToolPermissionPolicy::allow_all())
                .expect_err("unsupported conditions must not silently disappear");
        assert!(error.to_string().contains("permission.tools"), "{error}");
    }
}

#[test]
fn conflicting_tool_aliases_cannot_be_lost_during_deserialization() {
    for value in [
        json!({"tools": {"names": {"shell.exec": "allow", "agena.shell.exec": "deny"}}}),
        json!({"tools": {"rules": {"shell.exec": {"*": "allow"}, "agena.shell.exec": {"*": "deny"}}}}),
    ] {
        let error = serde_json::from_value::<PermissionConfig>(value).unwrap_err();
        assert!(error.to_string().contains("same tool"), "{error}");
    }
}

#[test]
fn network_overlay_reordering_changes_effective_authorization() {
    let base: PermissionConfig = serde_json::from_str(
        r#"{"network":{"rules":{
            "8.8.8.8:443":"deny","8.0.0.0/8":"allow","9.9.9.9":"deny"
        }}}"#,
    )
    .unwrap();
    let overlay: PermissionConfig =
        serde_json::from_str(r#"{"network":{"rules":{"8.0.0.0/8":"allow","8.8.8.8:443":"deny"}}}"#)
            .unwrap();
    let target = "8.8.8.8:443".parse().unwrap();
    let original =
        apply_to_network_permission_policy(&base, NetworkPermissionPolicy::allow_all()).unwrap();
    assert!(matches!(
        original.check_connect(&target),
        PermissionDecision::Allow
    ));

    let effective = base.merged_with(&overlay);
    let policy =
        apply_to_network_permission_policy(&effective, NetworkPermissionPolicy::allow_all())
            .unwrap();
    for target in [target, "9.9.9.9:443".parse().unwrap()] {
        assert!(matches!(
            policy.check_connect(&target),
            PermissionDecision::Deny { .. }
        ));
    }
    assert!(matches!(
        policy.check_connect(&"8.8.4.4:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
}

#[test]
fn network_rules_accept_only_concrete_targets_and_valid_ports() {
    for pattern in [
        "example.com",
        "api.example.com:443",
        "localhost:8080",
        "192.0.2.1",
        "192.0.2.0/24:443",
        "[2001:db8::1]:443",
        "2001:db8::/32",
        "[2001:db8::/32]:443",
    ] {
        let cfg = config(json!({"network": {"rules": {pattern: "deny"}}}));
        apply_to_network_permission_policy(&cfg, NetworkPermissionPolicy::allow_all())
            .unwrap_or_else(|error| panic!("{pattern}: {error}"));
    }
    for pattern in [
        "*",
        "*.example.com",
        "192.0.*",
        "https://example.com/path",
        ":443",
        "example.com:+443",
        "example.com/path",
        "example.com:0",
        "example.com:*",
        "example.com:",
        "example.com:65536",
        "example.com#fragment",
        "a@b.com",
        " example.com",
        "bad host",
        "[example.com]:443",
        "[::1]junk",
        "a..example.com",
        "-bad.example.com",
        "example.com?x=1",
        "999.999.999.999",
    ] {
        let cfg = config(json!({"network": {"rules": {pattern: "deny"}}}));
        assert!(
            apply_to_network_permission_policy(&cfg, NetworkPermissionPolicy::allow_all()).is_err(),
            "unsupported target accepted: {pattern}"
        );
    }
}
