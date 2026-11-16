use agena_runtime_config::{
    ConfigEnvironment, ConfigSettingsEditOptions, ConfigSettingsLayer, ConfigSettingsSetInput,
    RuntimeConfigSettingsError, set_layered_file_setting, set_runtime_file_setting,
    validate_layered_config_text,
};
use serde_json::json;

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

#[test]
fn requested_activation_is_independent_of_file_changes_in_both_edit_paths() {
    for runtime_service in [false, true] {
        for changed in [false, true] {
            for dry_run in [false, true] {
                for reload in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let global = directory.path().join("agena.json");
                    let workspace = directory.path().join("project/.agena/agena.json");
                    std::fs::create_dir_all(workspace.parent().unwrap()).unwrap();
                    let previous = json!({"permission":{"path":{
                        "workspace":"readonly", "external":"deny"
                    }}});
                    let original_bytes = serde_json::to_vec(&previous).unwrap();
                    std::fs::write(&global, &original_bytes).unwrap();
                    let value = if changed {
                        json!({"path":{"workspace":"readwrite", "external":"readonly"}})
                    } else {
                        previous["permission"].clone()
                    };
                    let input = ConfigSettingsSetInput {
                        path: "permission".into(),
                        value: value.clone(),
                        options: ConfigSettingsEditOptions {
                            expected_revision: None,
                            dry_run,
                            validate: true,
                            reload,
                        },
                    };
                    let response = if runtime_service {
                        let global_validation = global.clone();
                        let workspace_validation = workspace.clone();
                        let validator = move |_: &std::path::Path, text: &str| {
                            validate_layered_config_text(
                                &global_validation,
                                &workspace_validation,
                                ConfigSettingsLayer::Global,
                                text,
                                &EmptyEnvironment,
                            )
                            .map_err(|error| {
                                RuntimeConfigSettingsError::invalid_input(error.to_string())
                            })
                        };
                        set_runtime_file_setting(&global, input, Some(&validator)).unwrap()
                    } else {
                        set_layered_file_setting(
                            &global,
                            &workspace,
                            ConfigSettingsLayer::Global,
                            input,
                        )
                        .unwrap()
                    };
                    assert_eq!(response.changed, changed);
                    assert_eq!(response.reload_requested, reload);
                    assert_eq!(response.reload_required, reload && !dry_run);
                    if !changed || dry_run {
                        assert_eq!(std::fs::read(&global).unwrap(), original_bytes);
                    } else {
                        let saved: serde_json::Value =
                            serde_json::from_slice(&std::fs::read(&global).unwrap()).unwrap();
                        assert_eq!(saved["permission"], value);
                    }
                }
            }
        }
    }
}
