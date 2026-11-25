use super::queue_tests::fixture_with_config;
use super::*;
use agena_domain::{AccessKind, PermissionDecision};
use agena_prompt_layers::{FragmentId, PromptFragment, PromptLayer, PromptScope};

fn layer(id: &str, body: &str) -> PromptLayer {
    let id = FragmentId::parse(id).unwrap();
    PromptLayer {
        order: vec![id.clone()],
        inline: vec![PromptFragment::new(id, body)],
        ..Default::default()
    }
}

fn config() -> RuntimeSessionManagerConfig {
    let mut config = RuntimeSessionManagerConfig {
        sandbox_enabled: false,
        default_profile: Some("review".into()),
        permission: serde_json::from_value(
            serde_json::json!({"path":{"workspace":"readwrite","external":"deny"}}),
        )
        .unwrap(),
        ..Default::default()
    };
    config.profiles.insert(
        "review".into(),
        agena_runtime_config::AgentProfileConfig {
            permission: serde_json::from_value(
                serde_json::json!({"path":{"workspace":"readonly"}}),
            )
            .unwrap(),
            prompts: layer("review-note", "PROFILE_REVIEW_INSTRUCTION"),
            max_turns: Some(3),
            ..Default::default()
        },
    );
    config.profiles.insert(
        "build".into(),
        agena_runtime_config::AgentProfileConfig {
            permission: serde_json::from_value(
                serde_json::json!({"path":{"workspace":"readwrite"}}),
            )
            .unwrap(),
            ..Default::default()
        },
    );
    config
}

#[tokio::test]
async fn profile_changes_the_actual_executor_and_preserves_a_stricter_ceiling() {
    let (manager, session) = fixture_with_config(config()).await;
    // Legacy sessions use the fixed main role, not the configured new-session default.
    assert!(
        manager
            .get_session(session.get())
            .await
            .unwrap()
            .execution
            .profile_is_default
    );
    manager
        .set_session_profile(session.get(), Some("review".into()))
        .await
        .unwrap();
    let context = manager.execution_context_value(session).await.unwrap();
    assert_eq!(context.max_turns, Some(3));
    let executor = manager
        .tool_executor()
        .for_session_context_async(&context)
        .await;
    assert!(matches!(
        executor
            .path_permission_check(AccessKind::Read, "/queue-tests/file.rs")
            .decision,
        PermissionDecision::Allow
    ));
    assert!(matches!(
        executor
            .path_permission_check(AccessKind::Write, "/queue-tests/file.rs")
            .decision,
        PermissionDecision::Deny { .. }
    ));
    let readonly = context.effective_permission.clone();
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                permission_ceiling: Some(readonly),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    manager
        .set_session_profile(session.get(), Some("build".into()))
        .await
        .unwrap();
    let context = manager.execution_context_value(session).await.unwrap();
    let executor = manager
        .tool_executor()
        .for_session_context_async(&context)
        .await;
    assert!(matches!(
        executor
            .path_permission_check(AccessKind::Write, "/queue-tests/file.rs")
            .decision,
        PermissionDecision::Deny { .. }
    ));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn profile_prompt_reaches_final_system_assembly_and_request_override_keeps_identity() {
    let mut config = config();
    config.prompts.global.inline = layer("global-note", "GLOBAL_INSTRUCTION").inline;
    config.prompts.global.order = vec![FragmentId::parse("global-note").unwrap()];
    config.profiles.get_mut("review").unwrap().extends = Some("base".into());
    config.profiles.insert(
        "base".into(),
        agena_runtime_config::AgentProfileConfig {
            prompts: layer("base-note", "BASE_PROFILE_INSTRUCTION"),
            ..Default::default()
        },
    );
    let (manager, session) = fixture_with_config(config).await;
    manager
        .set_session_profile(session.get(), Some("review".into()))
        .await
        .unwrap();
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                prompts: Some(layer("session-note", "SESSION_INSTRUCTION")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let context = manager.execution_context_value(session).await.unwrap();
    let executor = manager
        .tool_executor()
        .for_session_context_async(&context)
        .await;
    let state = manager.state.load_full();
    let model = ModelRef::new("test", "test");
    let mut inputs = super::super::session_prompt::PromptInputs {
        profile: &context.profile_prompts,
        project_instructions_on_demand: context.project_instructions_on_demand,
        preload_skills: &context.preload_skills,
        assignment: &context.agent_assignment,
        profile_from_bot: context.profile_from_bot,
        session: &context.prompts,
        bot: None,
        request_layer: None,
        model: &model,
        session_id: session.get(),
        workspace: "/queue-tests".into(),
        global: false,
        tools_enabled: false,
    };
    let resolution = manager
        .session_system_prompt(&executor, &state, &inputs, context.profile.as_deref())
        .await
        .unwrap();
    assert_eq!(
        resolution
            .fragments
            .iter()
            .filter(|f| f.scope == PromptScope::Profile)
            .count(),
        2
    );
    assert!(
        resolution.text.find("GLOBAL_INSTRUCTION").unwrap()
            < resolution.text.find("BASE_PROFILE_INSTRUCTION").unwrap()
    );
    assert!(
        resolution.text.find("BASE_PROFILE_INSTRUCTION").unwrap()
            < resolution.text.find("PROFILE_REVIEW_INSTRUCTION").unwrap()
    );
    assert!(
        resolution.text.find("PROFILE_REVIEW_INSTRUCTION").unwrap()
            < resolution.text.find("SESSION_INSTRUCTION").unwrap()
    );
    let builtin = resolution
        .fragments
        .iter()
        .find(|fragment| fragment.scope == PromptScope::Builtin)
        .unwrap();
    assert!(resolution.text.starts_with(&builtin.text));
    let request = super::super::session_prompt::request_system_layer(Some("REQUEST_INSTRUCTION"));
    inputs.request_layer = request.as_ref();
    let final_system = manager
        .session_system_prompt(&executor, &state, &inputs, context.profile.as_deref())
        .await
        .unwrap()
        .text;
    assert!(final_system.starts_with(&builtin.text));
    assert!(final_system.contains("REQUEST_INSTRUCTION"));
    assert!(!final_system.contains("PROFILE_REVIEW_INSTRUCTION"));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn new_roots_use_defaults_children_inherit_and_empty_restores_main() {
    let (manager, _) = fixture_with_config(config()).await;
    let workspace = std::env::temp_dir().join(format!(
        "agena-profile-session-workspace-{}",
        uuid::Uuid::new_v4(),
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let context = agena_runtime_session_core::model::SessionExecutionContext {
        effective_workspace_root: Some(workspace.clone()),
        ..Default::default()
    };
    let mut state = (*manager.state.load_full()).clone();
    state.executor = state.executor.for_session_context_async(&context).await;
    manager.state.store(Arc::new(state));
    let root = manager
        .create_session(SessionCreateRequest {
            profile: None,
            title: "profile root".into(),
            auto_title: false,
            parent_session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(root.execution.profile.as_deref(), Some("review"));
    let child = manager
        .create_session(SessionCreateRequest {
            profile: None,
            title: "child".into(),
            auto_title: false,
            parent_session_id: Some(root.id),
        })
        .await
        .unwrap();
    assert_eq!(child.execution.profile, root.execution.profile);
    let unprofiled = manager
        .create_session(SessionCreateRequest {
            profile: Some(String::new()),
            title: "unprofiled".into(),
            auto_title: false,
            parent_session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(unprofiled.execution.profile.as_deref(), Some("main"));
    assert!(unprofiled.execution.profile_is_default);
    let child = manager
        .create_session(SessionCreateRequest {
            profile: None,
            title: "unprofiled child".into(),
            auto_title: false,
            parent_session_id: Some(unprofiled.id),
        })
        .await
        .unwrap();
    assert_eq!(child.execution.profile.as_deref(), Some("main"));
    assert!(child.execution.profile_is_default);
    let unknown = manager
        .create_session(SessionCreateRequest {
            profile: Some("missing".into()),
            title: "invalid".into(),
            auto_title: false,
            parent_session_id: None,
        })
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("missing"));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
    std::fs::remove_dir_all(workspace).unwrap();
}

#[tokio::test]
async fn profile_reload_is_live_and_a_deleted_selection_can_be_cleared() {
    let (manager, session) = fixture_with_config(config()).await;
    manager
        .set_session_profile(session.get(), Some("review".into()))
        .await
        .unwrap();
    let mut state = (*manager.state.load_full()).clone();
    state.config.profiles.get_mut("review").unwrap().max_turns = Some(9);
    manager.state.store(Arc::new(state));
    assert_eq!(
        manager
            .execution_context_value(session)
            .await
            .unwrap()
            .max_turns,
        Some(9)
    );
    let mut state = (*manager.state.load_full()).clone();
    state.config.profiles.remove("review");
    manager.state.store(Arc::new(state));
    assert!(
        manager
            .get_session(session.get())
            .await
            .unwrap_err()
            .to_string()
            .contains("review")
    );
    assert!(
        manager
            .prompt_preview(session, &ModelRef::new("test", "test"))
            .await
            .is_err()
    );
    manager
        .set_session_profile(session.get(), None)
        .await
        .unwrap();
    assert!(
        manager
            .execution_context_value(session)
            .await
            .unwrap()
            .profile_is_default
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn inherited_models_are_not_pinned_but_explicit_request_models_are() {
    let mut config = config();
    for (name, model) in [("review", "review-model"), ("build", "build-model")] {
        config.profiles.get_mut(name).unwrap().model = Some(agena_domain::ModelSelectionConfig {
            provider: Some("provider".into()),
            model: Some(model.into()),
            ..Default::default()
        });
    }
    let (manager, session) = fixture_with_config(config).await;
    manager
        .set_session_profile(session.get(), Some("review".into()))
        .await
        .unwrap();
    let options = SessionRunOptions {
        selection_override: Some(ExecutionSelection::default()),
        model: ModelRef::new("provider", "review-model"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        model_supports_parallel_tool_calls: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    };
    manager
        .persist_submission_selection(session.get(), options.clone())
        .await
        .unwrap();
    assert!(
        manager
            .kernel()
            .reader
            .metadata(session)
            .await
            .unwrap()
            .selection
            .is_none()
    );
    manager
        .set_session_profile(session.get(), Some("build".into()))
        .await
        .unwrap();
    assert_eq!(
        manager
            .get_session(session.get())
            .await
            .unwrap()
            .execution
            .selection
            .model
            .as_deref(),
        Some("build-model")
    );
    let mut explicit = options.clone();
    explicit.selection_override = Some(ExecutionSelection {
        provider: Some("provider".into()),
        model: Some("explicit-model".into()),
        ..Default::default()
    });
    manager
        .persist_submission_selection(session.get(), explicit)
        .await
        .unwrap();
    manager
        .set_session_profile(session.get(), Some("review".into()))
        .await
        .unwrap();
    assert_eq!(
        manager
            .get_session(session.get())
            .await
            .unwrap()
            .execution
            .selection
            .model
            .as_deref(),
        Some("explicit-model")
    );
    let mut partial = options;
    partial.model = ModelRef::new("other-provider", "resolved-default");
    partial.selection_override = Some(ExecutionSelection {
        provider: Some("other-provider".into()),
        ..Default::default()
    });
    manager
        .persist_submission_selection(session.get(), partial)
        .await
        .unwrap();
    let selected = manager
        .get_session(session.get())
        .await
        .unwrap()
        .execution
        .selection;
    assert_eq!(selected.provider.as_deref(), Some("other-provider"));
    assert_eq!(selected.model.as_deref(), Some("resolved-default"));
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}
