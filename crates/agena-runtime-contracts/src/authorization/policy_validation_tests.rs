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
fn native_network_overlays_replace_rule_arrays_and_clear_explicit_denials() {
    let base = config(json!({"network":{"egress":{
        "default":"deny",
        "allow":[{"to":[{"cidr":"192.0.2.0/24"}]}],
        "deny":[{"to":[{"cidr":"192.0.2.13/32"}]}]
    }}}));
    let original =
        apply_to_network_permission_policy(&base, NetworkPermissionPolicy::allow_all()).unwrap();
    assert!(matches!(
        original.check_connect(&"192.0.2.14:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        original.check_connect(&"192.0.2.13:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    let overlay = config(json!({"network":{"egress":{
        "allow":[{"to":[{"cidr":"198.51.100.0/24"}]}],
        "deny":[]
    }}}));
    let effective = base.merged_with(&overlay);
    let policy =
        apply_to_network_permission_policy(&effective, NetworkPermissionPolicy::allow_all())
            .unwrap();
    assert!(matches!(
        policy.check_connect(&"192.0.2.14:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        policy.check_connect(&"198.51.100.7:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
    assert_eq!(
        effective.network.unwrap().egress.unwrap().deny,
        Some(vec![])
    );
}

#[test]
fn native_network_rules_accept_cidrs_exclusions_and_transport_selectors() {
    for rule in [
        json!({}),
        json!({"to":[{"cidr":"192.0.2.7/32"}]}),
        json!({"to":[{"cidr":"192.0.2.0/24","except":["192.0.2.128/25"]}]}),
        json!({"to":[{"cidr":"2001:db8::/32"}]}),
        json!({"ports":[{"protocol":"tcp","port":443}]}),
        json!({"ports":[{"protocol":"udp","port":50000,"endPort":50010}]}),
        json!({"ports":[{"protocol":"icmp"}]}),
        json!({"ports":[{"protocol":"any","port":53}]}),
    ] {
        let cfg = config(json!({"network":{"egress":{"allow":[rule.clone()]}}}));
        apply_to_network_permission_policy(&cfg, NetworkPermissionPolicy::allow_all())
            .unwrap_or_else(|error| panic!("{rule}: {error}"));
    }
    for rule in [
        json!({"to":[]}),
        json!({"ports":[]}),
        json!({"to":[{"cidr":"example.com"}]}),
        json!({"to":[{"cidr":"https://example.com/path"}]}),
        json!({"to":[{"cidr":"192.0.2.1"}]}),
        json!({"to":[{"cidr":"192.0.2.0/24:443"}]}),
        json!({"to":[{"cidr":"192.0.2.0/33"}]}),
        json!({"to":[{"cidr":"192.0.2.0/24","except":["198.51.100.0/24"]}]}),
        json!({"ports":[{"protocol":"tcp","port":0}]}),
        json!({"ports":[{"protocol":"udp","endPort":53}]}),
        json!({"ports":[{"protocol":"tcp","port":443,"endPort":80}]}),
        json!({"ports":[{"protocol":"icmp","port":53}]}),
    ] {
        let cfg = config(json!({"network":{"egress":{"allow":[rule.clone()]}}}));
        assert!(
            apply_to_network_permission_policy(&cfg, NetworkPermissionPolicy::allow_all()).is_err(),
            "unsupported native rule accepted: {rule}"
        );
    }
}
