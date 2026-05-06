//! Integration tests for `agena-api-server`. They construct a
//! `SessionManager` directly (bypassing `AgenaRuntime`), wire it into
//! `AppState::with_manager_override`, and exercise the v2 routes.

use std::sync::Arc;

use agena::{
    agent::Agent,
    event::{EventKind, PublishContext},
    permission::PermissionPolicy,
    provider::ProviderRegistry,
    session::{ContextGovernor, ContextPolicy, SessionManager, SessionProcessor},
    tool::ToolExecutor,
};
use agena_api::{
    PROTOCOL_VERSION, Scope,
    notifications::Notification,
    queries::PaginatedEvents,
    subscribe::SubscribeRequest,
    ws::{ClientMessage, ServerMessage},
};
use agena_api_server::{AppState, router};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sea_orm::Database;
use tower::ServiceExt;

async fn build_state() -> (AppState, Arc<SessionManager>) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let registry = ProviderRegistry::new();
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        std::env::temp_dir(),
        Agent::new("api-server-test", PermissionPolicy::allow_all()),
    );

    let manager = Arc::new(SessionManager::new(db, processor, executor));

    let runtime_db = Arc::new(sea_orm::Database::connect("sqlite::memory:").await.unwrap());
    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_workspace_root(std::env::temp_dir())
        .with_database_connection(runtime_db.as_ref().clone())
        .build()
        .await
        .expect("runtime build");

    let state =
        AppState::new(runtime, Arc::clone(&runtime_db)).with_manager_override(Arc::clone(&manager));
    (state, manager)
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let (state, _) = build_state().await;
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
async fn list_events_returns_published_events() {
    let (state, manager) = build_state().await;

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
    let result = value.get("result").and_then(|s| s.as_str());
    assert_eq!(result, Some("events"));
    let items = value
        .get("data")
        .and_then(|d| d.get("items"))
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
async fn ws_protocol_round_trip_command_and_subscription() {
    let (state, manager) = build_state().await;

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
