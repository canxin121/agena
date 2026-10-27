use agena_runtime_config::{RawConfig, config::raw::RawProjectMergeKeys};
use serde_json::{Value, json};

fn merge_project(global: Value, project: Value) -> agena_runtime_config::ResolvedConfig {
    let mut raw = serde_json::from_value::<RawConfig>(global).unwrap();
    raw.merge_project_from_with_keys(
        serde_json::from_value(project).unwrap(),
        RawProjectMergeKeys::default(),
    );
    raw.resolve().unwrap()
}

#[test]
fn project_prompt_distinguishes_inheritance_from_explicit_empty() {
    for (project, expected) in [
        (json!({}), "global"),
        (
            json!({"session": {"additional_system_prompt": null}}),
            "global",
        ),
        (json!({"session": {"additional_system_prompt": ""}}), ""),
        (
            json!({"session": {"additional_system_prompt": "  project\n"}}),
            "  project\n",
        ),
    ] {
        let resolved = merge_project(
            json!({"session": {"additional_system_prompt": "global"}}),
            project,
        );
        assert_eq!(
            resolved.session.additional_system_prompt.as_deref(),
            Some(expected)
        );
        assert_eq!(
            serde_json::to_value(resolved.session).unwrap()["additional_system_prompt"],
            json!(expected)
        );
    }
    assert!(
        RawConfig::default()
            .resolve()
            .unwrap()
            .session
            .additional_system_prompt
            .is_none()
    );
}

fn provider_config(model: Value) -> Value {
    json!({"providers": {"local": {
        "auth": {
            "mode": "api", "subtype": "custom", "base_url": "http://localhost:1234",
            "api_key": {"kind": "env", "value": "AGENA_PROMPT_TEST_API_KEY"}
        },
        "adapters": {"openai_chat_completions": {
            "enabled": true,
            "models": {"model": model}
        }}
    }}})
}

#[test]
fn project_model_edits_inherit_prompt_without_merging_unrelated_metadata() {
    for (model, expected) in [
        (json!({}), "global model"),
        (json!({"additional_system_prompt": null}), "global model"),
        (json!({"additional_system_prompt": ""}), ""),
        (
            json!({"additional_system_prompt": "project model"}),
            "project model",
        ),
    ] {
        let resolved = merge_project(
            provider_config(json!({
                "additional_system_prompt": "global model",
                "display_name": "Global metadata"
            })),
            json!({"providers": {"local": {"adapters": {"openai_chat_completions": {
                "models": {"model": model}
            }}}}}),
        );
        let model = &resolved.providers["local"].models["openai_chat_completions/model"];
        assert_eq!(model.additional_system_prompt.as_deref(), Some(expected));
        assert!(model.definition.display_name.is_none());
    }
}

#[test]
fn configured_model_prompts_stay_route_specific() {
    let mut value = provider_config(json!({"additional_system_prompt": "first route"}));
    value["providers"]["local"]["adapters"]["openai_responses"] = json!({
        "enabled": true,
        "models": {"model": {"additional_system_prompt": ""}}
    });
    let resolved = serde_json::from_value::<RawConfig>(value)
        .unwrap()
        .resolve()
        .unwrap();
    let models = &resolved.providers["local"].models;
    assert_eq!(
        models["openai_chat_completions/model"]
            .additional_system_prompt
            .as_deref(),
        Some("first route")
    );
    assert_eq!(
        models["openai_responses/model"]
            .additional_system_prompt
            .as_deref(),
        Some("")
    );
}
