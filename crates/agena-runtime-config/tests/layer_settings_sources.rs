use agena_runtime_config::{
    ConfigSettingsGetInput, ConfigSettingsLayer, ConfigSettingsReadResponse,
    read_runtime_layer_setting,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn read(
    global: &Path,
    workspace: &Path,
    layer: ConfigSettingsLayer,
    path: Option<&str>,
) -> ConfigSettingsReadResponse {
    let mut input = ConfigSettingsGetInput::default();
    input.target.path = path.map(str::to_owned);
    read_runtime_layer_setting(global, workspace, layer, input).unwrap()
}

#[test]
fn scalar_sources_describe_the_edited_layer_before_later_overrides() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(
        &global,
        json!({"sandbox":false,"ui":{"locale":"zh-CN"}}).to_string(),
    )
    .unwrap();
    fs::write(
        &workspace,
        json!({"sandbox":true,"ui":{"locale":"en-US"}}).to_string(),
    )
    .unwrap();

    let baseline = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        Some("sandbox"),
    );
    assert_eq!(baseline.value, false);
    assert_eq!(baseline.has_override, Some(true));
    assert_eq!(baseline.default_value, Some(json!(true)));
    assert_eq!(baseline.layer_value, Some(json!(false)));
    assert_eq!(baseline.inherited_value, None);
    let project = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("sandbox"),
    );
    assert_eq!(project.value, true);
    assert_eq!(project.inherited_value, Some(json!(false)));
    assert_eq!(project.default_value, Some(json!(true)));
    assert_eq!(project.layer_value, Some(json!(true)));
    let locale = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("ui.locale"),
    );
    assert_eq!(locale.value, "en-US");
    assert_eq!(locale.inherited_value, Some(json!("zh-CN")));
}

#[test]
fn root_parent_preserves_false_zero_empty_and_quoted_preference_keys() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    let ui = json!({"tui":{"show_status":false,"width":0,"theme":"","tool_visibility":{"agena.fs.read":false}}});
    fs::write(&global, json!({"ui":ui,"resource_guard":false}).to_string()).unwrap();
    fs::write(
        &workspace,
        json!({"ui":{"tui":{"show_status":true,"width":50,"theme":"dark"}}}).to_string(),
    )
    .unwrap();

    let selected = read(&global, &workspace, ConfigSettingsLayer::Workspace, None);
    assert_eq!(selected.value["ui"]["tui"]["width"], 50);
    let parent = selected.inherited_value.unwrap();
    assert_eq!(parent["ui"], ui);
    assert_eq!(parent["resource_guard"], false);
    assert_eq!(parent["sandbox"], true);
    let quoted = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("ui.tui.tool_visibility.\"agena.fs.read\""),
    );
    assert!(quoted.value.is_null());
    assert_eq!(quoted.inherited_value, Some(json!(false)));
}

#[test]
fn resolved_layer_fills_partial_groups_without_materializing_sparse_files() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    let global_file = json!({"session":{"auto_title":{"cooldown_seconds":0}}}).to_string();
    let workspace_file = json!({"session":{"auto_title":{"enabled":false}}}).to_string();
    fs::write(&global, &global_file).unwrap();
    fs::write(&workspace, &workspace_file).unwrap();

    let baseline = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        Some("session.auto_title"),
    );
    assert_eq!(baseline.value, json!({"cooldown_seconds":0}));
    assert_eq!(baseline.inherited_value, None);
    let defaults = baseline.default_value.as_ref().unwrap();
    assert_eq!(defaults["cooldown_seconds"], 300);
    let global_resolved = baseline.layer_value.unwrap();
    assert_eq!(global_resolved["cooldown_seconds"], 0);
    assert_eq!(global_resolved["enabled"], defaults["enabled"]);
    assert!(global_resolved["message_interval"].as_u64().unwrap() > 0);

    let selected = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("session.auto_title"),
    );
    assert_eq!(selected.value, json!({"enabled":false}));
    assert_eq!(selected.default_value, baseline.default_value);
    assert_eq!(selected.inherited_value, Some(global_resolved));
    let workspace_resolved = selected.layer_value.unwrap();
    assert_eq!(workspace_resolved["enabled"], false);
    assert_eq!(workspace_resolved["cooldown_seconds"], 0);
    assert!(workspace_resolved["message_interval"].as_u64().unwrap() > 0);
    assert_eq!(fs::read_to_string(&global).unwrap(), global_file);
    assert_eq!(fs::read_to_string(&workspace).unwrap(), workspace_file);
}

#[test]
fn resolved_layer_uses_atomic_rule_groups_and_keeps_explicit_preference_clears() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(
        &global,
        json!({
            "ui":{"tui":{"theme":"dark","width":40}},
            "permission":{
                "network":{"version":2,"default":"deny","inbound":"deny"},
                "tools":{"rules":{"agena.shell.exec":{"routine":"ask"}}}
            }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        &workspace,
        json!({
            "ui":{"tui":{"theme":null}},
            "permission":{
                "network":{"version":2,"default":"allow"},
                "tools":{"rules":{"agena.shell.exec":{"dangerous":"ask"}}}
            }
        })
        .to_string(),
    )
    .unwrap();

    let dangerous = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        Some("permission.tools.rules.\"shell.*\".dangerous"),
    );
    assert_eq!(dangerous.default_value, Some(json!("deny")));
    assert_eq!(dangerous.layer_value, Some(Value::Null));
    assert_eq!(dangerous.has_override, Some(true));
    assert_eq!(
        dangerous.override_path.as_deref(),
        Some("permission.tools.rules.\"shell.*\"")
    );
    assert_eq!(dangerous.requires_group_reset, Some(false));

    let selected = read(&global, &workspace, ConfigSettingsLayer::Workspace, None);
    let value = selected.layer_value.unwrap();
    assert_eq!(value["permission"]["network"]["default"], "allow");
    assert_eq!(value["permission"]["network"]["inbound"], "deny");
    assert_eq!(
        value["permission"]["tools"]["rules"]["shell.*"],
        json!({"dangerous":"ask"})
    );
    assert_eq!(value["ui"]["tui"]["width"], 40);
    assert_eq!(value["ui"]["tui"]["theme"], Value::Null);

    let theme = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("ui.tui.theme"),
    );
    assert_eq!(theme.has_override, Some(true));
    assert_eq!(theme.default_value, Some(Value::Null));
    assert_eq!(theme.inherited_value, Some(json!("dark")));
    assert_eq!(theme.layer_value, Some(Value::Null));
    let wire = serde_json::to_value(theme).unwrap();
    assert!(wire.get("layer_value").is_some_and(Value::is_null));
}

#[test]
fn absent_files_expose_defaults_without_creating_configuration() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    for layer in [ConfigSettingsLayer::Global, ConfigSettingsLayer::Workspace] {
        let source = read(&global, &workspace, layer, Some("resource_guard"));
        assert!(!source.config_found);
        assert_eq!(source.has_override, Some(false));
        assert!(source.value.is_null());
        assert_eq!(source.default_value, Some(json!(true)));
        assert_eq!(source.layer_value, Some(json!(true)));
        assert_eq!(
            source.inherited_value,
            (layer == ConfigSettingsLayer::Workspace).then(|| json!(true))
        );
        let opaque = read(&global, &workspace, layer, Some("ui.tui.theme"));
        assert_eq!(opaque.default_value, Some(Value::Null));
        assert_eq!(opaque.layer_value, Some(Value::Null));
        assert_eq!(
            opaque.inherited_value,
            (layer == ConfigSettingsLayer::Workspace).then_some(Value::Null)
        );
    }
    assert!(!global.exists());
    assert!(!workspace.exists());
}

#[test]
fn resolved_model_selection_is_atomic_instead_of_merging_parent_modes() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(&global, json!({"providers":{
        "default_selection":{"provider":"local","model":"main","thinking_mode":"high","parallel_tool_calls":false},
        "local":{
            "auth":{"mode":"api","subtype":"custom","base_url":"http://localhost:1234","api_key":{"kind":"env","value":"AGENA_LAYER_TEST_API_KEY"}},
            "adapters":{"openai_chat_completions":{"enabled":true}}
        }
    }}).to_string()).unwrap();
    fs::write(
        &workspace,
        json!({"providers":{
            "default_selection":{"provider":"local","model":"other"}
        }})
        .to_string(),
    )
    .unwrap();

    let model = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("providers.default_selection"),
    );
    assert_eq!(model.default_value, Some(Value::Null));
    assert_eq!(
        model.inherited_value.as_ref().unwrap()["thinking_mode"],
        "high"
    );
    assert_eq!(
        model.inherited_value.as_ref().unwrap()["parallel_tool_calls"],
        false
    );
    let current = model.layer_value.unwrap();
    assert_eq!(current["model"], "other");
    assert!(current.get("thinking_mode").is_none());
    assert!(current.get("parallel_tool_calls").is_none());
    let mode = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("providers.default_selection.thinking_mode"),
    );
    assert_eq!(mode.inherited_value, Some(json!("high")));
    assert_eq!(mode.layer_value, Some(Value::Null));
    assert_eq!(mode.has_override, Some(true));
    assert_eq!(
        mode.override_path.as_deref(),
        Some("providers.default_selection")
    );
    assert_eq!(mode.requires_group_reset, Some(false));
    let global_mode = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        Some("providers.default_selection.speed_mode"),
    );
    assert_eq!(global_mode.has_override, Some(false));
    assert_eq!(global_mode.override_path, None);
    assert_eq!(global_mode.layer_value, Some(Value::Null));
}

#[test]
fn invalid_parent_keeps_the_selected_raw_layer_available_for_repair() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(
        &global,
        json!({"session":{"auto_title":{"message_interval":0}}}).to_string(),
    )
    .unwrap();
    fs::write(&workspace, json!({"sandbox":false}).to_string()).unwrap();
    let selected = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("sandbox"),
    );
    assert!(selected.config_found);
    assert_eq!(selected.value, false);
    assert_eq!(selected.inherited_value, None);
    assert_eq!(selected.default_value, Some(json!(true)));
    assert_eq!(selected.layer_value, None);
    let broken = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Global,
        Some("session.auto_title.message_interval"),
    );
    assert_eq!(broken.value, 0);
    assert_eq!(broken.inherited_value, None);
    assert!(broken.default_value.unwrap().as_u64().unwrap() > 0);
    assert_eq!(broken.layer_value, None);
}

#[test]
fn inherited_permission_uses_domain_group_replacement() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(
        &global,
        json!({"permission":{"network":{},"tools":{"rules":{"*":{}}}}}).to_string(),
    )
    .unwrap();
    fs::write(
        &workspace,
        json!({"permission":{"network":{"default":"allow"}}}).to_string(),
    )
    .unwrap();
    let selected = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("permission"),
    );
    assert_eq!(
        selected.value["network"],
        json!({"version":2,"default":"allow","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
    );
    let parent = selected.inherited_value.unwrap();
    assert_eq!(
        parent["network"],
        json!({"version":2,"default":"deny","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
    );
    assert_eq!(parent["tools"]["rules"]["*"], json!({}));
    assert_eq!(parent["path"]["workspace"], "readwrite");
}

#[test]
fn opaque_null_and_optional_null_keep_their_distinct_layer_semantics() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    let parent = json!({
        "sandbox":false,
        "session":{"compaction":{"reserved_tokens":80}},
        "ui":{"locale":"zh-CN","tui":{"theme":"dark","width":40}},
    });
    let local = json!({
        "sandbox":null,
        "session":{"compaction":{"reserved_tokens":null}},
        "ui":{"locale":null,"tui":{"theme":null}},
    });
    fs::write(&global, parent.to_string()).unwrap();
    fs::write(&workspace, local.to_string()).unwrap();

    for (path, inherited) in [
        ("sandbox", json!(false)),
        ("session.compaction.reserved_tokens", json!(80)),
        ("ui.locale", json!("zh-CN")),
        ("ui.tui.width", json!(40)),
    ] {
        let response = read(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            Some(path),
        );
        assert_eq!(response.value, Value::Null, "{path}");
        assert_eq!(response.has_override, Some(false), "{path}");
        assert_eq!(response.override_path, None, "{path}");
        assert_eq!(response.inherited_value, Some(inherited), "{path}");
    }
    let cleared = read(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        Some("ui.tui.theme"),
    );
    assert_eq!(cleared.value, Value::Null);
    assert_eq!(cleared.has_override, Some(true));
    assert_eq!(cleared.override_path.as_deref(), Some("ui.tui.theme"));
    assert_eq!(cleared.inherited_value, Some(json!("dark")));
    let wire = serde_json::to_value(cleared).unwrap();
    assert_eq!(wire["has_override"], true);
    assert!(wire["value"].is_null());

    // The source metadata follows the existing merge; it does not change it.
    let mut raw: agena_runtime_config::RawConfig = serde_json::from_value(parent).unwrap();
    raw.merge_project_from_with_keys(serde_json::from_value(local).unwrap(), Default::default());
    let effective =
        agena_runtime_config::resolved_config_json_value(&raw.resolve().unwrap()).unwrap();
    assert_eq!(effective["sandbox"], false);
    assert_eq!(effective["session"]["compaction"]["reserved_tokens"], 80);
    assert_eq!(effective["ui"]["locale"], "zh-CN");
    assert!(effective["ui"]["tui"]["theme"].is_null());
    assert_eq!(effective["ui"]["tui"]["width"], 40);
}

#[test]
fn both_file_readers_preserve_null_presence_and_original_file_revisions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.json");
    let original = json!({"sandbox":null,"ui":{"tui":{"theme":null,"width":0,"show_status":false,"label":""}}}).to_string();
    fs::write(&path, &original).unwrap();
    for field in [
        "ui.tui.theme",
        "ui.tui.width",
        "ui.tui.show_status",
        "ui.tui.label",
        "sandbox",
        "ui.tui.absent",
    ] {
        let input = ConfigSettingsGetInput {
            target: agena_runtime_config::ConfigSettingsPathInput {
                path: Some(field.to_owned()),
            },
            ..Default::default()
        };
        let runtime =
            agena_runtime_config::read_runtime_file_setting(&path, input.clone()).unwrap();
        let legacy = agena_runtime_config::read_file_setting(&path, input).unwrap();
        let expected = !matches!(field, "sandbox" | "ui.tui.absent");
        assert_eq!(runtime.has_override, Some(expected), "{field}");
        assert_eq!(legacy.has_override, runtime.has_override, "{field}");
        assert_eq!(runtime.override_path.as_deref(), expected.then_some(field));
        assert_eq!(legacy.override_path, runtime.override_path, "{field}");
        assert_eq!(legacy.value, runtime.value, "{field}");
        assert_eq!(legacy.revision, runtime.revision);
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn inherited_subtree_values_are_restored_by_clearing_the_reported_override_boundary() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace = root.path().join("workspace.json");
    fs::write(
        &global,
        json!({"ui":{"tui":{"theme":"dark","width":40}}}).to_string(),
    )
    .unwrap();
    for replacement in [Value::Null, json!(false), json!([])] {
        fs::write(
            &workspace,
            json!({"ui":{"tui":replacement,"web":{"theme":"light"}}}).to_string(),
        )
        .unwrap();
        let before = read(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            Some("ui.tui.theme"),
        );
        assert_eq!(before.has_override, Some(true));
        assert_eq!(before.override_path.as_deref(), Some("ui.tui"));
        assert!(before.value.is_null());
        assert_eq!(before.inherited_value, Some(json!("dark")));
        agena_runtime_config::delete_runtime_file_setting(
            &workspace,
            agena_runtime_config::ConfigSettingsDeleteInput {
                path: before.override_path.unwrap(),
                options: agena_runtime_config::ConfigSettingsEditOptions {
                    reload: false,
                    ..Default::default()
                },
            },
            None,
        )
        .unwrap();
        let after = read(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            Some("ui.tui.theme"),
        );
        assert_eq!(after.has_override, Some(false));
        assert_eq!(after.override_path, None);
        assert_eq!(after.inherited_value, Some(json!("dark")));
        let document: Value =
            serde_json::from_str(&fs::read_to_string(&workspace).unwrap()).unwrap();
        assert_eq!(document["ui"]["web"]["theme"], "light");
        assert!(!document["ui"].as_object().unwrap().contains_key("tui"));
    }
}
