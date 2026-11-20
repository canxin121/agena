//! Named profile loading, inheritance, and configuration-write contracts.
use std::fs;

use agena_prompt_layers::LayerMode;
use agena_runtime_config::config::ConfigSettingsLayer;
use agena_runtime_config::{
    ConfigEnvironment, ConfigLoader, LoadConfigRequest, RawConfig, resolve_agent_profile,
    validate_layered_config_text,
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

fn fixture(global: Value, workspace: Value) -> (tempfile::TempDir, LoadConfigRequest) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("agena.json");
    let workspace_root = dir.path().join("project");
    fs::create_dir_all(workspace_root.join(".agena")).unwrap();
    fs::write(&config_path, serde_json::to_vec(&global).unwrap()).unwrap();
    fs::write(
        workspace_root.join(".agena/agena.json"),
        serde_json::to_vec(&workspace).unwrap(),
    )
    .unwrap();
    (
        dir,
        LoadConfigRequest {
            config_path: Some(config_path),
            workspace_root: Some(workspace_root),
            ..Default::default()
        },
    )
}

fn resolve(value: Value) -> agena_runtime_config::ResolvedConfig {
    serde_json::from_value::<RawConfig>(value)
        .unwrap()
        .resolve()
        .unwrap()
}

fn provider() -> Value {
    json!({"auth": {
        "mode":"api", "subtype":"custom", "base_url":"http://localhost:1234",
        "api_key":{"kind":"env","value":"AGENA_PROFILE_TEST_API_KEY"}
    }, "adapters":{"openai_chat_completions":{"enabled":true}}})
}

#[test]
fn a_profile_uses_the_complete_named_permission_instead_of_inheriting_rule_tables() {
    let config = resolve(json!({
        "permission": {"tools": {"names": {"agena.fs.read": "allow"}}},
        "access_control": {"profiles":{"closed":{"name":"Closed", "complete":true}}},
        "profiles":{"main":{"permission_profile":"closed"}}
    }));
    let profile = resolve_agent_profile(&config.profiles, "main")
        .unwrap()
        .with_permission_profile(&config.access_control)
        .unwrap();
    assert_eq!(
        profile.effective_permission(&config.permission),
        agena_domain::PermissionConfig::default().standalone()
    );
}

#[test]
fn old_configuration_and_unselected_catalog_preserve_global_defaults() {
    let baseline = resolve(json!({}));
    let with_catalog = resolve(json!({"profiles":{"review":{
        "permission":{"path":{"workspace":"readonly"}},
        "max_turns":12
    }}}));
    assert_eq!(with_catalog.permission, baseline.permission);
    assert_eq!(with_catalog.default_selection, baseline.default_selection);
    assert_eq!(with_catalog.session.max_turns, baseline.session.max_turns);
    assert_eq!(with_catalog.default_profile.as_deref(), Some("main"));
    assert_eq!(baseline.profiles.len(), 4);
}

#[test]
fn workspace_replaces_named_entries_and_can_reset_inherited_default() {
    let (_dir, request) = fixture(
        json!({
            "sandbox": false,
            "default_profile":"review",
            "profiles":{
                "review":{"title":"Global", "permission":{"path":{"external":"readonly"}}, "max_turns":9},
                "build":{"description":"Retained"}
            }
        }),
        json!({
            "default_profile":"",
            "profiles":{"review":{"title":"Workspace","permission":{"path":{"workspace":"readonly"}}}}
        }),
    );
    let loader = ConfigLoader::new(EmptyEnvironment);
    let scoped = loader.load(&request).unwrap().config;
    assert_eq!(scoped.default_profile.as_deref(), Some("main"));
    assert_eq!(scoped.profiles["review"].title, "Workspace");
    assert!(scoped.profiles["review"].max_turns.is_none());
    let permission = serde_json::to_value(&scoped.profiles["review"].permission).unwrap();
    assert_eq!(permission["path"]["workspace"], "readonly");
    assert!(permission["path"].get("external").is_none());
    assert!(scoped.profiles.contains_key("build"));
    let global = loader.load_global(&request).unwrap().config;
    assert_eq!(global.default_profile.as_deref(), Some("review"));
    assert_eq!(global.profiles["review"].title, "Global");
}

#[test]
fn null_default_in_workspace_inherits_the_global_default() {
    let (_dir, request) = fixture(
        json!({
            "sandbox":false,"profiles":{"review":{}},"default_profile":"review"
        }),
        json!({"default_profile":null}),
    );
    let config = ConfigLoader::new(EmptyEnvironment)
        .load(&request)
        .unwrap()
        .config;
    assert_eq!(config.default_profile.as_deref(), Some("review"));
}

#[test]
fn explicit_inheritance_preserves_prompt_layers_and_overlays_permissions() {
    let config = resolve(json!({"profiles":{
        "base":{
            "description":"Shared style",
            "permission":{"path":{"workspace":"readwrite","external":"deny"}},
            "prompts":{"order":["base"],"inline":[{"id":"base","body":"Base instructions"}]},
            "max_turns":10
        },
        "review":{
            "extends":"base", "title":"Review",
            "permission":{"path":{"workspace":"readonly"},"network":{"default":"deny"}},
            "prompts":{"mode":"replace","order":["review"],"inline":[{"id":"review","body":"Review instructions"}]},
            "max_turns":0
        }
    }}));
    let profile = resolve_agent_profile(&config.profiles, "review").unwrap();
    assert_eq!(profile.title, "Review");
    assert_eq!(profile.description, "Shared style");
    assert_eq!(profile.max_turns, Some(0));
    assert_eq!(profile.prompts.len(), 2);
    assert_eq!(profile.prompts[0].mode, LayerMode::Append);
    assert_eq!(profile.prompts[1].mode, LayerMode::Replace);
    let permission = serde_json::to_value(profile.permission).unwrap();
    assert_eq!(permission["path"]["workspace"], "readonly");
    assert_eq!(permission["path"]["external"], "deny");
    assert_eq!(permission["network"]["default"], "deny");
}

#[test]
fn profile_routes_use_model_cascade_and_validate_effective_provider() {
    let config = resolve(json!({
        "providers":{"local":provider()},
        "profiles":{
            "base":{"model":{"provider":"local","adapter":"openai_chat_completions","model":"large","thinking_mode":"high"}},
            "quick":{"extends":"base","model":{"model":"small"}},
            "quiet":{"extends":"base","model":{"verbosity":"low"}}
        }
    }));
    let quick = resolve_agent_profile(&config.profiles, "quick").unwrap();
    assert_eq!(quick.selection.provider.as_deref(), Some("local"));
    assert_eq!(
        quick.selection.adapter.as_deref(),
        Some("openai_chat_completions")
    );
    assert_eq!(quick.selection.model.as_deref(), Some("small"));
    assert!(quick.selection.thinking_mode.is_none());
    let quiet = resolve_agent_profile(&config.profiles, "quiet").unwrap();
    assert_eq!(quiet.selection.model.as_deref(), Some("large"));
    assert_eq!(quiet.selection.thinking_mode.as_deref(), Some("high"));
    assert_eq!(quiet.selection.verbosity.as_deref(), Some("low"));
}

#[test]
fn partial_profile_route_can_inherit_the_instance_model() {
    let config = resolve(json!({
        "providers":{
            "default_selection":{"provider":"local","model":"main"},
            "local":provider()
        },
        "profiles":{"review":{"model":{"thinking_mode":"high"}}}
    }));
    let profile = resolve_agent_profile(&config.profiles, "review").unwrap();
    assert_eq!(profile.selection.thinking_mode.as_deref(), Some("high"));
    assert!(profile.selection.provider.is_none());
}

#[test]
fn variant_only_profiles_allow_the_provider_catalog_to_choose_the_default_route() {
    let config = resolve(json!({
        "providers":{"local":provider()},
        "profiles":{"careful":{"model":{"thinking_mode":"high","parallel_tool_calls":false}}}
    }));
    assert!(config.default_selection.is_empty());
    let profile = resolve_agent_profile(&config.profiles, "careful").unwrap();
    assert!(profile.selection.model.is_none());
    assert_eq!(profile.selection.thinking_mode.as_deref(), Some("high"));
    assert_eq!(profile.selection.parallel_tool_calls, Some(false));
}

#[test]
fn invalid_catalogs_are_rejected_even_when_unselected() {
    for (value, expected) in [
        (
            json!({"profiles":{"review":{}},"default_profile":"missing"}),
            "default_profile",
        ),
        (
            json!({"profiles":{"review":{"extends":"missing"}}}),
            "missing",
        ),
        (
            json!({"profiles":{"a":{"extends":"b"},"b":{"extends":"a"}}}),
            "cycle",
        ),
        (json!({"profiles":{"a":{"extends":"a"}}}), "cycle"),
        (json!({"profiles":{"../escape":{}}}), "name"),
        (json!({"profiles":{"review":{"extends":""}}}), "name"),
        (
            json!({"profiles":{"review":{"model":{"provider":"missing","model":"x"}}}}),
            "provider",
        ),
        (
            json!({"profiles":{"review":{"model":{"model":"x"}}}}),
            "provider",
        ),
        (
            json!({"profiles":{"review":{"prompts":{"order":["same","same"]}}}}),
            "prompts",
        ),
    ] {
        let error = serde_json::from_value::<RawConfig>(value)
            .unwrap()
            .resolve()
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn unknown_profile_fields_and_bad_permissions_fail_deserialization() {
    for profile in [
        json!({"system_prompt":"legacy"}),
        json!({"permissions":{}}),
        json!({"permission":{"path":{"workspace":"maybe"}}}),
        json!({"max_turns":-1}),
    ] {
        assert!(
            serde_json::from_value::<RawConfig>(json!({"profiles":{"review":profile}})).is_err()
        );
    }
}

#[test]
fn profile_model_rejects_disabled_provider_and_unknown_adapter() {
    for (disabled, adapter) in [(true, "openai_chat_completions"), (false, "missing")] {
        let mut local = provider();
        local["enabled"] = json!(!disabled);
        let raw: RawConfig = serde_json::from_value(json!({
            "providers":{"local":local},
            "profiles":{"review":{"model":{"provider":"local","adapter":adapter,"model":"main"}}}
        }))
        .unwrap();
        assert!(
            raw.resolve()
                .unwrap_err()
                .to_string()
                .contains(if disabled { "disabled" } else { "adapter" })
        );
    }
}

#[test]
fn profile_prompts_resolve_saved_files_and_ancestor_inline_fragments() {
    let (dir, request) = fixture(
        json!({
            "sandbox":false,
            "prompts":{"library":[{"id":"saved","body":"Saved instructions"}]},
            "profiles":{
                "base":{"prompts":{"inline":[{"id":"ancestor","body":"Inherited fragment"}]}}
            }
        }),
        json!({
            "prompts":{"library":[{"id":"workspace","body":"Workspace instructions"}]},
            "profiles":{"review":{"extends":"base","prompts":{"order":["saved","file","ancestor","workspace"]}}}
        }),
    );
    fs::create_dir_all(dir.path().join("prompts")).unwrap();
    fs::write(dir.path().join("prompts/file.md"), "File instructions").unwrap();
    let config = ConfigLoader::new(EmptyEnvironment)
        .load(&request)
        .unwrap()
        .config;
    assert_eq!(
        resolve_agent_profile(&config.profiles, "review")
            .unwrap()
            .prompts
            .len(),
        2
    );
}

#[test]
fn missing_prompt_reference_fails_startup_and_settings_validation() {
    let (_dir, request) = fixture(json!({"sandbox":false}), json!({}));
    let edited = json!({
        "sandbox":false,
        "profiles":{"review":{"prompts":{"order":["missing"]}}}
    })
    .to_string();
    let global = request.config_path.as_ref().unwrap();
    let workspace = request
        .workspace_root
        .as_ref()
        .unwrap()
        .join(".agena/agena.json");
    let error = validate_layered_config_text(
        global,
        &workspace,
        ConfigSettingsLayer::Global,
        &edited,
        &EmptyEnvironment,
    )
    .unwrap_err();
    assert!(error.to_string().contains("profiles.review.prompts"));
    fs::write(global, edited).unwrap();
    let error = ConfigLoader::new(EmptyEnvironment)
        .load(&request)
        .unwrap_err();
    assert!(error.to_string().contains("missing"));
}

#[test]
fn descendants_cannot_supply_missing_ancestor_prompt_references() {
    let (_dir, request) = fixture(
        json!({
            "sandbox":false,
            "profiles":{
                "base":{"prompts":{"order":["late"]}},
                "review":{"extends":"base","prompts":{"inline":[{"id":"late","body":"Too late"}]}}
            }
        }),
        json!({}),
    );
    let error = ConfigLoader::new(EmptyEnvironment)
        .load(&request)
        .unwrap_err();
    assert!(error.to_string().contains("late"));
}

#[test]
fn reloading_replaces_profile_definitions_without_changing_the_saved_catalog() {
    let (_dir, request) = fixture(
        json!({
            "sandbox":false,"default_profile":"review",
            "profiles":{"review":{"description":"Old","permission":{"path":{"workspace":"readwrite"}}}}
        }),
        json!({}),
    );
    let loader = ConfigLoader::new(EmptyEnvironment);
    let first = loader.load(&request).unwrap().config;
    fs::write(request.config_path.as_ref().unwrap(), json!({
        "sandbox":false,"default_profile":"review",
        "profiles":{"review":{"description":"New","permission":{"path":{"workspace":"readonly"}}}}
    }).to_string()).unwrap();
    let second = loader.load(&request).unwrap().config;
    assert_eq!(first.profiles["review"].description, "Old");
    assert_eq!(second.profiles["review"].description, "New");
    assert_eq!(
        serde_json::to_value(&second.profiles["review"].permission).unwrap()["path"]["workspace"],
        "readonly"
    );
    assert_eq!(second.default_profile, first.default_profile);
}

#[test]
fn overlong_inheritance_fails_without_recursive_stack_growth() {
    let mut profiles = serde_json::Map::new();
    profiles.insert("p0".into(), json!({}));
    for index in 1..65 {
        profiles.insert(
            format!("p{index}"),
            json!({"extends":format!("p{}",index-1)}),
        );
    }
    let error = serde_json::from_value::<RawConfig>(json!({"profiles":profiles}))
        .unwrap()
        .resolve()
        .unwrap_err();
    assert!(error.to_string().contains("depth"));
}

#[test]
fn global_profiles_cannot_depend_on_workspace_only_definitions() {
    for (global, workspace, expected) in [
        (
            json!({"sandbox":false,"profiles":{"review":{"extends":"project"}}}),
            json!({"profiles":{"project":{}}}),
            "project",
        ),
        (
            json!({"sandbox":false,"default_profile":"project"}),
            json!({"profiles":{"project":{}}}),
            "default_profile",
        ),
        (
            json!({"sandbox":false,"profiles":{"review":{"prompts":{"order":["project"]}}}}),
            json!({"prompts":{"library":[{"id":"project","body":"Project only"}]}}),
            "profiles.review.prompts",
        ),
    ] {
        let (_dir, request) = fixture(global.clone(), workspace);
        let error = ConfigLoader::new(EmptyEnvironment)
            .load(&request)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        let error = validate_layered_config_text(
            request.config_path.as_ref().unwrap(),
            &request
                .workspace_root
                .as_ref()
                .unwrap()
                .join(".agena/agena.json"),
            ConfigSettingsLayer::Global,
            &global.to_string(),
            &EmptyEnvironment,
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn global_subagent_default_cannot_depend_on_a_workspace_only_role() {
    let (_dir, request) = fixture(
        json!({"sandbox":false,"default_subagent_profile":"project-scout"}),
        json!({"profiles":{"project-scout":{"extends":"explore"}}}),
    );
    let error = ConfigLoader::new(EmptyEnvironment)
        .load(&request)
        .unwrap_err();
    assert!(
        error.to_string().contains("default_subagent_profile"),
        "{error}"
    );
}
