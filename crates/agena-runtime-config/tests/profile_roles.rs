use agena_runtime_config::{AgentProfileMode, RawConfig, resolve_agent_profile};
use serde_json::{Value, json};

fn resolve(value: Value) -> agena_runtime_config::ResolvedConfig {
    serde_json::from_value::<RawConfig>(value)
        .unwrap()
        .resolve()
        .unwrap()
}

#[test]
fn empty_configuration_has_executable_roles_and_role_specific_instructions() {
    let config = resolve(json!({}));
    assert_eq!(config.default_profile.as_deref(), Some("main"));
    assert_eq!(config.default_subagent_profile.as_deref(), Some("worker"));
    assert_eq!(config.profiles, config.builtin_profiles);
    let main = resolve_agent_profile(&config.profiles, "main").unwrap();
    let explore = resolve_agent_profile(&config.profiles, "explore").unwrap();
    assert_eq!(main.mode, AgentProfileMode::Primary);
    assert_eq!(explore.mode, AgentProfileMode::Subagent);
    assert!(explore.read_only);
    assert!(
        resolve_agent_profile(&config.profiles, "planner")
            .unwrap()
            .read_only
    );
    let main_text = &main.prompts[0].inline[0].body;
    let explore_text = &explore.prompts[0].inline[0].body;
    assert!(main_text.contains("agena.agent.spawn"));
    for name in ["explore", "planner", "worker", "reviewer", "verifier"] {
        assert!(main_text.contains(name));
    }
    assert!(explore_text.contains("read-only investigation"));
    assert!(!explore_text.contains("# Main role"));
    assert_eq!(
        serde_json::to_value(explore.permission).unwrap()["path"]["workspace"],
        "readonly"
    );
    assert_eq!(explore.max_turns, Some(40));
}

#[test]
fn the_resolved_read_only_policy_is_inherited_with_the_role() {
    let config = resolve(json!({"profiles": {
        "explore": {"isolation": "worktree"},
        "project-research": {"extends": "explore"},
        "project-worker": {"extends": "project-research", "read_only": false}
    }}));
    let research = resolve_agent_profile(&config.profiles, "project-research").unwrap();
    assert!(research.read_only);
    assert!(research.read_only_ceiling().is_some());
    assert_eq!(research.isolation, agena_domain::AgentIsolation::Shared);
    let worker = resolve_agent_profile(&config.profiles, "project-worker").unwrap();
    assert!(worker.read_only_ceiling().is_none());
    assert_eq!(worker.isolation, agena_domain::AgentIsolation::Worktree);
}

#[test]
fn custom_roles_extend_builtins_and_can_override_inherited_visibility_and_mode() {
    let config = resolve(json!({"profiles":{
        "private-scout":{"extends":"explore","hidden":true},
        "research":{"extends":"private-scout","mode":"all","hidden":false}
    }}));
    let hidden = resolve_agent_profile(&config.profiles, "private-scout").unwrap();
    assert_eq!(hidden.mode, AgentProfileMode::Subagent);
    assert!(hidden.hidden);
    let visible = resolve_agent_profile(&config.profiles, "research").unwrap();
    assert!(visible.mode.allows_primary());
    assert!(visible.mode.allows_subagent());
    assert!(!visible.hidden);
    assert_eq!(hidden.prompts, visible.prompts);
    assert_eq!(hidden.permission, visible.permission);
}

#[test]
fn an_existing_custom_profile_remains_usable_in_both_roles() {
    let config = resolve(json!({"profiles":{"existing":{"description":"Existing custom role"}}}));
    let profile = resolve_agent_profile(&config.profiles, "existing").unwrap();
    assert_eq!(profile.mode, AgentProfileMode::All);
    assert!(!profile.hidden);
}

#[test]
fn explicit_prompt_replacement_preserves_other_builtin_defaults() {
    let config = resolve(json!({"profiles":{"explore":{
        "description":"A custom explorer", "mode":"subagent", "max_turns":6,
        "prompts":{"mode":"replace","order":["custom"],"inline":[{"id":"custom","body":"CUSTOM_SCOUT"}]}
    }}}));
    let profile = resolve_agent_profile(&config.profiles, "explore").unwrap();
    assert_eq!(profile.prompts.len(), 1);
    assert_eq!(profile.prompts[0].inline[0].body, "CUSTOM_SCOUT");
    assert_eq!(profile.max_turns, Some(6));
    assert!(!profile.permission.is_empty());
    assert!(!config.builtin_profiles["explore"].permission.is_empty());
    assert_eq!(config.builtin_profiles["explore"].max_turns, Some(40));
}

#[test]
fn defaults_and_automatic_fallback_roles_must_support_their_execution_scope() {
    for (value, expected) in [
        (json!({"default_profile":"explore"}), "default_profile"),
        (
            json!({"default_subagent_profile":"main"}),
            "default_subagent_profile",
        ),
        (
            json!({"default_subagent_profile":"missing"}),
            "default_subagent_profile",
        ),
        (
            json!({"profiles":{"main":{"mode":"subagent"}}}),
            "profiles.main",
        ),
        (
            json!({"profiles":{"worker":{"mode":"primary"}}}),
            "profiles.worker",
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
fn hidden_roles_remain_available_for_explicit_configuration() {
    let config = resolve(json!({
        "default_subagent_profile":"private-worker",
        "profiles":{"private-worker":{"extends":"worker","hidden":true}}
    }));
    let selected = resolve_agent_profile(
        &config.profiles,
        config.default_subagent_profile.as_deref().unwrap(),
    )
    .unwrap();
    assert!(selected.hidden);
    assert!(selected.mode.allows_subagent());
}

#[test]
fn empty_default_values_restore_builtin_roles_and_raw_settings_stay_user_owned() {
    let raw: RawConfig = serde_json::from_value(json!({
        "default_profile":"", "default_subagent_profile":""
    }))
    .unwrap();
    let document = serde_json::to_value(&raw).unwrap();
    assert!(document.get("profiles").is_none());
    assert!(document.get("builtin_profiles").is_none());
    let config = raw.resolve().unwrap();
    assert_eq!(config.default_profile.as_deref(), Some("main"));
    assert_eq!(config.default_subagent_profile.as_deref(), Some("worker"));
}

#[test]
fn invalid_role_mode_is_rejected_at_deserialization() {
    let error = serde_json::from_value::<RawConfig>(json!({
        "profiles":{"bad":{"mode":"sometimes"}}
    }))
    .unwrap_err();
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn changing_only_a_builtin_model_keeps_its_role_instructions_and_boundaries() {
    let config = resolve(json!({"profiles":{"explore":{"model":{"thinking_mode":"high"}}}}));
    let profile = resolve_agent_profile(&config.profiles, "explore").unwrap();
    assert_eq!(profile.mode, AgentProfileMode::Subagent);
    assert_eq!(profile.selection.thinking_mode.as_deref(), Some("high"));
    assert_eq!(
        profile.prompts[0].inline[0].body,
        config.builtin_profiles["explore"].prompts.inline[0].body
    );
    assert_eq!(
        profile.permission,
        config.builtin_profiles["explore"].permission
    );
    assert!(!profile.description.is_empty());
}

#[test]
fn builtin_prompt_customization_appends_or_explicitly_suppresses_factory_instructions() {
    let config = resolve(json!({"profiles":{"main":{"prompts":{
        "order":["project"],"inline":[{"id":"project","body":"PROJECT_RULE"}]
    }}}}));
    let profile = resolve_agent_profile(&config.profiles, "main").unwrap();
    assert_eq!(
        profile.prompts[0]
            .order
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![
            "agena-profile-main",
            "agena.profile.investigation",
            "agena.profile.planning",
            "agena.profile.implementation",
            "agena.profile.verification",
            "project"
        ]
    );
    let suppressed = resolve(json!({"profiles":{"main":{"prompts":{
        "disabled":["agena-profile-main"], "order":["project"], "inline":[{"id":"project","body":"PROJECT_RULE"}]
    }}}}));
    let profile = resolve_agent_profile(&suppressed.profiles, "main").unwrap();
    assert_eq!(profile.prompts[0].order.len(), 1);
    assert_eq!(profile.prompts[0].inline.len(), 1);
    assert_eq!(profile.prompts[0].inline[0].body, "PROJECT_RULE");
}

#[test]
fn factory_roles_reference_the_common_library_using_compatible_methods() {
    use agena_prompt_layers::{LayerSource, PromptContext, PromptScope};
    let config = resolve(json!({}));
    let library = config.prompts.library(PromptScope::Global).unwrap();
    let roles: &[(&str, &[&str])] = &[
        (
            "main",
            &[
                "agena.profile.planning",
                "agena.profile.implementation",
                "agena.profile.verification",
            ],
        ),
        (
            "worker",
            &["agena.profile.implementation", "agena.profile.verification"],
        ),
        ("reviewer", &["agena.profile.read-only"]),
        ("explore", &["agena.profile.read-only"]),
        (
            "planner",
            &["agena.profile.planning", "agena.profile.read-only"],
        ),
        ("verifier", &["agena.profile.verification"]),
    ];
    for &(role, methods) in roles {
        let layer = &config.profiles[role].prompts;
        let prompt = agena_prompt_layers::resolve(
            &[LayerSource::new(PromptScope::Profile, layer, &library)],
            &PromptContext::default(),
        )
        .unwrap();
        let ids: Vec<_> = prompt
            .fragments
            .iter()
            .map(|fragment| fragment.id.as_str())
            .collect();
        let role_id = format!("agena-profile-{role}");
        let mut expected = vec![role_id.as_str(), "agena.profile.investigation"];
        expected.extend_from_slice(methods);
        assert_eq!(ids, expected, "{role}");
    }
}

#[test]
fn builtin_customization_does_not_hide_invalid_duplicate_prompt_order() {
    let error = serde_json::from_value::<RawConfig>(json!({"profiles":{"main":{"prompts":{
        "order":["project","project"],"inline":[{"id":"project","body":"PROJECT_RULE"}]
    }}}}))
    .unwrap()
    .resolve()
    .unwrap_err();
    assert!(error.to_string().contains("profiles.main.prompts"));
}
