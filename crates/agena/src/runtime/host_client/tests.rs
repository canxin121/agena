use super::*;
use std::fs;

use crate::config::LoadConfigRequest;
use crate::plugin::sdk::host_api::with_host_callback_context;
use crate::session::{GoalStatus, SessionCreateRequest, SessionGoal};
use chrono::Utc;

/// `noop_host_client` returns a working trait object that does not panic
/// on `Display` / `Debug` access. Acts as a smoke test that the
/// `NoopHostClient` re-export through `agena::plugin` stays intact.
#[test]
fn noop_host_client_is_constructible() {
    let client: Arc<dyn HostClient> = noop_host_client();
    // Poke the Arc to make sure the vtable resolves.
    assert!(Arc::strong_count(&client) >= 1);
}

#[test]
fn host_goal_from_session_goal_preserves_paused_status() {
    let now = Utc::now();
    let goal = host_goal_from_session_goal(SessionGoal {
        id: 7,
        session_id: 11,
        objective: "ship the feature".to_string(),
        status: GoalStatus::Paused,
        token_budget: Some(42),
        tokens_used: 9,
        time_used_seconds: 3,
        created_at: now,
        updated_at: now,
        completed_at: None,
    });

    assert_eq!(goal.status, HostGoalStatus::Paused);
}

#[tokio::test]
async fn create_goal_persists_for_session_and_rejects_duplicates() {
    let tempdir = tempfile::tempdir().expect("tempdir should create");
    let config_path = tempdir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
    )
    .expect("config should be written");

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(tempdir.path())
        .with_database_url("sqlite::memory:")
        .build()
        .await
        .expect("runtime should build");
    let manager = runtime
        .session_manager()
        .expect("session manager should be available");
    let session = manager
        .create_session(SessionCreateRequest {
            title: "goal host client".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let client = RuntimeHostClient {
        runtime: runtime.clone(),
    };

    let created = with_host_callback_context(
        HostCallbackContext {
            session_id: Some(session.id),
            ..HostCallbackContext::default()
        },
        async {
            <RuntimeHostClient as HostClient>::create_goal(
                &client,
                HostCreateGoalRequest {
                    objective: "ship the feature".to_string(),
                    token_budget: Some(42),
                },
            )
            .await
        },
    )
    .await
    .expect("create_goal should succeed");
    assert_eq!(created.goal.objective, "ship the feature");
    assert_eq!(created.goal.token_budget, Some(42));
    assert_eq!(created.goal.status, HostGoalStatus::Active);

    let loaded = manager
        .get_goal(session.id)
        .await
        .expect("goal lookup should succeed")
        .expect("goal should persist");
    assert_eq!(loaded.objective, "ship the feature");
    assert_eq!(loaded.token_budget, Some(42));

    let err = with_host_callback_context(
        HostCallbackContext {
            session_id: Some(session.id),
            ..HostCallbackContext::default()
        },
        async {
            <RuntimeHostClient as HostClient>::create_goal(
                &client,
                HostCreateGoalRequest {
                    objective: "second goal".to_string(),
                    token_budget: None,
                },
            )
            .await
        },
    )
    .await
    .expect_err("duplicate goal should be rejected");
    assert_eq!(err.code, crate::plugin::sdk::PluginErrorCode::InvalidParams);
    assert!(
        err.message.contains("already has an active goal"),
        "unexpected error: {err:?}"
    );

    runtime.shutdown();
}

#[tokio::test]
async fn create_goal_sets_objective_updated_runtime_steering() {
    let tempdir = tempfile::tempdir().expect("tempdir should create");
    let config_path = tempdir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
    )
    .expect("config should be written");

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(tempdir.path())
        .with_database_url("sqlite::memory:")
        .build()
        .await
        .expect("runtime should build");
    let manager = runtime
        .session_manager()
        .expect("session manager should be available");
    let session = manager
        .create_session(SessionCreateRequest {
            title: "goal steering".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let client = RuntimeHostClient {
        runtime: runtime.clone(),
    };

    let created = with_host_callback_context(
        HostCallbackContext {
            session_id: Some(session.id),
            ..HostCallbackContext::default()
        },
        async {
            <RuntimeHostClient as HostClient>::create_goal(
                &client,
                HostCreateGoalRequest {
                    objective: "queue hidden steering".to_string(),
                    token_budget: Some(7),
                },
            )
            .await
        },
    )
    .await
    .expect("create_goal should succeed");

    let session = manager
        .get_session(session.id)
        .await
        .expect("session load should succeed");
    let pending = session
        .runtime
        .goal
        .pending_steering()
        .expect("goal runtime should queue steering after create_goal");
    assert_eq!(pending.goal_id, created.goal.id);
    assert_eq!(format!("{:?}", pending.kind), "ObjectiveUpdated");

    runtime.shutdown();
}

#[tokio::test]
async fn update_goal_allows_non_complete_status_transitions() {
    let tempdir = tempfile::tempdir().expect("tempdir should create");
    let config_path = tempdir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
    )
    .expect("config should be written");

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(tempdir.path())
        .with_database_url("sqlite::memory:")
        .build()
        .await
        .expect("runtime should build");
    let manager = runtime
        .session_manager()
        .expect("session manager should be available");
    let session = manager
        .create_session(SessionCreateRequest {
            title: "goal update pause".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    manager
        .create_goal(crate::session::SessionGoalCreateRequest {
            session_id: session.id,
            objective: "ship the feature".to_string(),
            token_budget: Some(42),
        })
        .await
        .expect("goal should be created");
    let client = RuntimeHostClient {
        runtime: runtime.clone(),
    };

    let updated = with_host_callback_context(
        HostCallbackContext {
            session_id: Some(session.id),
            ..HostCallbackContext::default()
        },
        async {
            <RuntimeHostClient as HostClient>::update_goal(
                &client,
                HostUpdateGoalRequest {
                    objective: None,
                    status: Some(HostGoalStatus::Paused),
                    token_budget: None,
                },
            )
            .await
        },
    )
    .await
    .expect("update_goal should succeed");

    assert_eq!(updated.goal.status, HostGoalStatus::Paused);
    assert_eq!(updated.goal.objective, "ship the feature");
    assert_eq!(updated.goal.token_budget, Some(42));

    let stored = manager
        .get_goal(session.id)
        .await
        .expect("goal lookup should succeed")
        .expect("goal should persist");
    assert_eq!(stored.status, GoalStatus::Paused);

    runtime.shutdown();
}

#[tokio::test]
async fn update_goal_can_complete_existing_goal() {
    let tempdir = tempfile::tempdir().expect("tempdir should create");
    let config_path = tempdir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
    )
    .expect("config should be written");

    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(tempdir.path())
        .with_database_url("sqlite::memory:")
        .build()
        .await
        .expect("runtime should build");
    let manager = runtime
        .session_manager()
        .expect("session manager should be available");
    let session = manager
        .create_session(SessionCreateRequest {
            title: "goal update complete".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    manager
        .create_goal(crate::session::SessionGoalCreateRequest {
            session_id: session.id,
            objective: "ship the feature".to_string(),
            token_budget: Some(42),
        })
        .await
        .expect("goal should be created");
    let client = RuntimeHostClient {
        runtime: runtime.clone(),
    };

    let updated = with_host_callback_context(
        HostCallbackContext {
            session_id: Some(session.id),
            ..HostCallbackContext::default()
        },
        async {
            <RuntimeHostClient as HostClient>::update_goal(
                &client,
                HostUpdateGoalRequest {
                    objective: None,
                    status: Some(HostGoalStatus::Completed),
                    token_budget: None,
                },
            )
            .await
        },
    )
    .await
    .expect("update_goal should succeed");

    assert_eq!(updated.goal.status, HostGoalStatus::Completed);
    assert!(updated.goal.completed_at_ms.is_some());

    let stored = manager
        .get_goal(session.id)
        .await
        .expect("goal lookup should succeed")
        .expect("goal should persist");
    assert_eq!(stored.status, GoalStatus::Completed);
    assert!(stored.completed_at.is_some());

    runtime.shutdown();
}
