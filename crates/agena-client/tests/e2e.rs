//! End-to-end test: spin up `agena-api-server` against an in-memory
//! `SessionManager`, drive the bus through the publisher, and consume
//! events via the SDK over both REST and WebSocket.

use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agena::{
    agent::Agent,
    config::LoadConfigRequest,
    event::{EventKind, PublishContext},
    message::PartContent,
    permission::PermissionPolicy,
    provider::{OpenAiCompatibleProvider, ProviderRegistry},
    session::{ContextGovernor, ContextPolicy, SessionManager, SessionProcessor},
    tool::ToolExecutor,
};
use agena_api::{
    Scope,
    commands::{
        Command, CommandResult, CreateSessionParams, ResolveWorkspaceParams, SubmitTurnParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, GetWorkspaceParams, ListEventsParams,
        ListMessagesParams, ListSessionsParams, ListWorkspacesParams, Query, QueryResult,
    },
    resource::{PartLoadMode, RunOptions},
    subscribe::SubscribeRequest,
};
use agena_api_server::{AppState, router};
use agena_client::{AgenaClient, WsClient, ws::SubscriptionEvent};
use mockito::ServerGuard;
use sea_orm::Database;

async fn spawn_server() -> (String, String, Arc<SessionManager>) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let registry = ProviderRegistry::new();
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        std::env::temp_dir(),
        Agent::new("client-e2e", PermissionPolicy::allow_all()),
    );
    let manager = Arc::new(SessionManager::new(db.clone(), processor, executor));

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_workspace_root(std::env::temp_dir())
        .with_database_connection(db.clone())
        .build()
        .await
        .expect("runtime build");
    let shared_db = Arc::new(db.clone());
    let state = AppState::new(runtime, Arc::clone(&shared_db)).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager)
}

fn write_temp_config(content: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agena-client-e2e-{suffix}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let path = dir.join("config.toml");
    fs::write(&path, content).expect("config should be written");
    path
}

async fn spawn_server_with_provider() -> (String, String, Arc<SessionManager>, ServerGuard) {
    let mut provider = mockito::Server::new_async().await;
    let _models = provider
        .mock("GET", "/models")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-4o-mini" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _chat = provider
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "id": "chatcmpl_test",
                "choices": [{
                    "message": { "content": "ack" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 1,
                    "total_tokens": 4
                }
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config_path = write_temp_config(&format!(
        r#"
[providers.openai]
kind = "openai_compatible"
base_url = "{base_url}"
default_model = "gpt-4o-mini"
api_key = "test"
"#,
        base_url = provider.url()
    ));
    let workspace_root = config_path
        .parent()
        .expect("config should have parent")
        .to_path_buf();

    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let mut registry = ProviderRegistry::new();
    registry.register(OpenAiCompatibleProvider::new(
        "openai",
        reqwest::Client::new(),
        "test",
        provider.url(),
        "gpt-4o-mini",
    ));
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        Agent::new("client-e2e-provider", PermissionPolicy::allow_all()),
    );
    let manager = Arc::new(SessionManager::new(db.clone(), processor, executor));

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(workspace_root)
        .with_database_connection(db.clone())
        .build()
        .await
        .expect("runtime build");
    let shared_db = Arc::new(db.clone());
    let state = AppState::new(runtime, Arc::clone(&shared_db)).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager, provider)
}

#[tokio::test]
async fn rest_health_via_sdk() {
    let (http_url, _, _) = spawn_server().await;
    let client = AgenaClient::new(http_url).unwrap();
    let health = client.health().await.unwrap();
    assert_eq!(health.status, "ok");
}

#[tokio::test]
async fn rest_list_events_returns_published() {
    let (http_url, _, manager) = spawn_server().await;

    manager
        .event_publisher()
        .publish(
            PublishContext::for_session(1),
            EventKind::PluginEvent(agena::event::PluginEventPayload {
                plugin_id: "test".into(),
                kind_label: "test_event".into(),
                payload: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();

    let client = AgenaClient::new(http_url).unwrap();
    let page = client
        .list_events(ListEventsParams {
            scope: Scope::Global,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].kind.tag_str(), "plugin_event");
}

#[tokio::test]
async fn rest_command_and_query_cover_workspace_session_and_message_routes() {
    let (http_url, _ws_url, _manager, _provider) = spawn_server_with_provider().await;
    let client = AgenaClient::new(http_url).unwrap();

    let workspace = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: std::env::temp_dir().display().to_string(),
            create_if_missing: true,
        }))
        .await
        .unwrap();
    let CommandResult::Workspace(workspace) = workspace else {
        panic!("expected workspace command result");
    };

    let workspaces = client
        .query(Query::ListWorkspaces(ListWorkspacesParams {
            cursor: None,
            limit: Some(20),
            search: None,
            include_session_count: true,
        }))
        .await
        .unwrap();
    let QueryResult::Workspaces(workspaces) = workspaces else {
        panic!("expected workspaces query result");
    };
    assert!(workspaces.items.iter().any(|item| item.id == workspace.id));

    let workspace_result = client
        .query(Query::GetWorkspace(GetWorkspaceParams {
            workspace_id: workspace.id,
        }))
        .await
        .unwrap();
    let QueryResult::Workspace(workspace_detail) = workspace_result else {
        panic!("expected workspace query result");
    };
    assert_eq!(workspace_detail.id, workspace.id);

    let created = client
        .command(Command::CreateSession(CreateSessionParams {
            workspace_id: workspace.id,
            title: "sdk session".to_string(),
            parent_id: None,
        }))
        .await
        .unwrap();
    let CommandResult::Session(session) = created else {
        panic!("expected session command result");
    };

    let sessions = client
        .query(Query::ListSessions(ListSessionsParams {
            cursor: None,
            limit: Some(20),
            workspace_id: Some(workspace.id),
            parent_id: None,
            roots: false,
            search: None,
        }))
        .await
        .unwrap();
    let QueryResult::Sessions(PaginatedResponse { items, .. }) = sessions else {
        panic!("expected sessions query result");
    };
    assert!(items.iter().any(|item| item.id == session.id));

    let get_session = client
        .query(Query::GetSession(GetSessionParams {
            session_id: session.id,
        }))
        .await
        .unwrap();
    let QueryResult::Session(session_detail) = get_session else {
        panic!("expected session detail result");
    };
    assert_eq!(session_detail.id, session.id);

    let turn = client
        .command(Command::SubmitTurn(SubmitTurnParams {
            session_id: session.id,
            options: RunOptions::default(),
            parts: vec![PartContent::text("hello from sdk")],
        }))
        .await
        .unwrap();
    let CommandResult::Execution(execution) = turn else {
        panic!("expected execution result");
    };
    assert_eq!(execution.session.id, session.id);

    let state = client
        .query(Query::GetSessionState(GetSessionParams {
            session_id: session.id,
        }))
        .await
        .unwrap();
    let QueryResult::SessionState(state) = state else {
        panic!("expected session state result");
    };
    assert_eq!(state.session.id, session.id);

    let messages = client
        .query(Query::ListMessages(ListMessagesParams {
            session_id: session.id,
            cursor: None,
            limit: Some(20),
            parts: PartLoadMode::Full,
        }))
        .await
        .unwrap();
    let QueryResult::Messages(PaginatedResponse { items: messages, .. }) = messages else {
        panic!("expected messages query result");
    };
    assert!(!messages.is_empty());
    let message = messages
        .iter()
        .find(|message| message.session_id == session.id)
        .expect("expected a message belonging to the created session")
        .clone();
    assert_eq!(message.session_id, session.id);
    assert!(message.parts.as_ref().is_some_and(|parts| !parts.is_empty()));
}

#[tokio::test]
async fn ws_subscription_delivers_event() {
    let (_, ws_url, manager) = spawn_server().await;

    let client = WsClient::connect(&ws_url).await.unwrap();
    let mut sub = client
        .subscribe(SubscribeRequest {
            scope: Scope::Global,
            kinds: None,
            since_seq_global: None,
        })
        .await
        .unwrap();

    // Give the server a moment to register the subscription before we
    // publish.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    manager
        .event_publisher()
        .publish(
            PublishContext::for_session(2),
            EventKind::RunStarted(agena::event::RunStartedEvent {
                session_id: 2,
                ts_ms: 0,
            }),
        )
        .await
        .unwrap();

    let item = tokio::time::timeout(std::time::Duration::from_millis(500), sub.recv())
        .await
        .expect("ws should deliver event")
        .expect("subscription not closed");
    let SubscriptionEvent::Event(event) = item else {
        panic!("expected event, got lagged");
    };
    assert_eq!(event.kind.tag_str(), "run_started");
    assert_eq!(event.meta.session_id, Some(2));
}
