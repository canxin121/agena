//! End-to-end test: spin up `agena-api-server` against an in-memory
//! `SessionManager`, drive the bus through the publisher, and consume
//! events via the SDK over both REST and WebSocket.

use std::sync::Arc;

use agena::{
    agent::Agent, event::{EventKind, PublishContext}, permission::PermissionPolicy,
    provider::ProviderRegistry, session::{ContextGovernor, ContextPolicy, SessionManager,
    SessionProcessor}, tool::ToolExecutor,
};
use agena_api::{Scope, queries::ListEventsParams, subscribe::SubscribeRequest};
use agena_api_server::{AppState, router};
use agena_client::{AgenaClient, WsClient, ws::SubscriptionEvent};
use sea_orm::Database;

async fn spawn_server() -> (String, String, Arc<SessionManager>) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let registry = ProviderRegistry::new();
    let processor =
        SessionProcessor::new(Arc::new(registry), ContextGovernor::new(ContextPolicy::default()));
    let executor = ToolExecutor::new(
        std::env::temp_dir(),
        Agent::new("client-e2e", PermissionPolicy::allow_all()),
    );
    let manager = Arc::new(SessionManager::new(db, processor, executor));

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_workspace_root(std::env::temp_dir())
        .with_database_connection(
            sea_orm::Database::connect("sqlite::memory:").await.unwrap(),
        )
        .build()
        .await
        .expect("runtime build");
    let state = AppState::new(runtime).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager)
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
