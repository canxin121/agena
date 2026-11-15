//! Exercise the real JSON editing boundary, including changes that only
//! reorder keys. Map equality alone cannot detect a policy-order change.
use std::fs;

use agena_domain::{PermissionConfig, PermissionDecision};
use agena_plugin_host::sdk::ToolTag;
use agena_runtime_config::{
    ConfigSettingsEditOptions, ConfigSettingsLayer, ConfigSettingsSetInput,
    set_layered_file_setting, set_runtime_file_setting,
};
#[cfg(not(target_os = "macos"))]
use agena_runtime_contracts::{
    authorization::apply_to_network_permission_policy, permission::NetworkPermissionPolicy,
};
use agena_runtime_contracts::{
    authorization::apply_to_tool_permission_policy, permission::ToolPermissionPolicy,
};

fn input(raw: &str) -> ConfigSettingsSetInput {
    ConfigSettingsSetInput {
        path: "permission".into(),
        value: serde_json::from_str(raw).unwrap(),
        options: ConfigSettingsEditOptions {
            validate: false,
            ..Default::default()
        },
    }
}

fn persisted_permission(path: &std::path::Path) -> PermissionConfig {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    serde_json::from_value(value["permission"].clone()).unwrap()
}

#[test]
fn both_save_services_preserve_command_order_and_persist_order_only_updates() {
    let deny_last =
        r#"{"tools":{"rules":{"agena.shell.exec":{"git status":"allow","git *":"deny"}}}}"#;
    let allow_last =
        r#"{"tools":{"rules":{"agena.shell.exec":{"git *":"deny","git status":"allow"}}}}"#;
    for layered in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("global.json");
        let workspace = root.path().join("project/.agena/agena.json");
        let mut previous_revision = None;
        for (raw, expect_allowed) in [(deny_last, false), (allow_last, true), (deny_last, false)] {
            let response = if layered {
                set_layered_file_setting(&file, &workspace, ConfigSettingsLayer::Global, input(raw))
                    .unwrap()
            } else {
                set_runtime_file_setting(&file, input(raw), None).unwrap()
            };
            assert!(response.changed, "an order-only update must be written");
            if let Some(previous) = previous_revision {
                assert_eq!(response.before_revision, previous);
                assert_ne!(response.after_revision, response.before_revision);
            }
            previous_revision = Some(response.after_revision);
            let permission = persisted_permission(&file);
            let policy =
                apply_to_tool_permission_policy(&permission, ToolPermissionPolicy::allow_all())
                    .unwrap();
            assert_eq!(
                matches!(
                    policy.check_tool("agena.shell.exec", Some("git status"), &[ToolTag::Shell]),
                    PermissionDecision::Allow
                ),
                expect_allowed,
                "layered={layered}, raw={raw}"
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn saved_native_network_denies_override_allows_regardless_of_json_key_order() {
    let deny_last = r#"{"network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":443}]}],"deny":[{"to":[{"cidr":"192.0.2.42/32"}]}]}}}"#;
    let allow_last = r#"{"network":{"egress":{"default":"deny","deny":[{"to":[{"cidr":"192.0.2.42/32"}]}],"allow":[{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":443}]}]}}}"#;
    for layered in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("global.json");
        let workspace = root.path().join("project/.agena/agena.json");
        for raw in [deny_last, allow_last] {
            if layered {
                set_layered_file_setting(&file, &workspace, ConfigSettingsLayer::Global, input(raw))
                    .unwrap()
            } else {
                set_runtime_file_setting(&file, input(raw), None).unwrap()
            };
            let policy = apply_to_network_permission_policy(
                &persisted_permission(&file),
                NetworkPermissionPolicy::allow_all(),
            )
            .unwrap();
            assert!(matches!(
                policy.check_connect(&"192.0.2.41:443".parse().unwrap()),
                PermissionDecision::Allow
            ));
            assert!(matches!(
                policy.check_connect(&"192.0.2.42:443".parse().unwrap()),
                PermissionDecision::Deny { .. }
            ));
        }
    }
}
