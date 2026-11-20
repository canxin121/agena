use std::{fs, path::Path};

use agena_domain::PermissionDecision;
use agena_domain::{NetworkPermissionConfig, NetworkProtocol};
use agena_runtime_config::{
    ConfigEnvironment, ConfigLoader, ConfigSettingsEditOptions, ConfigSettingsLayer,
    ConfigSettingsSetInput, LoadConfigRequest, project_config_path, set_layered_file_setting,
};
#[cfg(not(target_os = "macos"))]
use agena_runtime_config::{ConfigSettingsDeleteInput, delete_layered_file_setting};
use agena_runtime_contracts::permission::NetworkPermissionPolicy;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Example {
    name: String,
    selective: bool,
    network: Value,
    checks: Vec<EndpointCheck>,
}

#[derive(Deserialize)]
struct EndpointCheck {
    ip: String,
    protocol: NetworkProtocol,
    port: Option<u16>,
    allowed: bool,
}

#[derive(Clone, Copy)]
struct EmptyEnvironment;

impl ConfigEnvironment for EmptyEnvironment {
    fn var(&self, _: &str) -> Option<String> {
        None
    }
    fn vars(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

fn examples() -> Vec<Example> {
    serde_json::from_str(include_str!("fixtures/network_rule_examples.json")).unwrap()
}

fn update(path: &str, value: Value) -> ConfigSettingsSetInput {
    ConfigSettingsSetInput {
        path: path.into(),
        value,
        options: ConfigSettingsEditOptions {
            validate: true,
            reload: false,
            ..Default::default()
        },
    }
}

fn load_network(global: &Path, workspace: &Path) -> NetworkPermissionConfig {
    ConfigLoader::new(EmptyEnvironment)
        .load(&LoadConfigRequest {
            config_path: Some(global.to_owned()),
            workspace_root: Some(workspace.to_owned()),
            ..Default::default()
        })
        .unwrap()
        .config
        .permission
        .network
        .unwrap()
}

fn assert_endpoints(example: &Example, network: &NetworkPermissionConfig) {
    let policy = NetworkPermissionPolicy::from_config(network).unwrap();
    for check in &example.checks {
        let allowed = matches!(
            policy.check_endpoint(check.ip.parse().unwrap(), check.protocol, check.port),
            PermissionDecision::Allow
        );
        assert_eq!(
            allowed, check.allowed,
            "{}: {} {:?} {:?}",
            example.name, check.ip, check.protocol, check.port
        );
    }
}

#[test]
fn valid_network_examples_save_and_reload_in_both_layers_without_changing_meaning() {
    for example in examples() {
        // Seatbelt accepts rules that lower exactly to defaults, but cannot
        // enforce remaining selective egress rules.
        if cfg!(target_os = "macos") && example.selective {
            continue;
        }
        for layer in [ConfigSettingsLayer::Global, ConfigSettingsLayer::Workspace] {
            for path in ["permission", "permission.network"] {
                let root = tempfile::tempdir().unwrap();
                let global = root.path().join("global.json");
                let workspace_root = root.path().join("workspace");
                fs::create_dir_all(&workspace_root).unwrap();
                let workspace = project_config_path(&workspace_root);
                let authored: NetworkPermissionConfig =
                    serde_json::from_value(example.network.clone()).unwrap();
                assert_endpoints(&example, &authored);
                let value = if path == "permission" {
                    json!({"network":example.network})
                } else {
                    example.network.clone()
                };
                let response =
                    set_layered_file_setting(&global, &workspace, layer, update(path, value))
                        .unwrap_or_else(|error| {
                            panic!("{} in {layer:?} at {path}: {error}", example.name)
                        });
                assert!(response.validated);
                let selected = if matches!(layer, ConfigSettingsLayer::Global) {
                    &global
                } else {
                    &workspace
                };
                let saved: Value = serde_json::from_slice(&fs::read(selected).unwrap()).unwrap();
                let stored: NetworkPermissionConfig =
                    serde_json::from_value(saved["permission"]["network"].clone()).unwrap();
                assert_eq!(stored, authored, "{} saved policy", example.name);
                let effective = load_network(&global, &workspace_root);
                assert_eq!(effective, authored, "{} effective policy", example.name);
                assert_endpoints(&example, &effective);
            }
        }
    }
}

#[test]
fn network_capability_checks_follow_the_native_backends_for_each_default_combination() {
    for default in ["deny", "allow"] {
        for inbound in ["deny", "allow"] {
            for loopback in ["deny", "allow"] {
                let network: NetworkPermissionConfig = serde_json::from_value(
                    json!({"default":default,"inbound":inbound,"hostLoopback":loopback}),
                )
                .unwrap();
                assert_eq!(
                    network.validate_for_platform("linux").is_ok(),
                    inbound == "deny" && loopback == "deny"
                );
                assert_eq!(
                    network.validate_for_platform("macos").is_ok(),
                    !(inbound == "deny" && loopback == "allow")
                );
                assert!(network.validate_for_platform("windows").is_ok());
            }
        }
    }
    for example in examples() {
        let network: NetworkPermissionConfig = serde_json::from_value(example.network).unwrap();
        assert!(
            network.validate_for_platform("linux").is_ok(),
            "{}",
            example.name
        );
        assert!(
            network.validate_for_platform("windows").is_ok(),
            "{}",
            example.name
        );
        let macos = network.validate_for_platform("macos");
        assert_eq!(macos.is_ok(), !example.selective, "{}", example.name);
        if let Err(error) = macos {
            assert!(error.contains("macOS Seatbelt"), "{error}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn legacy_workspace_network_replaces_parent_rules_and_empty_offline_can_restore_inheritance() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace_root = root.path().join("workspace");
    fs::create_dir_all(&workspace_root).unwrap();
    let workspace = project_config_path(&workspace_root);
    let parent =
        json!({"default":"allow","deny":[{"to":"192.0.2.99"}],"allow":[{"to":"198.51.100.5"}]});
    set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        update("permission.network", parent.clone()),
    )
    .unwrap();
    let parent_bytes = fs::read(&global).unwrap();
    let custom =
        json!({"default":"deny","allow":[{"to":"192.0.2.99","protocol":"tcp","ports":[443]}]});
    set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        update("permission.network", custom.clone()),
    )
    .unwrap();
    assert_eq!(
        load_network(&global, &workspace_root),
        serde_json::from_value::<NetworkPermissionConfig>(custom).unwrap()
    );
    let policy =
        NetworkPermissionPolicy::from_config(&load_network(&global, &workspace_root)).unwrap();
    assert!(matches!(
        policy.check_endpoint(
            "192.0.2.99".parse().unwrap(),
            NetworkProtocol::Tcp,
            Some(443)
        ),
        PermissionDecision::Allow
    ));
    assert!(!matches!(
        policy.check_endpoint(
            "198.51.100.5".parse().unwrap(),
            NetworkProtocol::Tcp,
            Some(443)
        ),
        PermissionDecision::Allow
    ));
    set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        update("permission.network", json!({})),
    )
    .unwrap();
    assert_eq!(
        load_network(&global, &workspace_root),
        NetworkPermissionConfig::offline()
    );
    delete_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        ConfigSettingsDeleteInput {
            path: "permission.network".into(),
            options: ConfigSettingsEditOptions {
                validate: true,
                reload: false,
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(
        load_network(&global, &workspace_root),
        serde_json::from_value::<NetworkPermissionConfig>(parent).unwrap()
    );
    assert_eq!(fs::read(&global).unwrap(), parent_bytes);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn actual_backend_limits_reject_before_mutating_either_layer() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace_root = root.path().join("workspace");
    fs::create_dir_all(&workspace_root).unwrap();
    let workspace = project_config_path(&workspace_root);
    let before = br#"{"permission":{"network":{"default":"deny"}}}"#;
    fs::write(&global, before).unwrap();
    fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    fs::write(&workspace, before).unwrap();
    let unsupported = if cfg!(target_os = "linux") {
        vec![
            json!({"inbound":"allow"}),
            json!({"hostLoopback":"allow"}),
            json!({"allow":[{"to":"::/0"}]}),
        ]
    } else {
        vec![
            json!({"hostLoopback":"allow"}),
            json!({"allow":[{"to":"192.0.2.10","protocol":"tcp","ports":[443]}]}),
            json!({"default":"allow","deny":[{"to":"192.0.2.10"}]}),
        ]
    };
    for layer in [ConfigSettingsLayer::Global, ConfigSettingsLayer::Workspace] {
        for network in &unsupported {
            let error = set_layered_file_setting(
                &global,
                &workspace,
                layer,
                update("permission.network", network.clone()),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("permission.network"), "{error}");
            assert_eq!(fs::read(&global).unwrap(), before);
            assert_eq!(fs::read(&workspace).unwrap(), before);
        }
    }
}
