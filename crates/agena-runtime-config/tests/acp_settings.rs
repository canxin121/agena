use std::fs;

use agena_domain::ExternalAgent;
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

fn acp_config() -> Value {
    json!({
        "agents": {
            "codex": {
                "name": "Codex CLI",
                "command": "codex-acp",
                "args": ["--stdio"],
                "env": {
                    "CODEX_PROFILE": "work"
                },
                "client_tools": false,
                "writable_paths": ["/tmp/codex-state"]
            },
            "antigravity": {
                "disabled": true
            }
        }
    })
}

fn input(path: &str, value: Value) -> ConfigSettingsSetInput {
    ConfigSettingsSetInput {
        path: path.into(),
        value,
        options: ConfigSettingsEditOptions {
            validate: false,
            ..Default::default()
        },
    }
}

#[test]
fn global_acp_config_round_trips_and_validates() {
    let resolved = serde_json::from_value::<RawConfig>(json!({"acp": acp_config()}))
        .unwrap()
        .resolve()
        .unwrap();
    let codex = ExternalAgent::Custom("codex".into());
    let spec = resolved.acp.resolve(&codex).expect("resolved codex agent");
    assert_eq!(spec.name, "Codex CLI");
    assert_eq!(spec.command, "codex-acp");
    assert_eq!(spec.args, vec!["--stdio"]);
    assert_eq!(spec.env.get("CODEX_PROFILE").map(String::as_str), Some("work"));
    assert!(!spec.client_tools);
    assert!(resolved.acp.resolve(&ExternalAgent::Antigravity).is_none());
    assert!(resolved.acp.resolve(&ExternalAgent::ClaudeCode).is_some());

    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("global.json");
    set_runtime_file_setting(&file, input("acp", acp_config()), None).unwrap();
    let before = fs::read(&file).unwrap();

    for invalid in [
        json!({"agents": {"missing-cmd": {}}}),
        json!({"agents": {"bad-env": {"command": "acp", "env": {"AGENA_SECRET": "1"}}}}),
        json!({"agents": {"bad-path": {"command": "acp", "writable_paths": ["relative/dir"]}}}),
    ] {
        assert!(
            set_runtime_file_setting(&file, input("acp", invalid), None).is_err(),
            "expected invalid acp config to be rejected"
        );
        assert_eq!(fs::read(&file).unwrap(), before);
    }
}

#[test]
fn workspace_cannot_configure_instance_owned_acp_agents() {
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global.json");
    let workspace_root = root.path().join("project");
    let workspace = workspace_root.join(".agena/agena.json");
    fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    fs::write(&global, serde_json::to_vec(&json!({"acp": acp_config()})).unwrap()).unwrap();
    fs::write(&workspace, "{}").unwrap();
    let before = fs::read(&workspace).unwrap();

    for path in ["acp", "acp.agents"] {
        let value = if path == "acp" {
            acp_config()
        } else {
            acp_config()["agents"].clone()
        };
        let error = set_layered_file_setting(
            &global,
            &workspace,
            ConfigSettingsLayer::Workspace,
            input(path, value),
        )
        .unwrap_err();
        assert!(error.to_string().contains("instance-owned"), "{error}");
        assert_eq!(fs::read(&workspace).unwrap(), before);
    }

    let malicious = serde_json::to_string(&json!({
        "acp": {
            "agents": {
                "pwn": {"command": "/bin/sh", "args": ["-c", "echo pwned"]}
            }
        }
    }))
    .unwrap();
    let error = validate_layered_config_text(
        &global,
        &workspace,
        ConfigSettingsLayer::Workspace,
        &malicious,
        &EmptyEnvironment,
    )
    .unwrap_err();
    assert!(error.to_string().contains("instance-owned"), "{error}");

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
fn project_merge_never_overrides_global_acp_config() {
    let mut global =
        serde_json::from_value::<RawConfig>(json!({"acp": acp_config()})).unwrap();
    let project = serde_json::from_value::<RawConfig>(json!({
        "acp": {
            "agents": {
                "codex": {"command": "/evil/bin"},
                "injected": {"command": "/evil/injected"}
            }
        }
    }))
    .unwrap();
    global.merge_project_from_with_keys(project, Default::default());
    let resolved = global.resolve().unwrap();
    assert_eq!(
        resolved
            .acp
            .resolve(&ExternalAgent::Custom("codex".into()))
            .unwrap()
            .command,
        "codex-acp"
    );
    assert!(
        resolved
            .acp
            .resolve(&ExternalAgent::Custom("injected".into()))
            .is_none()
    );
}
