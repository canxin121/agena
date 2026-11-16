use std::collections::BTreeMap;

use agena_runtime_config::{ConfigEnvironment, RawConfig, SandboxConfig};
use serde_json::{Value, json};

#[derive(Default)]
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
    assert_eq!(config.sandbox, SandboxConfig::default());
    // Like other optional top-level sections, null means no layer override.
    assert_eq!(
        raw(json!({"sandbox":null})).resolve().unwrap().sandbox,
        SandboxConfig::default()
    );
    let partial = raw(json!({"sandbox":{"enabled":false}}));
    let round_trip =
        serde_json::from_value::<RawConfig>(serde_json::to_value(partial).unwrap()).unwrap();
    assert!(!round_trip.resolve().unwrap().sandbox.enabled);
    assert_eq!(
        serde_json::to_value(config).unwrap()["sandbox"],
        json!({"enabled":true,"resource_checks":true,"command_hints":true})
    );
}

#[test]
fn sandbox_fields_inherit_independently_and_false_is_an_override() {
    let mut global = raw(json!({"sandbox":{
        "enabled":false,"resource_checks":false,"command_hints":true
    }}));
    global.merge_project_from_with_keys(
        raw(json!({"sandbox":{"enabled":true,"command_hints":false}})),
        Default::default(),
    );
    assert_eq!(
        global.resolve().unwrap().sandbox,
        SandboxConfig {
            enabled: true,
            resource_checks: false,
            command_hints: false,
        }
    );
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
        json!(false),
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
            let mut config = raw(json!({"sandbox":{"enabled":true,"command_hints":false}}));
            config
                .merge_from(RawConfig::from_env(&Environment::new(&[(variable, value)])).unwrap());
            assert_eq!(
                config.resolve().unwrap().sandbox,
                SandboxConfig {
                    enabled: false,
                    resource_checks: true,
                    command_hints: false,
                }
            );
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
    let mut config = raw(json!({"sandbox":{"enabled":false}}));
    config.merge_from(overlay);
    assert!(!config.resolve().unwrap().sandbox.enabled);
    assert!(
        RawConfig::from_env(&env)
            .unwrap()
            .resolve()
            .unwrap()
            .sandbox
            .enabled
    );
}

#[test]
fn native_external_defaults_are_rejected_only_when_sandbox_is_enabled() {
    for mode in ["readonly", "readwrite"] {
        let mut value = json!({"permission":{"path":{"external":mode}}});
        let error = raw(value.clone()).resolve().unwrap_err();
        assert!(error.to_string().contains("permission.path.external"));
        value["sandbox"] = json!({"enabled":false});
        assert!(!raw(value).resolve().unwrap().sandbox.enabled);
    }
}

#[test]
fn layered_settings_validate_native_workspace_conflicts_before_saving() {
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
    assert!(validate(&edited).is_err());
    edited["sandbox"] = json!({"enabled":false});
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
