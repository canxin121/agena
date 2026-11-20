use std::{fs, path::Path};

use agena_runtime_config::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsGetInput,
    ConfigSettingsLayer, ConfigSettingsPathInput, ConfigSettingsReadResponse,
    ConfigSettingsSetInput, RawConfig, delete_layered_file_setting, delete_runtime_file_setting,
    read_runtime_layer_setting, resolved_config_json_value, set_layered_file_setting,
    set_runtime_file_setting,
};
use serde_json::{Value, json};

fn options() -> ConfigSettingsEditOptions {
    ConfigSettingsEditOptions {
        reload: false,
        ..Default::default()
    }
}

fn read(global: &Path, workspace: &Path, path: &str) -> ConfigSettingsReadResponse {
    read_runtime_layer_setting(
        global,
        workspace,
        ConfigSettingsLayer::Global,
        ConfigSettingsGetInput {
            target: ConfigSettingsPathInput {
                path: Some(path.to_owned()),
            },
            ..Default::default()
        },
    )
    .unwrap()
}

fn put(runtime: bool, global: &Path, workspace: &Path, path: &str, value: Value) {
    let input = ConfigSettingsSetInput {
        path: path.to_owned(),
        value,
        options: options(),
    };
    if runtime {
        set_runtime_file_setting(global, input, None).unwrap();
    } else {
        set_layered_file_setting(global, workspace, ConfigSettingsLayer::Global, input).unwrap();
    }
}

fn reset(runtime: bool, global: &Path, workspace: &Path, path: &str) {
    let input = ConfigSettingsDeleteInput {
        path: path.to_owned(),
        options: options(),
    };
    if runtime {
        delete_runtime_file_setting(global, input, None).unwrap();
    } else {
        delete_layered_file_setting(global, workspace, ConfigSettingsLayer::Global, input).unwrap();
    }
}

fn stored(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn resolved(path: &Path) -> Value {
    resolved_config_json_value(
        &serde_json::from_value::<RawConfig>(stored(path))
            .unwrap()
            .resolve()
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn global_values_edit_directly_and_reset_to_defaults_without_materializing_them() {
    for runtime in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let initial = json!({
            "ui":{"locale":"zh-CN"},
            "session":{"auto_title":{"enabled":false,"cooldown_seconds":0}},
        });
        fs::write(&global, initial.to_string()).unwrap();
        let project = json!({
            "sandbox":false,
            "session":{"auto_title":{"enabled":true,"cooldown_seconds":17}},
        })
        .to_string();
        fs::write(&workspace, &project).unwrap();

        let missing = read(&global, &workspace, "sandbox");
        assert!(missing.value.is_null());
        assert_eq!(missing.has_override, Some(false));
        assert_eq!(missing.default_value, Some(json!(true)));
        assert_eq!(missing.layer_value, Some(json!(true)));
        assert_eq!(missing.inherited_value, None);
        assert_eq!(stored(&global), initial);
        assert_eq!(resolved(&global)["sandbox"], true);

        put(runtime, &global, &workspace, "sandbox", json!(false));
        assert_eq!(read(&global, &workspace, "sandbox").value, false);
        assert_eq!(resolved(&global)["sandbox"], false);
        assert!(stored(&global).get("permission").is_none());
        reset(runtime, &global, &workspace, "sandbox");
        assert_eq!(stored(&global), initial);
        assert_eq!(resolved(&global)["sandbox"], true);
        assert_eq!(
            read(&global, &workspace, "sandbox").has_override,
            Some(false)
        );

        let cooldown = "session.auto_title.cooldown_seconds";
        let before = read(&global, &workspace, cooldown);
        assert_eq!(before.value, 0);
        assert_eq!(before.has_override, Some(true));
        assert_eq!(before.default_value, Some(json!(300)));
        assert_eq!(before.layer_value, Some(json!(0)));
        reset(runtime, &global, &workspace, cooldown);
        assert!(
            stored(&global)["session"]["auto_title"]
                .get("cooldown_seconds")
                .is_none()
        );
        assert_eq!(stored(&global)["session"]["auto_title"]["enabled"], false);
        assert_eq!(
            resolved(&global)["session"]["auto_title"]["cooldown_seconds"],
            300
        );
        assert_eq!(fs::read_to_string(&workspace).unwrap(), project);
    }
}

#[test]
fn built_in_permission_values_and_rules_are_directly_editable_and_individually_restorable() {
    for runtime in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        fs::write(&global, json!({"ui":{"locale":"zh-CN"}}).to_string()).unwrap();
        let fetch = "permission.tools.names.\"agena.web.fetch\"";
        assert!(read(&global, &workspace, fetch).value.is_null());
        assert_eq!(
            read(&global, &workspace, fetch).default_value,
            Some(json!("allow"))
        );

        put(
            runtime,
            &global,
            &workspace,
            "permission.path.workspace",
            json!("readonly"),
        );
        put(runtime, &global, &workspace, fetch, json!("deny"));
        let permission = resolved(&global)["permission"].clone();
        assert_eq!(permission["tools"]["names"]["agena.web.fetch"], "deny");
        assert_eq!(
            permission["tools"]["names"]["agena.interaction.ask"],
            "allow"
        );
        assert_eq!(permission["path"]["workspace"], "readonly");
        assert_eq!(permission["path"]["rules"]["<state>"], "readwrite");
        assert!(stored(&global)["permission"].get("network").is_none());

        reset(runtime, &global, &workspace, fetch);
        assert_eq!(
            resolved(&global)["permission"]["tools"]["names"]["agena.web.fetch"],
            "allow"
        );
        assert_eq!(
            stored(&global)["permission"]["path"]["workspace"],
            "readonly"
        );
        reset(runtime, &global, &workspace, "permission");
        assert_eq!(stored(&global), json!({"ui":{"locale":"zh-CN"}}));
        assert_eq!(
            resolved(&global)["permission"]["path"]["workspace"],
            "readwrite"
        );
    }
}

#[test]
fn global_network_reset_is_per_field_and_group_reset_preserves_other_settings() {
    for runtime in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let network = json!({"version":2,"default":"deny","allow":[],"deny":[]});
        fs::write(
            &global,
            json!({"sandbox":false,"permission":{"network":network,"path":{"workspace":"readonly"}}}).to_string(),
        )
        .unwrap();
        let project = json!({"permission":{"network":{"version":2,"default":"deny"}}}).to_string();
        fs::write(&workspace, &project).unwrap();

        let path = "permission.network.default";
        reset(runtime, &global, &workspace, path);
        assert_eq!(
            stored(&global)["permission"]["network"],
            json!({"version":2,"allow":[],"deny":[]})
        );
        let after = read(&global, &workspace, path);
        assert_eq!(after.has_override, Some(false));
        assert_eq!(after.default_value, Some(json!("allow")));
        assert_eq!(after.layer_value, Some(json!("allow")));
        assert_eq!(
            resolved(&global)["permission"]["network"]["default"],
            "allow"
        );
        assert_eq!(
            read(&global, &workspace, "permission.network.allow").has_override,
            Some(true)
        );

        reset(runtime, &global, &workspace, "permission.network");
        assert_eq!(
            stored(&global),
            json!({"sandbox":false,"permission":{"path":{"workspace":"readonly"}}})
        );
        let effective = resolved(&global);
        assert_eq!(effective["permission"]["network"]["default"], "allow");
        assert_eq!(effective["permission"]["path"]["workspace"], "readonly");
        assert_eq!(effective["sandbox"], false);
        assert_eq!(fs::read_to_string(&workspace).unwrap(), project);
    }
}
