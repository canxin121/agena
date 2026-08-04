// macOS ld can emit a non-actionable compact-unwind diagnostic for this large
// test binary; keep test output focused on actionable Rust diagnostics.
#![cfg_attr(test, allow(linker_messages))]

//! # agena-api-server
//!
//! Unified transport crate for Agena surfaces. It wires the shared [`agena_api`]
//! protocol and adjacent local protocols over feature-gated transports including
//! HTTP/REST, WebSocket, SSE, Unix-socket IPC, and JSON-RPC app-server entrypoints,
//! all backed by the same Runtime application-service event-stream boundary.
//!
//! ## Layout
//!
//! - [`state`]: shared `AppState` (runtime + manager + bus accessors).
//! - [`dispatch`]: command/query dispatch helpers — used by both REST and WS
//!   so that adding a new operation only requires touching one place.
//! - [`rest`]: HTTP routes that accept JSON bodies and return JSON responses.
//! - [`ws`]: `/api/v1/ws` upgrade handler implementing the
//!   [`agena_api::ws::ClientMessage`] / [`agena_api::ws::ServerMessage`]
//!   protocol with multiplexed subscriptions.
//! - [`sse`]: `/api/v1/events/stream` push-only event stream.
//! - [`ipc`]: optional Unix socket binder reusing the same WS protocol.
//!
//! ## Design notes
//!
//! - The transports never poll the database. They consume Runtime's stable
//!   live-event stream service. Resume from the persisted store happens on
//!   initial subscribe / on `Lagged` recovery via `EventStore::range`.
//! - Commands and queries route through `dispatch::*` so that the WS handler
//!   and REST handler share identical semantics where practical, while the
//!   REST surface keeps returning the plain JSON resources the Studio web UI
//!   already consumes.

pub mod error;
#[cfg(feature = "ipc")]
pub mod ipc;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
#[cfg(feature = "http")]
pub mod rest;
#[cfg(feature = "sse")]
pub mod sse;
pub mod state;
#[cfg(feature = "ws")]
pub mod ws;

pub use state::AppState;

use axum::Router;
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
use axum::routing::get;
#[cfg(feature = "http")]
use axum::{
    extract::DefaultBodyLimit,
    middleware::{self, Next},
    routing::post,
};

/// Increment the per-process HTTP request counter and record the
/// request duration in the latency histogram.
#[cfg(feature = "http")]
async fn count_request(req: axum::extract::Request, next: Next) -> axum::response::Response {
    rest::METRIC_HTTP_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let elapsed_us = start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    rest::record_http_latency(elapsed_us);
    response
}

/// Build the v1 axum router with every transport mounted.
pub fn router(state: AppState) -> Router {
    #[cfg(feature = "http")]
    let router = {
        let router = Router::new();
        router
            .route("/healthz", get(rest::healthz))
            .route("/readyz", get(rest::readyz))
            .route("/metrics", get(rest::metrics))
            .route("/api/v1/health", get(rest::health))
            .route("/api/v1/runtime", get(rest::get_runtime_status))
            .route("/api/v1/usage", get(rest::get_usage_stats))
            .route("/api/v1/runtime/reload", post(rest::reload_runtime))
            .route(
                "/api/v1/runtime/tasks",
                get(rest::list_runtime_background_tasks),
            )
            .route(
                "/api/v1/runtime/tasks/{task_id}/cancel",
                post(rest::cancel_runtime_background_task),
            )
            .route(
                "/api/v1/settings",
                get(rest::get_settings)
                    .put(rest::set_settings)
                    .patch(rest::patch_settings)
                    .delete(rest::delete_settings),
            )
            .route("/api/v1/settings/entries", get(rest::list_settings))
            .route("/api/v1/settings/validate", post(rest::validate_settings))
            .route("/api/v1/model-catalog", get(rest::get_model_catalog))
            .route(
                "/api/v1/model-catalog/lookup",
                post(rest::lookup_model_catalog),
            )
            .route(
                "/api/v1/model-catalog/refresh",
                post(rest::refresh_model_catalog),
            )
            .route("/api/v1/git/status", get(rest::get_git_status))
            .route("/api/v1/snapshots", get(rest::get_snapshot_status))
            .route("/api/v1/git/stage", post(rest::stage_git_changes))
            .route("/api/v1/git/commits", post(rest::create_git_commit))
            .route(
                "/api/v1/git/pull-requests",
                post(rest::create_git_pull_request),
            )
            .route("/api/v1/project/git/init", post(rest::init_git_repository))
            .route("/api/v1/vcs/diff/raw", get(rest::get_vcs_diff_raw))
            .route("/api/v1/memories", get(rest::list_memories))
            .route(
                "/api/v1/memories/{name}",
                get(rest::get_memory)
                    .put(rest::save_memory)
                    .delete(rest::delete_memory),
            )
            .route("/api/v1/plugins", get(rest::list_plugins))
            .route("/api/v1/plugins/ui", get(rest::get_plugin_ui_catalog))
            .route(
                "/api/v1/plugins/tools/changes",
                get(rest::list_plugin_tool_registry_changes),
            )
            .route(
                "/api/v1/plugins/ui/invoke-tool",
                post(rest::invoke_plugin_ui_tool),
            )
            .route("/api/v1/plugins/{plugin_id}", get(rest::get_plugin))
            .route(
                "/api/v1/plugins/{plugin_id}/ui/actions/{action_id}",
                post(rest::run_plugin_ui_action),
            )
            .route(
                "/api/v1/plugins/{plugin_id}/logs",
                get(rest::list_plugin_logs),
            )
            .route(
                "/api/v1/plugins/marketplace/search",
                post(rest::search_marketplace_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/sync",
                post(rest::sync_marketplace_registry),
            )
            .route(
                "/api/v1/plugins/marketplace/installed",
                get(rest::list_marketplace_installed_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/outdated",
                get(rest::list_marketplace_outdated_plugins),
            )
            .route(
                "/api/v1/plugins/marketplace/install",
                post(rest::install_marketplace_plugin),
            )
            .route(
                "/api/v1/plugins/marketplace/uninstall",
                post(rest::uninstall_marketplace_plugin),
            )
            .route(
                "/api/v1/plugins/marketplace/upgrade",
                post(rest::upgrade_marketplace_plugins),
            )
            .route("/api/v1/auth/providers", get(rest::list_auth_providers))
            .route(
                "/api/v1/auth/providers/openai/browser/start",
                post(rest::start_openai_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/browser/finish",
                post(rest::finish_openai_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/device/start",
                post(rest::start_openai_device_auth),
            )
            .route(
                "/api/v1/auth/providers/openai/device/poll",
                post(rest::poll_openai_device_auth),
            )
            .route(
                "/api/v1/auth/providers/gitlab/browser/start",
                post(rest::start_gitlab_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/gitlab/browser/finish",
                post(rest::finish_gitlab_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/github-copilot/device/start",
                post(rest::start_copilot_device_auth),
            )
            .route(
                "/api/v1/auth/providers/github-copilot/device/poll",
                post(rest::poll_copilot_device_auth),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}",
                get(rest::get_auth_provider).delete(rest::delete_auth_provider),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}/api-key",
                axum::routing::put(rest::set_auth_provider_api_key),
            )
            .route(
                "/api/v1/auth/providers/{provider_id}/refresh",
                post(rest::refresh_auth_provider),
            )
            .route("/api/v1/providers", get(rest::list_providers))
            .route(
                "/api/v1/providers/models",
                post(rest::list_provider_adapter_models),
            )
            .route(
                "/api/v1/providers/{provider_id}/models",
                get(rest::list_provider_models).post(rest::list_saved_provider_adapter_models),
            )
            .route(
                "/api/v1/workspaces",
                get(rest::list_workspaces).post(rest::create_workspace),
            )
            .route("/api/v1/workspaces/resolve", post(rest::resolve_workspace))
            .route(
                "/api/v1/workspaces/{workspace_id}",
                get(rest::get_workspace)
                    .put(rest::replace_workspace)
                    .delete(rest::delete_workspace),
            )
            .route(
                "/api/v1/workspaces/{workspace_id}/files",
                get(rest::list_workspace_files),
            )
            .route(
                "/api/v1/workspaces/{workspace_id}/download",
                get(rest::download_workspace_file),
            )
            .route(
                "/api/v1/sessions",
                get(rest::list_sessions).post(rest::create_session),
            )
            .route(
                "/api/v1/sessions/{session_id}",
                get(rest::get_session)
                    .put(rest::replace_session)
                    .delete(rest::delete_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/state",
                get(rest::get_session_state),
            )
            .route(
                "/api/v1/sessions/{session_id}/operations/{activity_id}/detail",
                get(rest::get_operation_detail),
            )
            .route(
                "/api/v1/sessions/{session_id}/permission",
                axum::routing::put(rest::replace_session_permission),
            )
            .route(
                "/api/v1/sessions/{session_id}/events",
                get(rest::list_session_events),
            )
            .route(
                "/api/v1/sessions/{session_id}/events/stream",
                get(rest::stream_session_events),
            )
            .route(
                "/api/v1/sessions/{session_id}/messages",
                post(rest::submit_message).layer(DefaultBodyLimit::max(96 * 1024 * 1024)),
            )
            .route(
                "/api/v1/sessions/{session_id}/continue",
                post(rest::continue_run),
            )
            .route(
                "/api/v1/sessions/{session_id}/compact",
                post(rest::compact_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/fork",
                post(rest::fork_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/cancel",
                post(rest::cancel_run),
            )
            .route(
                "/api/v1/sessions/{session_id}/permission-replies",
                post(rest::reply_permission),
            )
            .route(
                "/api/v1/sessions/{session_id}/user-input-replies",
                post(rest::reply_user_input),
            )
            .route(
                "/api/v1/sessions/{session_id}/rewind",
                post(rest::rewind_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/export",
                get(rest::export_session),
            )
            .route(
                "/api/v1/sessions/import",
                post(rest::import_session).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
            )
            .route(
                "/api/v1/sessions/tree/{root_id}",
                get(rest::list_session_tree),
            )
            .route(
                "/api/v1/permission-rules",
                get(rest::list_permission_rules).post(rest::create_permission_rule),
            )
            .route(
                "/api/v1/permission-rules/{rule_id}",
                get(rest::get_permission_rule)
                    .put(rest::replace_permission_rule)
                    .delete(rest::delete_permission_rule),
            )
            .route(
                "/api/v1/permission-rules/{rule_id}/revoke",
                post(rest::revoke_permission_rule),
            )
            .route("/api/v1/events", get(rest::list_events))
            .route("/plugin-rpc/{plugin_id}", post(rest::plugin_rpc))
            .layer(middleware::from_fn(count_request))
    };

    #[cfg(not(feature = "http"))]
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router.route("/api/v1/events/stream", get(sse::handler));

    router.with_state(state)
}

/// Build the streaming-only transport router for hosts that already mount
/// overlapping REST endpoints via another API surface.
pub fn transport_router(state: AppState) -> Router {
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router.route("/api/v1/events/stream", get(sse::handler));

    router.with_state(state)
}

#[cfg(test)]
mod router_contract_tests {
    use agena_application::Application;
    use agena_runtime::{
        RuntimeBootstrapRequest, RuntimeEventPublishRequest, bootstrap_application_services,
    };
    use axum::body::to_bytes;
    use futures_util::{SinkExt, StreamExt};
    use http::Request;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use tower::ServiceExt;

    use super::{AppState, router};

    fn application_for_test(runtime: &agena_runtime::RuntimeBootstrapResult) -> Application {
        Application::from_composed_runtime_services(runtime.application_services())
            .expect("test runtime composes application repositories")
    }

    #[tokio::test]
    async fn health_route_is_served_by_the_real_api_router() {
        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            tracing_reload_handle: None,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let app = router(AppState::from_application(application_for_test(&runtime)));
        let runtime_app = app.clone();
        let command_app = runtime_app.clone();
        let response = app
            .oneshot(
                Request::get("/api/v1/health")
                    .body(axum::body::Body::empty())
                    .expect("build health request"),
            )
            .await
            .expect("serve health request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read health response");
        let health: agena_api::resource::HealthResponse =
            serde_json::from_slice(&body).expect("decode shared health response");
        assert_eq!(health.status, "ok");
        assert_eq!(health.generation, 1);

        let response = runtime_app
            .oneshot(
                Request::get("/api/v1/runtime")
                    .body(axum::body::Body::empty())
                    .expect("build runtime request"),
            )
            .await
            .expect("serve runtime request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read runtime response");
        let runtime: agena_api::resource::RuntimeStatusResponse =
            serde_json::from_slice(&body).expect("decode shared runtime response");
        assert_eq!(runtime.generation, 1);

        let workspace_path = std::env::temp_dir().join(format!(
            "agena-api-server-contract-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let response = command_app
            .clone()
            .oneshot(
                Request::post("/api/v1/workspaces")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({"path": workspace_path}).to_string(),
                    ))
                    .expect("build create workspace request"),
            )
            .await
            .expect("serve create workspace request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read workspace response");
        let workspace: agena_api::resource::WorkspaceResource =
            serde_json::from_slice(&body).expect("decode shared workspace response");
        assert_eq!(workspace.path, workspace_path);

        let response = command_app
            .oneshot(
                Request::get("/api/v1/workspaces/9223372036854775807")
                    .body(axum::body::Body::empty())
                    .expect("build missing workspace request"),
            )
            .await
            .expect("serve missing workspace request");
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read api error response");
        let error: agena_api::ApiError =
            serde_json::from_slice(&body).expect("decode shared api error response");
        assert_eq!(
            error.problem.category,
            agena_failure::FailureCategory::NotFound
        );
        assert_eq!(error.problem.user.fallback, "The resource was not found.");

        let _ = std::fs::remove_dir_all(workspace_path);
    }

    #[tokio::test]
    async fn websocket_upgrade_serves_shared_hello_and_pong_frames() {
        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            tracing_reload_handle: None,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let event_publisher = runtime
            .application_services()
            .event_publisher
            .expect("access stable websocket event publisher");
        let state = AppState::from_application(application_for_test(&runtime));
        let app = router(state);
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind websocket contract listener");
        let address = listener.local_addr().expect("read listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve websocket contract router");
        });

        let (mut socket, _) = connect_async(format!("ws://{address}/api/v1/ws"))
            .await
            .expect("upgrade websocket contract client");
        let hello = socket
            .next()
            .await
            .expect("receive websocket hello")
            .expect("read websocket hello");
        let Message::Text(hello) = hello else {
            panic!("websocket hello must be a text frame");
        };
        let hello: agena_api::ws::ServerMessage =
            serde_json::from_str(&hello).expect("decode websocket hello");
        assert!(matches!(
            hello,
            agena_api::ws::ServerMessage::Hello {
                protocol_version: agena_api::PROTOCOL_VERSION
            }
        ));

        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Ping {
                    nonce: Some("loopback".into()),
                })
                .expect("encode websocket ping")
                .into(),
            ))
            .await
            .expect("send websocket ping");
        let pong = socket
            .next()
            .await
            .expect("receive websocket pong")
            .expect("read websocket pong");
        let Message::Text(pong) = pong else {
            panic!("websocket pong must be a text frame");
        };
        let pong: agena_api::ws::ServerMessage =
            serde_json::from_str(&pong).expect("decode websocket pong");
        assert!(matches!(
            pong,
            agena_api::ws::ServerMessage::Pong { nonce: Some(nonce) }
                if nonce == "loopback"
        ));

        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Query {
                    id: "health-query".into(),
                    query: agena_api::queries::Query::Health,
                })
                .expect("encode websocket health query")
                .into(),
            ))
            .await
            .expect("send websocket health query");
        let query_result = socket
            .next()
            .await
            .expect("receive websocket query result")
            .expect("read websocket query result");
        let Message::Text(query_result) = query_result else {
            panic!("websocket query result must be a text frame");
        };
        let query_result: agena_api::ws::ServerMessage =
            serde_json::from_str(&query_result).expect("decode websocket query result");
        assert!(matches!(
            query_result,
            agena_api::ws::ServerMessage::QueryResult {
                id,
                result: agena_api::queries::QueryResult::Health(health),
            } if id == "health-query" && health.status == "ok" && health.database_connected
        ));

        let workspace_path = std::env::temp_dir().join(format!(
            "agena-api-server-ws-contract-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Command {
                    id: "create-workspace".into(),
                    command: agena_api::commands::Command::CreateWorkspace(
                        agena_api::commands::CreateWorkspaceParams {
                            path: workspace_path.to_string_lossy().into_owned(),
                        },
                    ),
                })
                .expect("encode websocket create workspace command")
                .into(),
            ))
            .await
            .expect("send websocket create workspace command");
        let command_result = socket
            .next()
            .await
            .expect("receive websocket command result")
            .expect("read websocket command result");
        let Message::Text(command_result) = command_result else {
            panic!("websocket command result must be a text frame");
        };
        let command_result: agena_api::ws::ServerMessage =
            serde_json::from_str(&command_result).expect("decode websocket command result");
        let workspace_id = match command_result {
            agena_api::ws::ServerMessage::CommandResult {
                id,
                result: agena_api::commands::CommandResult::Workspace(workspace),
            } => {
                assert_eq!(id, "create-workspace");
                assert_eq!(workspace.path, workspace_path.to_string_lossy());
                workspace.id
            }
            other => panic!("unexpected websocket command result: {other:?}"),
        };

        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Command {
                    id: "delete-workspace".into(),
                    command: agena_api::commands::Command::DeleteWorkspace(
                        agena_api::commands::DeleteWorkspaceParams { workspace_id },
                    ),
                })
                .expect("encode websocket delete workspace command")
                .into(),
            ))
            .await
            .expect("send websocket delete workspace command");
        let delete_result = socket
            .next()
            .await
            .expect("receive websocket delete result")
            .expect("read websocket delete result");
        let Message::Text(delete_result) = delete_result else {
            panic!("websocket delete result must be a text frame");
        };
        let delete_result: agena_api::ws::ServerMessage =
            serde_json::from_str(&delete_result).expect("decode websocket delete result");
        assert!(matches!(
            delete_result,
            agena_api::ws::ServerMessage::CommandResult {
                id,
                result: agena_api::commands::CommandResult::WorkspaceDeleted { id: deleted_id },
            } if id == "delete-workspace" && deleted_id == workspace_id
        ));

        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Subscribe {
                    id: "global-subscription".into(),
                    request: agena_api::subscribe::SubscribeRequest {
                        scope: agena_api::Scope::Global,
                        kinds: None,
                        since_seq_global: None,
                    },
                })
                .expect("encode websocket subscribe request")
                .into(),
            ))
            .await
            .expect("send websocket subscribe request");
        let subscribed = socket
            .next()
            .await
            .expect("receive websocket subscribe result")
            .expect("read websocket subscribe result");
        let Message::Text(subscribed) = subscribed else {
            panic!("websocket subscribe result must be a text frame");
        };
        let subscribed: agena_api::ws::ServerMessage =
            serde_json::from_str(&subscribed).expect("decode websocket subscribe result");
        assert!(matches!(
            subscribed,
            agena_api::ws::ServerMessage::Subscribed { id }
                if id == "global-subscription"
        ));

        event_publisher
            .publish_event(RuntimeEventPublishRequest::PluginEvent {
                plugin_id: agena_plugin_host::PluginKey::new("contract", "fixture")
                    .expect("build fixture plugin key"),
                kind_label: "contract_event".to_owned(),
                payload: serde_json::json!({"value": 42}),
            })
            .await
            .expect("publish websocket notification fixture event");
        let notification = socket
            .next()
            .await
            .expect("receive websocket notification")
            .expect("read websocket notification");
        let Message::Text(notification) = notification else {
            panic!("websocket notification must be a text frame");
        };
        let notification: agena_api::ws::ServerMessage =
            serde_json::from_str(&notification).expect("decode websocket notification");
        assert!(matches!(
            notification,
            agena_api::ws::ServerMessage::Notification(
                agena_api::notifications::Notification::Event { subscription, event }
            ) if subscription == "global-subscription"
                && event.kind == "plugin_event"
                && event.payload["kind_label"] == "contract_event"
                && event.payload["payload"]["value"] == 42
        ));

        socket
            .send(Message::Text(
                serde_json::to_string(&agena_api::ws::ClientMessage::Unsubscribe {
                    id: "global-subscription".into(),
                })
                .expect("encode websocket unsubscribe request")
                .into(),
            ))
            .await
            .expect("send websocket unsubscribe request");
        let unsubscribed = socket
            .next()
            .await
            .expect("receive websocket unsubscribe result")
            .expect("read websocket unsubscribe result");
        let Message::Text(unsubscribed) = unsubscribed else {
            panic!("websocket unsubscribe result must be a text frame");
        };
        let unsubscribed: agena_api::ws::ServerMessage =
            serde_json::from_str(&unsubscribed).expect("decode websocket unsubscribe result");
        assert!(matches!(
            unsubscribed,
            agena_api::ws::ServerMessage::Unsubscribed { id }
                if id == "global-subscription"
        ));

        let _ = socket.close(None).await;
        server.abort();
        let _ = server.await;
    }
}
