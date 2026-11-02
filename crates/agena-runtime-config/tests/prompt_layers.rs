//! Layered prompt configuration: per-scope capture, composition, and validation.

use std::fs;
use std::path::Path;

use agena_prompt_layers::{FragmentId, LayerMode};
use agena_runtime_config::{
    ConfigLoader, LoadConfigRequest, ProcessEnvironment, PromptScopeConfig,
};
use serde_json::json;

fn fixture(
    global: serde_json::Value,
    workspace: serde_json::Value,
) -> (tempfile::TempDir, LoadConfigRequest) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("agena.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&global).unwrap()).unwrap();
    let workspace_root = dir.path().join("workspace");
    fs::create_dir_all(workspace_root.join(".agena")).unwrap();
    fs::write(
        workspace_root.join(".agena/agena.json"),
        serde_json::to_vec_pretty(&workspace).unwrap(),
    )
    .unwrap();
    let request = LoadConfigRequest {
        config_path: Some(config_path),
        workspace_root: Some(workspace_root),
        overrides: Vec::new(),
    };
    (dir, request)
}

fn write_fragment(dir: &Path, id: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(format!("{id}.md")), body).unwrap();
}

#[test]
fn global_and_workspace_layers_stay_separate_and_compose() {
    let (dir, request) = fixture(
        json!({
            "prompts": {
                "mode": "append",
                "order": ["house-style"],
                "inline": [{"id": "shared", "title": "Global shared", "body": "global shared"}]
            }
        }),
        json!({
            "prompts": {
                "mode": "append",
                "order": ["project-notes"],
                "disabled": ["house-style"]
            }
        }),
    );
    write_fragment(
        &dir.path().join("prompts"),
        "house-style",
        "Global house style.\n",
    );
    write_fragment(
        &dir.path().join("workspace/.agena/prompts"),
        "project-notes",
        "Project notes.\n",
    );

    let resolution = ConfigLoader::new(ProcessEnvironment)
        .load(&request)
        .unwrap();
    let prompts = &resolution.config.prompts;
    assert_eq!(
        prompts.global.order,
        vec![FragmentId::parse("house-style").unwrap()]
    );
    assert_eq!(
        prompts.workspace.order,
        vec![FragmentId::parse("project-notes").unwrap()]
    );
    assert_eq!(
        prompts.workspace.disabled,
        vec![FragmentId::parse("house-style").unwrap()]
    );
    assert_eq!(prompts.global.inline.len(), 1);
    assert_eq!(prompts.global_dir, dir.path().join("prompts"));
    assert_eq!(
        prompts.workspace_dir,
        request
            .workspace_root
            .as_ref()
            .unwrap()
            .join(".agena/prompts")
    );
    assert_eq!(prompts.global.mode, LayerMode::Append);
}

#[test]
fn missing_project_file_leaves_the_global_layer_intact() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("agena.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "prompts": {"order": [], "inline": [{"id": "only", "body": "only global"}]}
        }))
        .unwrap(),
    )
    .unwrap();
    write_fragment(&dir.path().join("prompts"), "only", "Global only.\n");
    let resolution = ConfigLoader::new(ProcessEnvironment)
        .load(&LoadConfigRequest {
            config_path: Some(config_path),
            workspace_root: Some(dir.path().join("empty-workspace")),
            overrides: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        resolution.config.prompts.workspace,
        PromptScopeConfig::default()
    );
    assert_eq!(resolution.config.prompts.global.inline.len(), 1);
}

#[test]
fn referencing_a_missing_fragment_fails_at_load_time() {
    let (dir, request) = fixture(json!({"prompts": {"order": ["absent"]}}), json!({}));
    write_fragment(&dir.path().join("prompts"), "present", "Present.\n");
    let error = ConfigLoader::new(ProcessEnvironment)
        .load(&request)
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("absent"), "{message}");
    assert!(message.contains("absent.md"), "{message}");
}

#[test]
fn model_scoped_layers_round_trip_and_require_a_pattern() {
    let (dir, request) = fixture(
        json!({
            "prompts": {
                "order": ["base"],
                "models": [{"match": "anthropic/*", "order": ["base"]}]
            }
        }),
        json!({}),
    );
    write_fragment(&dir.path().join("prompts"), "base", "Base.\n");
    let resolution = ConfigLoader::new(ProcessEnvironment)
        .load(&request)
        .unwrap();
    let models = &resolution.config.prompts.global.models;
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].match_pattern, "anthropic/*");
    assert_eq!(models[0].order, vec![FragmentId::parse("base").unwrap()]);

    let (dir, request) = fixture(
        json!({"prompts": {"models": [{"match": "  ", "order": []}]}}),
        json!({}),
    );
    write_fragment(&dir.path().join("prompts"), "base", "Base.\n");
    let error = ConfigLoader::new(ProcessEnvironment)
        .load(&request)
        .unwrap_err();
    assert!(error.to_string().contains("non-empty `match`"), "{error}");
}

#[test]
fn retired_session_prompt_field_is_rejected() {
    let (_dir, request) = fixture(
        json!({"session": {"additional_system_prompt": "legacy"}}),
        json!({}),
    );
    let error = ConfigLoader::new(ProcessEnvironment)
        .load(&request)
        .unwrap_err();
    assert!(
        error.to_string().contains("additional_system_prompt"),
        "{error}"
    );
}
