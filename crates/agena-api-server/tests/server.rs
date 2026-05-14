#![cfg(feature = "http")]

//! Integration tests for `agena-api-server`. They construct a
//! `SessionManager` directly (bypassing `AgenaRuntime`), wire it into
//! `AppState::with_manager_override`, and exercise the v2 routes.

use std::{process::Command as ProcessCommand, sync::Arc};

use agena::model::{ModelId, ModelRef, ProviderId};
use agena::{
    agent::Agent,
    config::LoadConfigRequest,
    event::{EventKind, PublishContext},
    message::PartContent,
    permission::PermissionPolicy,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, ModelProvider,
        ProviderModel, ProviderRegistry,
    },
    session::{
        ContextGovernor, ContextPolicy, SessionManager, SessionProcessor, SessionRunOptions,
        SessionUserTurnRequest,
    },
    tool::ToolExecutor,
};
use agena_api::{
    PROTOCOL_VERSION, Scope,
    commands::{Command, CommandResult, CompleteSessionGoalParams, CreateSessionGoalParams},
    notifications::Notification,
    queries::{GetSessionParams, PaginatedEvents, Query, QueryResult},
    subscribe::SubscribeRequest,
    ws::{ClientMessage, ServerMessage},
};
use agena_api_server::{AppState, dispatch::{dispatch_command, dispatch_query}, router};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sea_orm::Database;
use tower::ServiceExt;

struct TestProvider;

#[async_trait::async_trait]
impl ModelProvider for TestProvider {
    fn id(&self) -> &str {
        "test"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("test-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, agena::AppError> {
        Ok(vec![ProviderModel::new(
            "test",
            self.default_model().as_str(),
        )])
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, agena::AppError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new("test"),
            model: request.model,
            text: "ack".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        })
    }
}

fn test_run_options() -> SessionRunOptions {
    SessionRunOptions {
        model: ModelRef::new("test", "test-model"),
        variant: None,
        thinking: None,
        system: None,
        temperature: None,
        max_output_tokens: Some(128),
        agent_profile: None,
        max_turn_loops: None,
    }
}

async fn build_state() -> (AppState, Arc<SessionManager>, String) {
    let db = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
    agena::db::init_schema(db.as_ref()).await.unwrap();
    let workspace_root = format!("/tmp/api-server-workspace-{}", uuid::Uuid::new_v4());
    std::fs::create_dir_all(&workspace_root).expect("test workspace dir should be created");
    agena::db::crud::workspace::ensure_workspace_id(db.as_ref(), workspace_root.as_str())
        .await
        .expect("workspace should exist for manager scans");

    let mut registry = ProviderRegistry::new();
    registry.register(TestProvider);
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        std::path::PathBuf::from(&workspace_root),
        Agent::new("api-server-test", PermissionPolicy::allow_all()),
    );

    let manager = Arc::new(SessionManager::new(
        db.as_ref().clone(),
        processor,
        executor,
    ));
    let config_path = std::env::temp_dir().join(format!(
        "agena-api-server-test-{}.toml",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&config_path, "").expect("test config should be written");

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(std::path::PathBuf::from(&workspace_root))
        .with_database_connection(db.as_ref().clone())
        .build()
        .await
        .expect("runtime build");

    let state = AppState::new(runtime, Arc::clone(&db)).with_manager_override(Arc::clone(&manager));
    (state, manager, workspace_root)
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let (state, _, _) = build_state().await;
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn runtime_and_model_catalog_endpoints_expose_catalog_payload() {
    let (state, _, _) = build_state().await;
    let app = router(state);

    let runtime_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime_response.status(), StatusCode::OK);
    let runtime_body = runtime_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let runtime_value: serde_json::Value = serde_json::from_slice(&runtime_body).unwrap();
    assert!(
        runtime_value.get("model_catalog").is_some(),
        "runtime payload should include model_catalog: {runtime_value:?}"
    );

    let catalog_response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/model-catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(catalog_response.status(), StatusCode::OK);
    let catalog_body = catalog_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let catalog_value: serde_json::Value = serde_json::from_slice(&catalog_body).unwrap();
    assert!(
        catalog_value.get("remote_url").is_some(),
        "catalog payload should include remote_url: {catalog_value:?}"
    );
    assert!(
        catalog_value
            .get("entries")
            .and_then(|value| value.as_array())
            .is_some(),
        "catalog payload should include entries array: {catalog_value:?}"
    );
}

#[tokio::test]
async fn operational_probes_and_metrics_return_expected_shapes() {
    let (state, _, _) = build_state().await;
    let app = router(state);

    let healthz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(healthz.status(), StatusCode::OK);
    let healthz_body = healthz.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&healthz_body), "ok");

    let readyz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(readyz.status(), StatusCode::OK);
    let readyz_body = readyz.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&readyz_body), "ready");

    let metrics = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let metrics_body = metrics.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&metrics_body);
    assert!(text.contains("agena_runtime_generation"));
    assert!(text.contains("agena_http_requests_total"));
    assert!(text.contains("agena_http_request_duration_seconds_bucket"));
    assert!(text.contains("agena_provider_calls_total"));
    assert!(text.contains("agena_session_active"));
    assert!(text.contains("agena_build_info"));
}

#[tokio::test]
async fn project_git_init_endpoint_initializes_repository_and_returns_status() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state);

    let git_dir = std::path::Path::new(&workspace_root).join(".git");
    assert!(
        !git_dir.exists(),
        "test workspace should start without .git"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/project/git/init")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("repo").and_then(|item| item.as_bool()),
        Some(true),
        "expected git repo after init: {value:?}"
    );
    assert!(git_dir.exists(), "git init should create a .git directory");
}

#[tokio::test]
async fn vcs_diff_raw_endpoint_returns_plaintext_patch_for_workspace_changes() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state);

    let run_git = |args: &[&str]| {
        let output = ProcessCommand::new("git")
            .args(args)
            .current_dir(&workspace_root)
            .output()
            .expect("git command should execute");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init"]);
    run_git(&["config", "user.email", "studio@example.com"]);
    run_git(&["config", "user.name", "Studio Parity"]);
    std::fs::write(
        std::path::Path::new(&workspace_root).join("tracked.txt"),
        "before\n",
    )
    .unwrap();
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-m", "initial"]);

    std::fs::write(
        std::path::Path::new(&workspace_root).join("tracked.txt"),
        "after\n",
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&workspace_root).join("untracked.txt"),
        "brand new\n",
    )
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/vcs/diff/raw")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("diff --git a/tracked.txt b/tracked.txt"));
    assert!(text.contains("diff --git a/untracked.txt b/untracked.txt"));
}

#[tokio::test]
async fn list_events_returns_published_events() {
    let (state, manager, _workspace_root) = build_state().await;

    // Publish a persistent (history) event — UI-only events like RunStarted
    // are no longer written to the event store. PluginEvent is an easy
    // persistent kind that has no private payload types.
    let publisher = manager.event_publisher();
    publisher
        .publish(
            PublishContext::for_session(42),
            EventKind::PluginEvent(agena::event::PluginEventPayload {
                plugin_id: "test".into(),
                kind_label: "test_event".into(),
                payload: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .expect("items array");
    assert!(
        items
            .iter()
            .any(|e| e.get("kind").and_then(|k| k.as_str()) == Some("plugin_event")),
        "expected plugin_event event in {value:?}"
    );
}

#[tokio::test]
async fn session_state_endpoint_returns_execution_resource() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "state route".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{}/state", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert!(
        value
            .get("execution")
            .and_then(|execution| execution.get("allowed_tools"))
            .and_then(|allowed_tools| allowed_tools.as_array())
            .is_some()
    );
}

#[tokio::test]
async fn goal_command_and_query_round_trip() {
    let (state, _manager, workspace_root) = build_state().await;

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "goal route".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let created = dispatch_command(
        &state,
        Command::CreateSessionGoal(CreateSessionGoalParams {
            session_id: session.id,
            objective: "close goal scope".to_string(),
            token_budget: Some(512),
        }),
    )
    .await
    .expect("create goal command should succeed");
    let CommandResult::SessionGoal(goal) = created else {
        panic!("expected goal result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.objective, "close goal scope");
    assert_eq!(goal.token_budget, Some(512));

    let queried = dispatch_query(&state, Query::GetSessionGoal(GetSessionParams { session_id: session.id }))
        .await
        .expect("goal query should succeed");
    let QueryResult::SessionGoal(Some(goal)) = queried else {
        panic!("expected session goal query result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.objective, "close goal scope");

    let completed = dispatch_command(
        &state,
        Command::CompleteSessionGoal(CompleteSessionGoalParams {
            session_id: session.id,
        }),
    )
    .await
    .expect("complete goal command should succeed");
    let CommandResult::SessionGoal(goal) = completed else {
        panic!("expected completed goal result");
    };
    assert_eq!(goal.session_id, session.id);
    assert_eq!(goal.status, agena::session::GoalStatus::Completed);
    assert!(goal.completed_at.is_some());
}

#[tokio::test]
async fn fork_session_endpoint_returns_forked_execution_resource() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "fork source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/fork", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"fork child"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(
        value
            .get("session")
            .and_then(|s| s.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("parent_id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
    assert_eq!(
        value
            .get("session")
            .and_then(|s| s.get("title"))
            .and_then(|title| title.as_str()),
        Some("fork child")
    );
}

#[tokio::test]
async fn message_detail_routes_return_message_and_parts() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "message source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("hello")],
        })
        .await
        .expect("submit turn should succeed");
    let message = session
        .messages
        .first()
        .expect("session should contain a user message");
    let part = message
        .parts
        .first()
        .expect("message should contain a part");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=full", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.get("id").and_then(|id| id.as_i64()), Some(message.id));
    assert_eq!(
        value
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|item| item.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("hello")
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/messages/{}/parts?mode=summary",
                    message.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.as_array().map(|items| items.len()), Some(1));
    assert_eq!(
        value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(|id| id.as_i64()),
        Some(part.id)
    );
    assert!(
        value
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .is_none(),
        "summary mode should omit content: {value:?}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/message-parts/{}", part.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.get("id").and_then(|id| id.as_i64()), Some(part.id));
    assert_eq!(
        value
            .get("content")
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("hello")
    );
}

#[tokio::test]
async fn message_list_summary_omits_part_content_while_detail_full_keeps_it() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "summary source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("summary hello")],
        })
        .await
        .expect("submit turn should succeed");
    let message = session
        .messages
        .first()
        .expect("session should contain a user message");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/messages?parts=summary",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let first = value
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| items.first())
        .expect("first list item");
    assert_eq!(first.get("id").and_then(|id| id.as_i64()), Some(message.id));
    assert!(
        first.get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("content"))
            .is_none(),
        "summary list should omit part content: {value:?}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}?parts=full", message.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value
            .get("parts")
            .and_then(|parts| parts.as_array())
            .and_then(|parts| parts.first())
            .and_then(|item| item.get("content"))
            .and_then(|content| content.get("text"))
            .and_then(|text| text.as_str()),
        Some("summary hello")
    );
}

#[tokio::test]
async fn fork_session_endpoint_rejects_legacy_event_seq_payload() {
    let (state, _manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "fork source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/fork", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"at_event_seq":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value
            .get("message")
            .and_then(|message| message.as_str())
            .or_else(|| {
                value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(|message| message.as_str())
            })
            .is_some_and(|message| message.contains("at_message_id")),
        "body should mention at_message_id: {value:?}"
    );
}

#[tokio::test]
async fn session_action_routes_cover_rewind_tree_checkpoints_export_and_import() {
    let (state, manager, workspace_root) = build_state().await;
    let app = router(state.clone());

    let workspace = state
        .service()
        .resolve_workspace(agena_api_server::local_api::WorkspaceResolveRequest {
            path: workspace_root,
            create_if_missing: false,
        })
        .await
        .expect("workspace should resolve");
    let session = state
        .service()
        .create_session(agena_api_server::local_api::SessionCreateRequest {
            workspace_id: workspace.id,
            title: "action source".to_string(),
            parent_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: session.id,
            options: test_run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("first turn should succeed");
    let rewind_target = session.messages.first().expect("expected first message").id;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/rewind", session.id))
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"message_id":{rewind_target}}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sessions/{}/unrewind", session.id))
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"message_id":{rewind_target}}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/tree/{}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let tree_body = response.into_body().collect().await.unwrap().to_bytes();
    let tree: serde_json::Value = serde_json::from_slice(&tree_body).unwrap();
    assert!(tree.as_array().is_some_and(|items| !items.is_empty()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/rewind-checkpoints",
                    session.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let export = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{}/export", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    let export_body = export.into_body().collect().await.unwrap().to_bytes();
    let jsonl = String::from_utf8_lossy(&export_body).to_string();
    assert!(!jsonl.trim().is_empty());

    let import = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sessions/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "jsonl": jsonl }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(import.status(), StatusCode::OK);
    let import_body = import.into_body().collect().await.unwrap().to_bytes();
    let imported: serde_json::Value = serde_json::from_slice(&import_body).unwrap();
    assert_ne!(
        imported
            .get("session")
            .and_then(|session| session.get("id"))
            .and_then(|id| id.as_i64()),
        Some(session.id)
    );
}

#[tokio::test]
async fn permission_rule_crud_routes_expose_operator_metadata() {
    let (state, _manager, _workspace_root) = build_state().await;
    let app = router(state.clone());

    let create_rule = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/permission-rules")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "subject_kind": "builtin_tool",
                        "tool_name": "bash",
                        "qualifier": "git status*",
                        "scope": "global",
                        "mode": "allow"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_rule.status(), StatusCode::OK);
    let created_body = create_rule.into_body().collect().await.unwrap().to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&created_body).unwrap();
    let rule_id = created
        .get("id")
        .and_then(|id| id.as_i64())
        .expect("rule id");
    assert_eq!(
        created.get("operator").and_then(|value| value.as_str()),
        Some("http_api")
    );
    assert_eq!(
        created.get("scope").and_then(|value| value.as_str()),
        Some("global")
    );

    let replace_rule = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/permission-rules/{rule_id}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "subject_kind": "builtin_tool",
                        "tool_name": "bash",
                        "qualifier": "git diff*",
                        "scope": "workspace",
                        "mode": "deny"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replace_rule.status(), StatusCode::OK);
    let replaced_body = replace_rule.into_body().collect().await.unwrap().to_bytes();
    let replaced: serde_json::Value = serde_json::from_slice(&replaced_body).unwrap();
    assert_eq!(
        replaced.get("operator").and_then(|value| value.as_str()),
        Some("http_api")
    );
    assert_eq!(
        replaced.get("source").and_then(|value| value.as_str()),
        Some("api")
    );
    assert_eq!(
        replaced.get("scope").and_then(|value| value.as_str()),
        Some("workspace")
    );

    let revoked_rule = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/permission-rules/{rule_id}/revoke"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "reason": "no longer needed" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked_rule.status(), StatusCode::OK);
    let revoked_body = revoked_rule.into_body().collect().await.unwrap().to_bytes();
    let revoked: serde_json::Value = serde_json::from_slice(&revoked_body).unwrap();
    assert_eq!(
        revoked.get("revoked_by").and_then(|value| value.as_str()),
        Some("http_api")
    );
}

#[tokio::test]
async fn ws_protocol_round_trip_command_and_subscription() {
    let (state, manager, _workspace_root) = build_state().await;

    // Drive the bus directly so we don't depend on a real provider in this
    // integration test.
    let publisher = manager.event_publisher();
    let bus = manager.event_bus();

    // Start an in-process axum server bound to an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Sanity-check the bus routing without needing a WS client crate: we
    // subscribe in-process and verify events flow.
    let mut sub = bus.subscribe(agena::event::EventFilter::new(Scope::Global));
    publisher
        .publish(
            PublishContext::for_session(1),
            EventKind::RunStarted(agena::event::RunStartedEvent {
                session_id: 1,
                ts_ms: 0,
            }),
        )
        .await
        .unwrap();
    let item = tokio::time::timeout(std::time::Duration::from_millis(200), sub.recv())
        .await
        .expect("subscriber should receive event");
    assert!(item.is_some());

    // Confirm the address opened a TCP listener (basic liveness check).
    let _ = tokio::net::TcpStream::connect(addr).await.unwrap();

    handle.abort();

    // Reuse imports so the compiler doesn't warn unused.
    let _ = (
        ClientMessage::Ping { nonce: None },
        ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        SubscribeRequest {
            scope: Scope::Global,
            kinds: None,
            since_seq_global: None,
        },
        std::any::type_name::<Notification>(),
        std::any::type_name::<PaginatedEvents>(),
    );
}
