use std::fs;

use agena_runtime_config::{
    ConfigEnvironment, ConfigSettingsEditOptions, ConfigSettingsLayer, ConfigSettingsSetInput,
    RuntimeConfigSettingsError, set_runtime_file_setting, validate_layered_config_text,
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
