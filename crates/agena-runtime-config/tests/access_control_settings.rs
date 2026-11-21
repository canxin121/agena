use std::fs;

use agena_runtime_config::{
    ConfigEnvironment, ConfigLoader, ConfigSettingsEditOptions, ConfigSettingsLayer,
    ConfigSettingsSetInput, LoadConfigRequest, RawConfig, set_layered_file_setting,
    set_runtime_file_setting, validate_layered_config_text,
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

fn catalog() -> Value {
    json!({
        "profiles": {
            "reader": {
                "name": "Reader",
                "permission": {"path": {"workspace": "readonly"}},
                "grant": {"tools": ["agena.fs.read"]}
            }
        },
        "users": [{
            "platform": "agena.telegram", "tenant": "telegram", "account": "bot",
            "actor": "alice", "resource": "room", "profile_id": "reader"
        }],
        "conversations": [{
            "platform": "agena.telegram", "tenant": "telegram", "account": "bot",
            "resource": "room", "default_profile_id": "reader"
        }]
    })
}

fn input(value: Value) -> ConfigSettingsSetInput {
    ConfigSettingsSetInput {
        path: "access_control".into(),
        value,
        options: ConfigSettingsEditOptions {
            validate: false,
            ..Default::default()
        },
    }
}

#[test]
fn global_catalog_round_trips_and_supplies_the_factory_permission_profile() {
    let resolved = serde_json::from_value::<RawConfig>(json!({"access_control": catalog()}))
        .unwrap()
        .resolve()
        .unwrap();
    assert_eq!(resolved.access_control.profiles["reader"].revision, 1);
    assert_eq!(resolved.access_control.users[0].actor, "alice");
    let default = RawConfig::default().resolve().unwrap();
    let id = agena_domain::DEFAULT_PERMISSION_PROFILE_ID;
    assert_eq!(default.permission_profile.as_deref(), Some(id));
    assert_eq!(default.access_control.profiles.len(), 1);
    assert_eq!(
        default.access_control.profiles[id].permission,
        default.permission
    );
    assert!(default.access_control.profiles[id].complete);
    let round_trip = serde_json::to_value(&resolved.access_control).unwrap();
    let parsed: agena_domain::AccessControlConfig = serde_json::from_value(round_trip).unwrap();
    assert_eq!(parsed, resolved.access_control);
}

#[test]
fn workspace_cannot_replace_the_instance_identity_directory() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace_root = root.path().join("project");
    let workspace = workspace_root.join(".agena/agena.json");
    fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    fs::write(
        &global,
        serde_json::to_vec(&json!({"access_control": catalog()})).unwrap(),
    )
    .unwrap();
    fs::write(&workspace, "{}").unwrap();
    let before = fs::read(&workspace).unwrap();
    let error = set_layered_file_setting(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        input(catalog()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("instance-owned"), "{error}");
    assert_eq!(fs::read(&workspace).unwrap(), before);

    let malicious = serde_json::to_string(&json!({"access_control": catalog()})).unwrap();
    assert!(
        validate_layered_config_text(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            &malicious,
            &EmptyEnvironment,
        )
        .unwrap_err()
        .to_string()
        .contains("instance-owned")
    );
    fs::write(&workspace, malicious).unwrap();
    let error = ConfigLoader::new(EmptyEnvironment)
        .load(&LoadConfigRequest {
            config_path: Some(global),
            workspace_root: Some(workspace_root),
            ..Default::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("instance-owned"), "{error}");
}

#[test]
fn project_merge_never_overrides_global_assignments() {
    let mut global =
        serde_json::from_value::<RawConfig>(json!({"access_control": catalog()})).unwrap();
    let mut replacement = catalog();
    replacement["users"][0]["actor"] = json!("mallory");
    let project =
        serde_json::from_value::<RawConfig>(json!({"access_control": replacement})).unwrap();
    global.merge_project_from_with_keys(project, Default::default());
    assert_eq!(
        global.resolve().unwrap().access_control.users[0].actor,
        "alice"
    );
}

#[test]
fn settings_reject_invalid_catalog_and_nested_permissions_before_writing() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("global.json");
    set_runtime_file_setting(&file, input(catalog()), None).unwrap();
    let before = fs::read(&file).unwrap();
    let mut missing_profile = catalog();
    missing_profile["profiles"] = json!({});
    let mut invalid_permission = catalog();
    invalid_permission["profiles"]["reader"]["permission"] =
        json!({"path":{"workspace":{"read":"deny","write":"allow"}}});
    let mut invalid_plugin = catalog();
    invalid_plugin["profiles"]["reader"]["permission"]["plugins"] = json!({"shell": false});
    let mut duplicate_user = catalog();
    duplicate_user["users"] = json!([
        duplicate_user["users"][0].clone(),
        duplicate_user["users"][0].clone()
    ]);
    let mut invalid_member_profile = catalog();
    invalid_member_profile["users"][0]["profile_id"] = json!("missing");
    for invalid in [
        missing_profile,
        invalid_permission,
        invalid_plugin,
        duplicate_user,
        invalid_member_profile,
    ] {
        assert!(set_runtime_file_setting(&file, input(invalid), None).is_err());
        assert_eq!(fs::read(&file).unwrap(), before);
    }
}

#[test]
fn concurrent_catalog_edits_require_the_revision_read_by_the_editor() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("global.json");
    let first = set_runtime_file_setting(&file, input(catalog()), None).unwrap();
    let mut updated = catalog();
    updated["profiles"]["reader"]["name"] = json!("Updated reader");
    let mut update = input(updated);
    update.options.expected_revision = Some(first.after_revision.clone());
    let second = set_runtime_file_setting(&file, update, None).unwrap();
    assert_ne!(first.after_revision, second.after_revision);
    let before = fs::read(&file).unwrap();
    let mut stale = input(catalog());
    stale.options.expected_revision = Some(first.after_revision);
    let error = set_runtime_file_setting(&file, stale, None).unwrap_err();
    assert!(error.diagnostic().contains("revision changed"), "{error}");
    assert_eq!(fs::read(&file).unwrap(), before);
}
