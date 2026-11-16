use std::collections::BTreeMap;

use agena_runtime_config::{ConfigEnvironment, RawConfig};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Environment(BTreeMap<String, String>);

impl Environment {
    fn new(values: &[(&str, &str)]) -> Self {
        Self(
            values
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        )
    }
}

impl ConfigEnvironment for Environment {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn vars(&self) -> Vec<(String, String)> {
        self.0
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

fn raw(value: Value) -> RawConfig {
    serde_json::from_value(value).unwrap()
}

#[test]
fn sandbox_defaults_preserve_isolation_and_direct_execution_checks() {
    let config = RawConfig::default().resolve().unwrap();
    assert!(config.sandbox);
    assert!(config.resource_guard);
    assert!(
        serde_json::to_value(RawConfig::default())
            .unwrap()
            .get("sandbox")
            .is_none()
    );
    // Previously accepted null values remain an absent override.
    let legacy_null = raw(json!({"sandbox":null}));
    assert!(legacy_null.sandbox.is_none());
    assert!(
        serde_json::to_value(&legacy_null)
            .unwrap()
            .get("sandbox")
            .is_none()
    );
    assert!(legacy_null.resolve().unwrap().sandbox);
    assert_eq!(
        serde_json::to_value(config).unwrap()["sandbox"],
        json!(true)
    );
}

#[test]
fn sandbox_booleans_round_trip_and_false_overrides_the_parent() {
    for enabled in [false, true] {
        let config = raw(json!({"sandbox":enabled}));
        assert_eq!(config.sandbox, Some(enabled));
        let serialized = serde_json::to_value(config).unwrap();
        assert_eq!(serialized["sandbox"], json!(enabled));
        let round_trip: RawConfig = serde_json::from_value(serialized).unwrap();
        assert_eq!(round_trip.resolve().unwrap().sandbox, enabled);
        let mut parent = raw(json!({"sandbox":!enabled}));
        parent.merge_project_from_with_keys(raw(json!({"sandbox":enabled})), Default::default());
        assert_eq!(parent.resolve().unwrap().sandbox, enabled);
    }
}

#[test]
fn legacy_sandbox_layers_without_enabled_preserve_inheritance() {
    for value in [
        Value::Null,
        json!({}),
        json!({"resource_checks":false}),
        json!({"command_hints":false}),
        json!({"resource_checks":true,"command_hints":false}),
    ] {
        let overlay = raw(json!({"sandbox":value}));
        assert!(overlay.sandbox.is_none());
        assert!(
            serde_json::to_value(&overlay)
                .unwrap()
                .get("sandbox")
                .is_none()
        );
        for inherited in [false, true] {
            let mut parent = raw(json!({"sandbox":inherited}));
            parent.merge_project_from_with_keys(overlay.clone(), Default::default());
            assert_eq!(parent.resolve().unwrap().sandbox, inherited);
        }
    }
}

#[test]
fn sandbox_and_resource_guard_inherit_independently_and_false_is_an_override() {
    let mut global = raw(json!({"sandbox":false, "resource_guard":false}));
    global.merge_project_from_with_keys(raw(json!({"sandbox":true})), Default::default());
    let resolved = global.resolve().unwrap();
    assert!(resolved.sandbox);
    assert!(!resolved.resource_guard);

    let mut global = raw(json!({"sandbox":false, "resource_guard":false}));
    global.merge_project_from_with_keys(raw(json!({"resource_guard":true})), Default::default());
    let resolved = global.resolve().unwrap();
    assert!(!resolved.sandbox);
    assert!(resolved.resource_guard);
}

#[test]
fn sandbox_settings_reject_unknown_fields_and_non_boolean_values() {
    for value in [
        json!({"enabled":"false"}),
        json!({"resource_checks":0}),
        json!({"command_hints":[]}),
        json!({"enabled":null}),
        json!({"resource_checks":null}),
        json!({"command_hints":null}),
        json!({"enable":true}),
        json!([]),
        json!([false, true, true]),
        json!(0),
        json!("disabled"),
    ] {
        assert!(serde_json::from_value::<RawConfig>(json!({"sandbox":value})).is_err());
    }
}

#[test]
fn environment_disable_is_visible_in_resolved_settings() {
    for variable in ["AGENA_SANDBOX", "AGENA_SHELL_SANDBOX"] {
        for value in ["disabled", "false", "0", "off", " Disabled "] {
            let mut config = raw(json!({"sandbox":true, "resource_guard":false}));
            config
                .merge_from(RawConfig::from_env(&Environment::new(&[(variable, value)])).unwrap());
            let resolved = config.resolve().unwrap();
            assert!(!resolved.sandbox);
            assert!(!resolved.resource_guard);
        }
    }
}

#[test]
fn environment_cannot_force_enable_and_primary_variable_wins_over_alias() {
    let env = Environment::new(&[
        ("AGENA_SANDBOX", "enabled"),
        ("AGENA_SHELL_SANDBOX", "disabled"),
    ]);
    let overlay = RawConfig::from_env(&env).unwrap();
    assert!(overlay.sandbox.is_none());
    let mut config = raw(json!({"sandbox":false}));
    config.merge_from(overlay);
    assert!(!config.resolve().unwrap().sandbox);
    assert!(
        RawConfig::from_env(&env)
            .unwrap()
            .resolve()
            .unwrap()
            .sandbox
    );
}

#[test]
fn raw_resolution_preserves_external_defaults_for_contextual_validation() {
    for mode in ["readonly", "readwrite"] {
        let mut value = json!({"permission":{"path":{"external":mode}}});
        let resolved = raw(value.clone()).resolve().unwrap();
        assert!(resolved.sandbox);
        assert_eq!(
            serde_json::to_value(resolved.permission).unwrap()["path"]["external"],
            mode
        );
        value["sandbox"] = json!(false);
        assert!(!raw(value).resolve().unwrap().sandbox);
    }
}

#[test]
fn layered_settings_support_writable_overrides_in_a_readonly_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let global = directory.path().join("global.json");
    let workspace_root = directory.path().join("workspace");
    std::fs::create_dir_all(workspace_root.join("output")).unwrap();
    let workspace = workspace_root.join(".agena/agena.json");
    std::fs::write(&global, "{}\n").unwrap();
    let mut edited = json!({"permission":{"path":{
        "workspace":"readonly","rules":{"<workspace>/output":"readwrite"}
    }}});
    let validate = |value: &Value| {
        agena_runtime_config::validate_layered_config_text(
            &global,
            &workspace,
            agena_runtime_config::ConfigSettingsLayer::Workspace,
            &value.to_string(),
            &Environment::default(),
        )
    };
    validate(&edited).unwrap();
    edited["sandbox"] = json!(false);
    validate(&edited).unwrap();
    assert_eq!(std::fs::read_to_string(&global).unwrap(), "{}\n");
    assert!(!workspace.exists());
}

#[test]
fn denied_workspace_with_concrete_read_and_write_roots_is_supported() {
    let directory = tempfile::tempdir().unwrap();
    let global = directory.path().join("global.json");
    let workspace = directory.path().join("workspace/.agena/agena.json");
    let workspace_root = workspace.parent().unwrap().parent().unwrap();
    std::fs::create_dir_all(workspace_root.join("input")).unwrap();
    std::fs::create_dir_all(workspace_root.join("output")).unwrap();
    std::fs::write(&global, "{}\n").unwrap();
    agena_runtime_config::validate_layered_config_text(
        &global,
        &workspace,
        agena_runtime_config::ConfigSettingsLayer::Workspace,
        &json!({"permission":{"path":{
            "workspace":"deny","external":"deny",
            "rules":{"<workspace>/input":"readonly","<workspace>/output":"readwrite"}
        }}})
        .to_string(),
        &Environment::default(),
    )
    .unwrap();
}

fn har_permission_request() -> Value {
    json!({
        "path": "permission",
        "value": {
            "path": {"workspace": "readwrite", "external": "readonly"},
            "approval_model": {
                "adapter": "openai_responses",
                "model": "gpt-6-luna",
                "provider": "local",
                "speed_mode": "fast",
                "thinking_mode": "medium"
            }
        },
        "dry_run": false,
        "validate": true,
        "reload": true
    })
}

#[test]
fn har_global_permission_request_is_saved_read_back_and_reloaded_in_each_mode() {
    use agena_runtime_config::{
        ConfigLoader, ConfigSettingsLayer, ConfigSettingsSetInput, LoadConfigRequest,
        config_error_to_settings_error, set_runtime_file_setting, validate_layered_config_text,
    };

    for (initial, environment, expected_sandbox) in [
        (json!({}), Environment::default(), true),
        (json!({"sandbox":false}), Environment::default(), false),
        (
            json!({"sandbox":true}),
            Environment::new(&[("AGENA_SANDBOX", "disabled")]),
            false,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let global = directory.path().join("global.json");
        let workspace_root = directory.path().join("workspace");
        std::fs::create_dir(&workspace_root).unwrap();
        let workspace = workspace_root.join(".agena/agena.json");
        std::fs::write(&global, initial.to_string()).unwrap();
        let request = har_permission_request();
        let update: ConfigSettingsSetInput = serde_json::from_value(request.clone()).unwrap();
        let validator_global = global.clone();
        let validator_workspace = workspace.clone();
        let validator_environment = environment.clone();
        let validator = move |_: &std::path::Path, text: &str| {
            validate_layered_config_text(
                &validator_global,
                &validator_workspace,
                ConfigSettingsLayer::Global,
                text,
                &validator_environment,
            )
            .map_err(config_error_to_settings_error)
        };
        let response = set_runtime_file_setting(&global, update, Some(&validator)).unwrap();
        assert!(response.changed && response.validated && response.reload_requested);
        let saved_text = std::fs::read_to_string(&global).unwrap();
        let saved: Value = serde_json::from_str(&saved_text).unwrap();
        assert_eq!(saved["permission"]["path"]["workspace"], "readwrite");
        assert_eq!(saved["permission"]["path"]["external"], "readonly");
        assert_eq!(
            saved["permission"]["approval_model"],
            request["value"]["approval_model"]
        );
        validate_layered_config_text(
            &global,
            &workspace,
            ConfigSettingsLayer::Global,
            &saved_text,
            &environment,
        )
        .unwrap();
        let load = LoadConfigRequest {
            config_path: Some(global),
            workspace_root: Some(workspace_root),
            ..Default::default()
        };
        let loader = ConfigLoader::new(environment);
        for resolution in [
            loader.load(&load).unwrap(),
            loader.load_global(&load).unwrap(),
        ] {
            assert_eq!(resolution.config.sandbox, expected_sandbox);
            let permission = serde_json::to_value(resolution.config.permission).unwrap();
            assert_eq!(permission["path"]["workspace"], "readwrite");
            assert_eq!(permission["path"]["external"], "readonly");
            assert_eq!(permission["path"]["rules"]["<state>"], "readwrite");
            assert_eq!(
                permission["approval_model"],
                request["value"]["approval_model"]
            );
        }
    }
}

#[test]
fn global_settings_keep_workspace_templates_and_default_state_inactive_without_an_owner() {
    use agena_runtime_config::{
        ConfigLoader, ConfigSettingsLayer, LoadConfigRequest, validate_layered_config_text,
    };

    let directory = tempfile::tempdir().unwrap();
    let global = directory.path().join("global.json");
    let mut document = json!({"permission": har_permission_request()["value"]});
    document["permission"]["path"]["rules"] = json!({"<workspace>/reference":"readonly"});
    let text = document.to_string();
    validate_layered_config_text(
        &global,
        std::path::Path::new(""),
        ConfigSettingsLayer::Global,
        &text,
        &Environment::default(),
    )
    .unwrap();
    agena_runtime_config::validate_config_text(&global, &text).unwrap();
    std::fs::write(&global, text).unwrap();
    let load = LoadConfigRequest {
        config_path: Some(global),
        ..Default::default()
    };
    let resolved = ConfigLoader::new(Environment::default())
        .load_global(&load)
        .unwrap();
    let rules = serde_json::to_value(resolved.config.permission).unwrap()["path"]["rules"].clone();
    assert_eq!(rules["<workspace>/reference"], "readonly");
    assert_eq!(rules["<state>"], "readwrite");
}

#[test]
fn global_inactive_templates_do_not_hide_invalid_active_host_paths() {
    let directory = tempfile::tempdir().unwrap();
    let global = directory.path().join("global.json");
    let document = json!({"permission":{"path":{"rules":{
        "<workspace>/reference":"readonly", "<tmp>":"readwrite"
    }}}});
    let error = agena_runtime_config::validate_layered_config_text(
        &global,
        std::path::Path::new(""),
        agena_runtime_config::ConfigSettingsLayer::Global,
        &document.to_string(),
        &Environment::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("shared OS temporary root"), "{error}");
    assert!(!global.exists());
}
