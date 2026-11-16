use std::fs;

use agena_runtime_config::{
    ConfigEnvironment, ConfigSettingsEditOptions, ConfigSettingsLayer, ConfigSettingsSetInput,
    RuntimeConfigSettingsError, set_layered_file_setting, set_runtime_file_setting,
    validate_layered_config_text,
};
use serde_json::{Value, json};

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

fn input(value: Value) -> ConfigSettingsSetInput {
    ConfigSettingsSetInput {
        path: "permission".into(),
        value,
        options: ConfigSettingsEditOptions {
            validate: false,
            ..Default::default()
        },
    }
}

#[test]
fn invalid_permission_never_replaces_the_file_even_when_validation_is_disabled() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    let before = b"{\n  \"permission\": {\"path\": {\"workspace\": \"readonly\"}}\n}\n";
    fs::write(&path, before).unwrap();
    for invalid in [
        json!({"path":{"workspace":{"read":"deny","write":"allow"}}}),
        json!({"path":{"workspace":"ask"}}),
        json!({"path":{"rules":{"<workspace>/**/*.key":"deny"}}}),
        json!({"path":{"rules":{"<workspace>/secret":"deny","<workspace>/secret/file":"readwrite"}}}),
        json!({"path":{"rules":{"<workspace>/data":"readwrite","<workspace>/data/reference":"readonly"}}}),
        json!({"network":{"rules":{"https://example.com/private":"allow"}}}),
        json!({"network":{"internet":"allow"}}),
        json!({"network":{"egress":{"default":"allow"}}}),
        json!({"network":{"default":"ask"}}),
        json!({"network":{"allow":[{"to":"example.com"}]}}),
        json!({"network":{"allow":[{"protocol":"tcp"}]}}),
        json!({"network":{"allow":[{"to":[]}]}}),
        json!({"network":{"allow":[{"to":"*","ports":[]}]}}),
        json!({"network":{"allow":[{"to":"*","protocol":"tcp","ports":[0]}]}}),
        json!({"network":{"allow":[{"to":"*","protocol":"tcp","ports":["443-80"]}]}}),
        json!({"network":{"allow":[{"to":"*","protocol":"icmp","ports":[53]}]}}),
        json!({"network":{"deny":[{"to":"192.0.2.0/24","except":["198.51.100.0/24"]}]}}),
        json!({"tools":{"rules":{"agena.fs.read":{"**/secret/**":"deny"}}}}),
        json!({"tools":{"names":{"shell.exec":"allow","agena.shell.exec":"deny"}}}),
    ] {
        let error = set_runtime_file_setting(&path, input(invalid), None).unwrap_err();
        assert!(error.diagnostic().contains("permission"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn dry_run_still_rejects_unsupported_permissions_without_creating_a_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    let mut update = input(json!({"path":{"external":"auto"}}));
    update.options.dry_run = true;
    let error = set_runtime_file_setting(&path, update, None).unwrap_err();
    assert!(error.diagnostic().contains("permission"));
    assert!(!path.exists());
}

#[test]
fn native_network_nulls_and_noncanonical_cidrs_never_replace_saved_settings() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace/.agena/agena.json");
    let before = br#"{"permission":{"network":{"default":"allow"}}}"#;
    fs::write(&global, before).unwrap();
    fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    fs::write(&workspace, before).unwrap();

    for (network, field) in [
        (json!(null), "permission.network"),
        (json!({"default":null}), "permission.network.default"),
        (json!({"allow":null}), "permission.network.allow"),
        (
            json!({"allow":[{"to":null}]}),
            "permission.network.allow[0].to",
        ),
        (
            json!({"allow":[{"to":"*","ports":[null]}]}),
            "permission.network.allow[0].ports[0]",
        ),
        (
            json!({"allow":[{"to":"192.0.2.7/24"}]}),
            "permission.network.allow[0].to",
        ),
        (
            json!({"deny":[{"to":"2001:db8::/32","except":["2001:db8::1/64"]}]}),
            "permission.network.deny[0].except[0]",
        ),
    ] {
        for path in ["permission", "permission.network"] {
            let mut update = input(if path == "permission" {
                json!({"network":network.clone()})
            } else {
                network.clone()
            });
            update.path = path.into();
            let error = set_runtime_file_setting(&global, update.clone(), None).unwrap_err();
            assert!(error.diagnostic().contains(field), "{error}");
            assert_eq!(fs::read(&global).unwrap(), before);

            let error = set_layered_file_setting(
                &global,
                &workspace,
                ConfigSettingsLayer::Workspace,
                update,
            )
            .unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
            assert_eq!(fs::read(&workspace).unwrap(), before);
            assert_eq!(fs::read(&global).unwrap(), before);
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn explicit_empty_network_is_saved_and_replaces_the_complete_parent_policy() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace/.agena/agena.json");
    set_runtime_file_setting(
        &global,
        input(json!({"network":{
            "default":"allow",
            "allow":[{"to":"192.0.2.0/24"}],
            "deny":[{"to":"192.0.2.128/25"}]
        }})),
        None,
    )
    .unwrap();
    set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        input(json!({"network":{}})),
    )
    .unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&workspace).unwrap()).unwrap();
    assert_eq!(saved["permission"]["network"], json!({"default":"deny"}));
    let global_value: Value = serde_json::from_slice(&fs::read(&global).unwrap()).unwrap();
    let mut merged: agena_domain::PermissionConfig =
        serde_json::from_value(global_value["permission"].clone()).unwrap();
    merged.merge_from(serde_json::from_value(saved["permission"].clone()).unwrap());
    let egress = merged.network.unwrap().egress.unwrap();
    assert_eq!(egress.default, Some(agena_domain::NetworkAction::Deny));
    assert_eq!(egress.allow, Some(vec![]));
    assert_eq!(egress.deny, Some(vec![]));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn network_editor_reads_the_complete_parent_before_the_selected_layer() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    let parent = json!({"default":"allow","deny":[{"to":"192.0.2.0/24"}]});
    let selected =
        json!({"default":"deny","allow":[{"to":"198.51.100.10","protocol":"tcp","ports":[443]}]});
    fs::write(
        &global,
        json!({"permission":{"network":parent}}).to_string(),
    )
    .unwrap();
    fs::write(
        &workspace,
        json!({"permission":{"network":selected}}).to_string(),
    )
    .unwrap();
    let mut query = agena_runtime_config::ConfigSettingsGetInput::default();
    query.target.path = Some("permission".into());
    let response = agena_runtime_config::read_runtime_layer_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        query.clone(),
    )
    .unwrap();
    assert_eq!(response.value["network"], selected);
    assert_eq!(response.inherited_value.unwrap()["network"], parent);

    query.target.path = Some("permission.network".into());
    let response = agena_runtime_config::read_runtime_layer_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        query.clone(),
    )
    .unwrap();
    assert_eq!(response.inherited_value, Some(parent.clone()));
    assert_eq!(response.value, selected);
    let response = agena_runtime_config::read_runtime_layer_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        query,
    )
    .unwrap();
    assert_eq!(response.value, parent);
    assert_eq!(response.inherited_value, Some(json!({"default":"deny"})));
}

#[test]
fn missing_network_layer_exposes_defaults_without_creating_files() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    let mut query = agena_runtime_config::ConfigSettingsGetInput::default();
    query.target.path = Some("permission.network".into());
    let response = agena_runtime_config::read_runtime_layer_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        query,
    )
    .unwrap();
    assert!(!response.config_found);
    assert!(response.value.is_null());
    assert_eq!(response.inherited_value, Some(json!({"default":"deny"})));
    assert!(!global.exists());
    assert!(!workspace.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn unsupported_native_backend_policy_cannot_be_saved() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    let before = b"{}\n";
    fs::write(&path, before).unwrap();
    let invalid = if cfg!(target_os = "linux") {
        json!({"network":{"inbound":"allow"}})
    } else {
        json!({"network":{"allow":[{"to":"192.0.2.1"}]}})
    };
    let error = set_runtime_file_setting(&path, input(invalid), None).unwrap_err();
    assert!(error.diagnostic().contains("permission.network"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn safe_legacy_pairs_are_saved_as_canonical_direct_modes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    let response = set_runtime_file_setting(
        &path,
        input(json!({"path":{
            "workspace":{"read":"deny","write":"deny"},
            "rules":{
                "<workspace>/input":{"read":"allow","write":"deny"},
                "<workspace>/input/**":{"read":"allow","write":"deny"},
                "./output":"readwrite"
            }
        }})),
        None,
    )
    .unwrap();
    assert!(response.changed);
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved["permission"]["path"],
        json!({
            "workspace":"deny",
            "rules":{"<workspace>/input":"readonly","<workspace>/output":"readwrite"}
        })
    );
}

#[test]
fn ambiguous_legacy_literal_allow_does_not_silently_become_recursive() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    let error = set_runtime_file_setting(
        &path,
        input(json!({"path":{"rules":{
            "<workspace>/input":{"read":"allow","write":"deny"}
        }}})),
        None,
    )
    .unwrap_err();
    assert!(error.diagnostic().contains("permission"));
    assert!(!path.exists());
}

#[test]
fn composed_validator_cannot_be_bypassed_by_validate_false() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agena.json");
    fs::write(&path, "{}\n").unwrap();
    let validator = |_: &std::path::Path, _: &str| {
        Err(RuntimeConfigSettingsError::invalid_input(
            "permission.path.rules conflicts with inherited policy",
        ))
    };
    let error = set_runtime_file_setting(
        &path,
        input(json!({"path":{"workspace":"readonly"}})),
        Some(&validator),
    )
    .unwrap_err();
    assert!(error.diagnostic().contains("inherited policy"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{}\n");
}

#[test]
fn composed_layers_reject_a_child_allow_under_an_inherited_deny() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace/.agena/agena.json");
    fs::write(
        &global,
        r#"{"permission":{"path":{"rules":{"<workspace>/secret":"deny"}}}}"#,
    )
    .unwrap();
    let error = validate_layered_config_text(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        r#"{"permission":{"path":{"rules":{"<workspace>/secret/file":"readwrite"}}}}"#,
        &EmptyEnvironment,
    )
    .unwrap_err();
    assert!(error.to_string().contains("secret"), "{error}");
}

#[cfg(unix)]
#[test]
fn composed_validation_rejects_conflicts_after_symbolic_link_resolution() {
    let root = tempfile::tempdir().unwrap();
    let workspace_root = root.path().join("workspace");
    fs::create_dir_all(workspace_root.join("data")).unwrap();
    std::os::unix::fs::symlink(workspace_root.join("data"), workspace_root.join("shortcut"))
        .unwrap();
    let global = root.path().join("global.json");
    let workspace = workspace_root.join(".agena/agena.json");
    let document = json!({"permission":{"path":{"rules":{
        "<workspace>/data":"deny",
        "<workspace>/shortcut/file":"readwrite"
    }}}});
    let error = validate_layered_config_text(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        &document.to_string(),
        &EmptyEnvironment,
    )
    .unwrap_err();
    assert!(error.to_string().contains("conflict"), "{error}");
}

#[test]
fn a_temporary_workspace_can_have_separate_readonly_inputs_and_writable_outputs() {
    let root = tempfile::tempdir().unwrap();
    validate_layered_config_text(
        &root.path().join("global.json"),
        &root.path().join("workspace/.agena/agena.json"),
        ConfigSettingsLayer::Workspace,
        r#"{"permission":{"path":{"workspace":"deny","external":"deny","rules":{"<workspace>/input":"readonly","<workspace>/output":"readwrite"}}}}"#,
        &EmptyEnvironment,
    ).unwrap();
}

#[test]
fn shared_and_other_tasks_private_temporary_grants_are_rejected_before_save() {
    let root = tempfile::tempdir().unwrap();
    for pattern in ["<tmp>", "<tmp>/agena-sandbox-another-task/result"] {
        let error = validate_layered_config_text(
            &root.path().join("global.json"),
            &root.path().join("workspace/.agena/agena.json"),
            ConfigSettingsLayer::Workspace,
            &json!({"permission":{"path":{"rules":{pattern:"readwrite"}}}}).to_string(),
            &EmptyEnvironment,
        )
        .unwrap_err();
        assert!(error.to_string().contains("temporary"), "{error}");
    }
}

#[test]
fn cli_layered_save_enforces_permissions_even_when_validate_is_false() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace/.agena/agena.json");
    fs::write(&global, "{}\n").unwrap();
    let error = agena_runtime_config::set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        input(json!({"path":{"rules":{"<workspace>/**/*.key":"deny"}}})),
    )
    .unwrap_err();
    assert!(error.to_string().contains("permission"), "{error}");
    assert_eq!(fs::read_to_string(&global).unwrap(), "{}\n");
}
