use std::fs;

use agena_runtime_config::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsGetInput,
    ConfigSettingsLayer, ConfigSettingsPathInput, ConfigSettingsSetInput, RawConfig,
    delete_layered_file_setting, delete_runtime_file_setting, read_runtime_file_setting,
    read_runtime_layer_setting, set_layered_file_setting, set_runtime_file_setting,
};
use serde_json::{Value, json};

fn options() -> ConfigSettingsEditOptions {
    ConfigSettingsEditOptions {
        reload: false,
        validate: false,
        ..Default::default()
    }
}
fn get(path: &str) -> ConfigSettingsGetInput {
    ConfigSettingsGetInput {
        target: ConfigSettingsPathInput {
            path: Some(path.into()),
        },
        ..Default::default()
    }
}

#[test]
fn raw_reads_materialize_legacy_fields_without_changing_bytes_or_revisions() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.json");
    let original = "{\"permission\":{\"network\":{}}}";
    fs::write(&file, original).unwrap();
    let root_before = read_runtime_file_setting(&file, ConfigSettingsGetInput::default()).unwrap();
    for (field, expected) in [
        ("default", json!("deny")),
        ("inbound", json!("deny")),
        ("hostLoopback", json!("deny")),
        ("allow", json!([])),
        ("deny", json!([])),
    ] {
        let value =
            read_runtime_file_setting(&file, get(&format!("permission.network.{field}"))).unwrap();
        assert_eq!(value.value, expected);
        assert_eq!(value.has_override, Some(true));
        assert_eq!(value.revision, root_before.revision);
    }
    assert_eq!(root_before.value["permission"]["network"]["version"], 2);
    assert_eq!(fs::read_to_string(file).unwrap(), original);
}

#[test]
fn deleting_an_implicit_legacy_field_restores_only_that_field_in_both_editors() {
    for runtime in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let parent = json!({"sandbox":false,"permission":{"network":{"version":2,"default":"allow","deny":[{"to":"10.0.0.0/8"}]}}});
        fs::write(&global, parent.to_string()).unwrap();
        serde_json::from_value::<RawConfig>(parent.clone())
            .unwrap()
            .resolve()
            .unwrap();
        fs::write(&workspace, json!({"permission":{"network":{}}}).to_string()).unwrap();
        let before = read_runtime_layer_setting(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            get("permission.network.default"),
        )
        .unwrap();
        assert_eq!(before.value, "deny");
        assert_eq!(before.inherited_value, Some(json!("allow")));
        let input = ConfigSettingsDeleteInput {
            path: "permission.network.default".into(),
            options: options(),
        };
        if runtime {
            delete_runtime_file_setting(&workspace, input, None).unwrap();
        } else {
            delete_layered_file_setting(&global, &workspace, ConfigSettingsLayer::Workspace, input)
                .unwrap();
        }
        let saved: Value = serde_json::from_str(&fs::read_to_string(&workspace).unwrap()).unwrap();
        assert_eq!(saved["permission"]["network"]["version"], 2);
        assert!(saved["permission"]["network"].get("default").is_none());
        assert_eq!(saved["permission"]["network"]["inbound"], "deny");
        assert_eq!(saved["permission"]["network"]["hostLoopback"], "deny");
        assert_eq!(saved["permission"]["network"]["deny"], json!([]));
        let mut resolved: RawConfig = serde_json::from_value(parent).unwrap();
        resolved.merge_project_from_with_keys(
            serde_json::from_value(saved).unwrap(),
            Default::default(),
        );
        let network =
            serde_json::to_value(resolved.resolve().unwrap().permission.network.unwrap()).unwrap();
        assert_eq!(network["default"], "allow");
        assert_eq!(network["inbound"], "deny");
        assert_eq!(network["hostLoopback"], "deny");
        assert_eq!(network["deny"], json!([]));
    }
}

#[test]
fn first_network_leaf_write_creates_a_sparse_version_two_overlay() {
    for runtime in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        fs::write(&global, json!({"sandbox":false}).to_string()).unwrap();
        let input = ConfigSettingsSetInput {
            path: "permission.network.default".into(),
            value: json!("deny"),
            options: options(),
        };
        if runtime {
            set_runtime_file_setting(&workspace, input, None).unwrap();
        } else {
            set_layered_file_setting(&global, &workspace, ConfigSettingsLayer::Workspace, input)
                .unwrap();
        }
        let saved: Value = serde_json::from_str(&fs::read_to_string(&workspace).unwrap()).unwrap();
        assert_eq!(
            saved["permission"]["network"],
            json!({"version":2,"default":"deny"})
        );
    }
}

#[test]
fn raw_projection_upgrades_profile_and_access_control_policy_sources() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.json");
    let original = json!({
        "profiles":{"agent":{"permission":{"network":{}}}},
        "access_control":{"profiles":{"restricted":{"permission":{"network":{"inbound":"allow"}}}},"users":[{"overrides":{"network":{"allow":[]}}}]},
    }).to_string();
    fs::write(&file, &original).unwrap();
    for path in [
        "profiles.agent.permission.network",
        "access_control.profiles.restricted.permission.network",
        "access_control.users.0.overrides.network",
    ] {
        let value = read_runtime_file_setting(&file, get(path)).unwrap().value;
        assert_eq!(value["version"], 2);
        assert_eq!(value["default"], "deny");
        assert_eq!(value["allow"], json!([]));
        assert_eq!(value["deny"], json!([]));
        assert_eq!(value["hostLoopback"], "deny");
    }
    assert_eq!(fs::read_to_string(file).unwrap(), original);
}
