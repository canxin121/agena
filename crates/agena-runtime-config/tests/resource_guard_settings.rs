use std::{fs, path::Path};

use agena_runtime_config::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsGetInput, ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, RawConfig, delete_layered_file_setting,
    delete_runtime_file_setting, list_file_settings, list_runtime_file_settings,
    normalize_resource_guard_document, patch_layered_file_settings, patch_runtime_file_settings,
    read_file_setting, read_runtime_file_setting, set_layered_file_setting,
    set_runtime_file_setting,
};
use serde_json::{Value, json};

fn raw(value: Value) -> RawConfig {
    serde_json::from_value(value).unwrap()
}

fn read(runtime: bool, path: &Path, target: Option<&str>) -> ConfigSettingsReadResponse {
    let mut input = ConfigSettingsGetInput::default();
    input.target.path = target.map(str::to_owned);
    if runtime {
        read_runtime_file_setting(path, input).unwrap()
    } else {
        read_file_setting(path, input).unwrap()
    }
}

fn set(
    runtime: bool,
    path: &Path,
    input: ConfigSettingsSetInput,
) -> Result<ConfigSettingsEditResponse, String> {
    if runtime {
        set_runtime_file_setting(path, input, None).map_err(|error| error.diagnostic().to_owned())
    } else {
        set_layered_file_setting(
            path,
            path.parent().unwrap().join("workspace/.agena/agena.json"),
            ConfigSettingsLayer::Global,
            input,
        )
        .map_err(|error| error.to_string())
    }
}

#[test]
fn legacy_resource_checks_control_the_guard_without_combining_command_hints() {
    for checks in [false, true] {
        for hints in [false, true] {
            let config = raw(json!({"sandbox":{
                "enabled":false, "resource_checks":checks, "command_hints":hints
            }}));
            assert_eq!(config.resource_guard, Some(checks));
            let canonical = serde_json::to_value(&config).unwrap();
            assert_eq!(canonical["resource_guard"], checks);
            assert_eq!(canonical["sandbox"], json!({"enabled":false}));
            let resolved = config.resolve().unwrap();
            assert!(!resolved.sandbox.enabled);
            assert_eq!(resolved.resource_guard, checks);
        }
    }
}

#[test]
fn hints_only_layers_inherit_and_new_and_legacy_guard_overrides_follow_layer_order() {
    for hints in [false, true] {
        let overlay = raw(json!({"sandbox":{"command_hints":hints}}));
        assert!(overlay.is_empty());
        assert_eq!(overlay.resource_guard, None);
        assert!(overlay.clone().resolve().unwrap().resource_guard);
        let canonical = serde_json::to_value(&overlay).unwrap();
        assert!(canonical.get("resource_guard").is_none());
        for parent in [false, true] {
            let mut global = raw(json!({"resource_guard":parent}));
            global.merge_project_from_with_keys(overlay.clone(), Default::default());
            assert_eq!(global.resolve().unwrap().resource_guard, parent);
        }
    }
    for (parent, child, expected) in [
        (
            json!({"resource_guard":true}),
            json!({"sandbox":{"resource_checks":false}}),
            false,
        ),
        (
            json!({"sandbox":{"resource_checks":false}}),
            json!({"resource_guard":true}),
            true,
        ),
    ] {
        let mut config = raw(parent);
        config.merge_from(raw(child));
        assert_eq!(config.resolve().unwrap().resource_guard, expected);
    }
}

#[test]
fn matching_new_and_legacy_values_normalize_and_conflicts_are_rejected() {
    for value in [false, true] {
        let mut document = json!({"resource_guard":value,"sandbox":{
            "enabled":true,"resource_checks":value,"command_hints":!value
        }});
        normalize_resource_guard_document(&mut document).unwrap();
        assert_eq!(
            document,
            json!({"resource_guard":value,"sandbox":{"enabled":true}})
        );
        let error = serde_json::from_value::<RawConfig>(json!({
            "resource_guard":value,"sandbox":{"resource_checks":!value}
        }))
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("resource_guard") && error.contains("sandbox.resource_checks"),
            "{error}"
        );
    }
}

#[test]
fn invalid_guard_values_cannot_be_loaded_or_saved_without_validation() {
    for value in [Value::Null, json!("false"), json!(0), json!([]), json!({})] {
        let error = serde_json::from_value::<RawConfig>(json!({"resource_guard":value.clone()}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("resource_guard"), "{error}");
        for runtime in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("agena.json");
            let before = b"{}\n";
            fs::write(&path, before).unwrap();
            let result = set(
                runtime,
                &path,
                ConfigSettingsSetInput {
                    path: "resource_guard".into(),
                    value: value.clone(),
                    options: ConfigSettingsEditOptions {
                        validate: false,
                        ..Default::default()
                    },
                },
            );
            assert!(result.is_err(), "{value}");
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
}

#[test]
fn raw_file_and_scalar_projections_are_canonical_without_changing_the_revision() {
    for runtime in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agena.json");
        let before = r#"{
  "sandbox": {"enabled": false, "resource_checks": false, "command_hints": true},
  "ui": {"locale": "zh-CN"}
}
"#;
        fs::write(&path, before).unwrap();
        let document = read(runtime, &path, None);
        let canonical = json!({
            "sandbox":{"enabled":false},"resource_guard":false,"ui":{"locale":"zh-CN"}
        });
        assert_eq!(document.value, canonical);
        for (target, expected) in [
            ("", canonical),
            ("resource_guard", json!(false)),
            ("sandbox.enabled", json!(false)),
            ("sandbox", json!({"enabled":false})),
        ] {
            let response = read(runtime, &path, Some(target));
            assert_eq!(response.value, expected);
            assert_eq!(response.revision, document.revision);
        }
        let input = ConfigSettingsListInput {
            recursive: true,
            ..Default::default()
        };
        let listed = if runtime {
            list_runtime_file_settings(&path, input).unwrap()
        } else {
            list_file_settings(&path, input).unwrap()
        };
        assert_eq!(listed.revision, document.revision);
        let items = serde_json::to_string(&listed.items).unwrap();
        assert!(items.contains("resource_guard"));
        assert!(!items.contains("resource_checks") && !items.contains("command_hints"));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
}

#[test]
fn editing_a_legacy_file_preserves_cas_and_dry_run_and_writes_only_canonical_fields() {
    for runtime in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agena.json");
        let before =
            r#"{"sandbox":{"enabled":false,"resource_checks":false,"command_hints":false}}"#;
        fs::write(&path, before).unwrap();
        let revision = read(runtime, &path, None).revision;
        let input = ConfigSettingsSetInput {
            path: "resource_guard".into(),
            value: json!(true),
            options: ConfigSettingsEditOptions {
                expected_revision: revision,
                dry_run: true,
                validate: false,
                ..Default::default()
            },
        };
        let dry_run = set(runtime, &path, input.clone()).unwrap();
        assert!(dry_run.changed && dry_run.dry_run);
        assert!(!dry_run.created);
        assert_eq!(dry_run.previous, json!(false));
        assert_eq!(dry_run.current, json!(true));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        let mut input = input;
        input.options.dry_run = false;
        let saved = set(runtime, &path, input.clone()).unwrap();
        assert!(saved.changed);
        assert!(!saved.created);
        assert_eq!(saved.before_revision, dry_run.before_revision);
        assert_eq!(saved.after_revision, dry_run.after_revision);
        let canonical: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            canonical,
            json!({"sandbox":{"enabled":false},"resource_guard":true})
        );
        assert!(raw(canonical).resolve().unwrap().resource_guard);
        let saved_bytes = fs::read(&path).unwrap();
        assert!(
            set(runtime, &path, input).is_err(),
            "stale revision was accepted"
        );
        assert_eq!(fs::read(&path).unwrap(), saved_bytes);
    }
}

#[test]
fn conflicting_legacy_patch_does_not_replace_the_saved_document() {
    for runtime in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agena.json");
        let before = "{}\n";
        fs::write(&path, before).unwrap();
        let input = ConfigSettingsPatchInput {
            target: Default::default(),
            changes: json!({"resource_guard":true,"sandbox":{"resource_checks":false}}),
            options: ConfigSettingsEditOptions {
                validate: false,
                ..Default::default()
            },
        };
        let rejected = if runtime {
            patch_runtime_file_settings(&path, input, None).is_err()
        } else {
            patch_layered_file_settings(
                &path,
                directory.path().join("workspace/.agena/agena.json"),
                ConfigSettingsLayer::Global,
                input,
            )
            .is_err()
        };
        assert!(rejected);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
}

#[test]
fn deleting_the_guard_from_a_legacy_file_restores_inheritance() {
    for runtime in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agena.json");
        fs::write(
            &path,
            r#"{"sandbox":{"enabled":false,"resource_checks":false}}"#,
        )
        .unwrap();
        let input = ConfigSettingsDeleteInput {
            path: "resource_guard".into(),
            options: ConfigSettingsEditOptions {
                expected_revision: read(runtime, &path, None).revision,
                validate: false,
                ..Default::default()
            },
        };
        let response = if runtime {
            delete_runtime_file_setting(&path, input, None).unwrap()
        } else {
            delete_layered_file_setting(
                &path,
                directory.path().join("workspace/.agena/agena.json"),
                ConfigSettingsLayer::Global,
                input,
            )
            .unwrap()
        };
        assert!(response.deleted && response.changed);
        assert_eq!(response.previous, json!(false));
        assert!(response.current.is_null());
        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document, json!({"sandbox":{"enabled":false}}));
        assert!(raw(document).resolve().unwrap().resource_guard);
    }
}
