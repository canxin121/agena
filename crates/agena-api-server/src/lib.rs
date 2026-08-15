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
//! - [`sse`]: `/api/v1/changes/stream` push-only part-patch/signal stream.
//! - [`ipc`]: optional Unix socket binder reusing the same WS protocol.
//!
//! ## Design notes
//!
//! - The transports never poll the database. They consume Runtime's stable
//!   live-change surfaces. Catch-up is an explicit current parts snapshot;
//!   live notifications themselves are never persisted or replayed.
//! - Commands and queries route through `dispatch::*` so that the WS handler
//!   and REST handler share identical semantics where practical, while the
//!   REST surface keeps returning the plain JSON resources the Studio web UI
//!   already consumes.

pub mod dispatch;
pub mod error;
#[cfg(feature = "ipc")]
pub mod ipc;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
mod live;
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
    routing::{delete, post, put},
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
            .route(
                "/api/v1/settings/layers/{layer}",
                get(rest::get_layer_settings)
                    .put(rest::set_layer_settings)
                    .delete(rest::delete_layer_settings),
            )
            .route("/api/v1/settings/entries", get(rest::list_settings))
            .route("/api/v1/settings/validate", post(rest::validate_settings))
            .route("/api/v1/config/resolved", get(rest::get_resolved_config))
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
                "/api/v1/mcp/credentials/{server}/bearer",
                put(rest::set_mcp_bearer_credential).delete(rest::delete_mcp_bearer_credential),
            )
            .route("/api/v1/mcp/oauth/start", post(rest::start_mcp_oauth))
            .route("/api/v1/mcp/oauth/finish", post(rest::finish_mcp_oauth))
            .route(
                "/api/v1/mcp/oauth/{server}",
                delete(rest::delete_mcp_oauth_credential),
            )
            .route("/api/v1/operator/tools", get(rest::list_operator_tools))
            .route(
                "/api/v1/operator/tools/invoke",
                post(rest::invoke_operator_tool),
            )
            .route("/api/v1/memories/overview", get(rest::get_memory_overview))
            .route("/api/v1/memories/index", post(rest::ensure_memory_index))
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
                get(rest::list_workspace_files).post(rest::upload_workspace_file),
            )
            .route(
                "/api/v1/workspaces/{workspace_id}/download",
                get(rest::download_workspace_file),
            )
            .route(
                "/api/v1/sessions",
                get(rest::list_sessions).post(rest::create_session),
            )
            .route("/api/v1/sessions/overview", get(rest::session_overview))
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
                "/api/v1/sessions/{session_id}/cost",
                get(rest::get_session_cost),
            )
            .route(
                "/api/v1/sessions/{session_id}/selection",
                axum::routing::put(rest::replace_session_selection),
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
                "/api/v1/sessions/{session_id}/parts",
                get(rest::list_session_parts),
            )
            .route(
                "/api/v1/sessions/{session_id}/changes/stream",
                get(rest::stream_session_changes),
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
                "/api/v1/sessions/{session_id}/interactive/{request_id}/present",
                post(rest::mark_interactive_request_presented),
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
            .route("/api/v1/activities", get(rest::list_activities))
            .route(
                "/api/v1/activities/clear-finished",
                post(rest::clear_finished_activities),
            )
            .route("/api/v1/activities/{activity_id}", get(rest::get_activity))
            .route(
                "/api/v1/activities/{activity_id}/logs",
                get(rest::get_activity_logs),
            )
            .route(
                "/api/v1/activities/{activity_id}/stop",
                post(rest::stop_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/pause",
                post(rest::pause_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/resume",
                post(rest::resume_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/delete",
                post(rest::delete_activity),
            )
            .route(
                "/api/v1/activities/{activity_id}/dismiss",
                post(rest::dismiss_activity),
            )
            .route("/api/v1/notifications", get(rest::list_notifications))
            .route(
                "/api/v1/notifications/{notification_id}/dismiss",
                post(rest::dismiss_notification),
            )
            .route(
                "/api/v1/notifications/{notification_id}/actions/{action_id}",
                post(rest::resolve_notification_action),
            )
            .route("/plugin-rpc/{plugin_id}", post(rest::plugin_rpc))
            .layer(middleware::from_fn(count_request))
    };

    #[cfg(not(feature = "http"))]
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router
        .route("/api/v1/changes/stream", get(sse::handler))
        .route(
            "/api/v1/notifications/stream",
            get(sse::notifications_stream),
        );

    router.with_state(state)
}

/// Build the streaming-only transport router for hosts that already mount
/// overlapping REST endpoints via another API surface.
pub fn transport_router(state: AppState) -> Router {
    let router = Router::new();

    #[cfg(feature = "ws")]
    let router = router.route("/api/v1/ws", get(ws::handler));

    #[cfg(feature = "sse")]
    let router = router
        .route("/api/v1/changes/stream", get(sse::handler))
        .route(
            "/api/v1/notifications/stream",
            get(sse::notifications_stream),
        );

    router.with_state(state)
}

#[cfg(test)]
mod router_contract_tests {
    use std::collections::{BTreeMap, VecDeque};

    use agena_api::{
        commands::{
            Command, CommandResult, ReplyPermissionParams, ReplyUserInputParams,
            ResolveWorkspaceParams, SubmitRunParams,
        },
        resource::{
            PermissionActionResource, PermissionReply, PermissionReplyKind, RunOptions,
            SessionExecutionResource, SessionState, UserInputReply, UserInputReplyKind,
        },
    };
    use agena_application::Application;
    use agena_client::AgenaClient;
    use agena_notification::NotificationService;
    use agena_runtime::{RuntimeBootstrapRequest, bootstrap_application_services};
    use axum::body::to_bytes;
    use futures_util::{SinkExt, StreamExt};
    use http::Request;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
    };
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
        let application = application_for_test(&runtime);
        let session_store = application
            .session_store_facade()
            .expect("test application exposes session store");
        let app = router(AppState::from_application(application));
        let runtime_app = app.clone();
        let command_app = runtime_app.clone();
        let config_app = command_app.clone();
        let settings_app = command_app.clone();
        let mcp_auth_app = command_app.clone();
        let memory_app = command_app.clone();
        let operator_app = command_app.clone();
        let overview_app = command_app.clone();
        let list_app = command_app.clone();
        let cost_app = command_app.clone();
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
        let server = health
            .server
            .expect("health identifies the server");
        assert_eq!(server.pid, std::process::id());
        assert_eq!(server.protocol_version, agena_api::PROTOCOL_VERSION);

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

        let response = config_app
            .oneshot(
                Request::get("/api/v1/config/resolved")
                    .body(axum::body::Body::empty())
                    .expect("build resolved config request"),
            )
            .await
            .expect("serve resolved config request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read resolved config response");
        let config: serde_json::Value =
            serde_json::from_slice(&body).expect("decode resolved config response");
        assert!(config.get("config").is_some());
        assert!(config.get("meta").is_some());

        let response = settings_app
            .oneshot(
                Request::put("/api/v1/settings/layers/workspace")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({
                            "path": "plugins.list.\"agena.mcp\"",
                            "value": {
                                "enabled": true,
                                "package": {"kind": "static"},
                                "config": {}
                            },
                            "dry_run": true,
                            "validate": true,
                            "reload": false
                        })
                        .to_string(),
                    ))
                    .expect("build workspace settings request"),
            )
            .await
            .expect("serve workspace settings request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read workspace settings response");
        let settings: serde_json::Value =
            serde_json::from_slice(&body).expect("decode workspace settings response");
        assert_eq!(
            settings.get("dry_run"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            settings.get("reload_requested"),
            Some(&serde_json::Value::Bool(false))
        );

        let response = mcp_auth_app
            .oneshot(
                Request::put("/api/v1/mcp/credentials/%20/bearer")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({
                            "token": "server-secret-must-not-echo",
                            "store": "keyring"
                        })
                        .to_string(),
                    ))
                    .expect("build rejected MCP credential request"),
            )
            .await
            .expect("serve rejected MCP credential request");
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read rejected MCP credential response");
        let public_error = String::from_utf8(body.to_vec()).expect("UTF-8 API error");
        assert!(!public_error.contains("server-secret-must-not-echo"));

        let response = memory_app
            .oneshot(
                Request::get("/api/v1/memories/overview")
                    .body(axum::body::Body::empty())
                    .expect("build memory overview request"),
            )
            .await
            .expect("serve memory overview request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read memory overview response");
        let memory: serde_json::Value =
            serde_json::from_slice(&body).expect("decode memory overview response");
        assert_eq!(
            memory
                .get("workspace_root")
                .and_then(serde_json::Value::as_str),
            Some(std::env::temp_dir().to_string_lossy().as_ref())
        );
        assert!(memory.get("directory").is_some());
        assert!(memory.get("items").is_some_and(serde_json::Value::is_array));

        let response = operator_app
            .oneshot(
                Request::get("/api/v1/operator/tools")
                    .body(axum::body::Body::empty())
                    .expect("build operator tools request"),
            )
            .await
            .expect("serve operator tools request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read operator tools response");
        let tools: Vec<agena_application::dto::OperatorToolResource> =
            serde_json::from_slice(&body).expect("decode operator tools response");
        assert!(tools.iter().any(|tool| tool.name == "fs.apply_patch"));

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

        let ready_session = session_store
            .create_session(agena_storage::store::NewSession {
                workspace_id: workspace.id,
                parent_id: None,
                relation_kind: agena_domain::SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "recent session".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create recent session");
        let running_session = session_store
            .create_session(agena_storage::store::NewSession {
                workspace_id: workspace.id,
                parent_id: None,
                relation_kind: agena_domain::SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "running session".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create running session");
        session_store
            .submit_user_run(
                running_session.id,
                "overview-contract-test",
                vec![agena_storage::store::NewPart::pending(
                    "text",
                    agena_storage::store::PartRole::User,
                    serde_json::json!({"text": "keep running"}),
                )],
                None,
            )
            .await
            .expect("start running session");

        let response = overview_app
            .oneshot(
                Request::get(format!(
                    "/api/v1/sessions/overview?workspace_id={}&recent_limit=10",
                    workspace.id
                ))
                .body(axum::body::Body::empty())
                .expect("build session overview request"),
            )
            .await
            .expect("serve session overview request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read session overview response");
        let overview: agena_api::resource::SessionOverviewResource =
            serde_json::from_slice(&body).expect("decode shared session overview response");
        assert!(overview.attention.is_empty());
        assert_eq!(
            overview
                .running
                .iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![running_session.id]
        );
        assert_eq!(
            overview
                .recent
                .iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![ready_session.id]
        );

        // Cursor pagination is nested through two flattened query DTOs. Keep
        // a real HTTP contract for numeric form values so clients can safely
        // send `limit` instead of being rejected during query extraction.
        let response = list_app
            .oneshot(
                Request::get(format!(
                    "/api/v1/sessions?workspace_id={}&limit=1&exclude_subagents=true",
                    workspace.id
                ))
                .body(axum::body::Body::empty())
                .expect("build paginated session request"),
            )
            .await
            .expect("serve paginated session request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read paginated session response");
        let page: agena_api::pagination::PaginatedResponse<agena_api::resource::SessionResource> =
            serde_json::from_slice(&body).expect("decode shared session page");
        assert_eq!(page.items.len(), 1);
        assert!(page.page.has_more);

        let response = cost_app
            .oneshot(
                Request::get(format!("/api/v1/sessions/{}/cost", ready_session.id))
                    .body(axum::body::Body::empty())
                    .expect("build session cost request"),
            )
            .await
            .expect("serve session cost request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read session cost response");
        let cost: agena_domain::SessionCostSummary =
            serde_json::from_slice(&body).expect("decode session cost response");
        assert!(cost.is_empty());

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

    // Runs on a multi-thread runtime like the real server. The session-state
    // route assembles the system prompt, which dispatches tool definitions
    // through the plugin host; the plugin host blocks on the ambient runtime
    // handle, and a current-thread runtime would deadlock there.
    #[tokio::test(flavor = "multi_thread")]
    async fn mark_interactive_request_presented_route_rejects_unknown_requests() {
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

        let workspace_path = std::env::temp_dir().join(format!(
            "agena-api-server-present-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let response = app
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

        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/sessions")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({
                            "workspace_id": workspace.id,
                            "title": "present contract session"
                        })
                        .to_string(),
                    ))
                    .expect("build create session request"),
            )
            .await
            .expect("serve create session request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read session response");
        let session: agena_api::resource::SessionResource =
            serde_json::from_slice(&body).expect("decode shared session response");

        // The route is wired and maps the runtime command error through the
        // standard failure contract. A request id that matches no pending
        // user-input part is rejected by the real command path.
        let response = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/v1/sessions/{}/interactive/host-input:999999:1:0/present",
                    session.id
                ))
                .body(axum::body::Body::empty())
                .expect("build present request"),
            )
            .await
            .expect("serve present request");
        assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read present error body");
        let error: agena_api::ApiError =
            serde_json::from_slice(&body).expect("decode shared api error response");
        assert_eq!(
            error.problem.category,
            agena_failure::FailureCategory::Internal
        );

        // A session with no pending requests reports an empty interactive list.
        let response = app
            .oneshot(
                Request::get(format!("/api/v1/sessions/{}/state", session.id))
                    .body(axum::body::Body::empty())
                    .expect("build session state request"),
            )
            .await
            .expect("serve session state request");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read session state body");
        let state: agena_application::dto::SessionExecutionResource =
            serde_json::from_slice(&body).expect("decode shared session execution resource");
        assert!(state.pending_interactive_requests.is_empty());

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
        let services = runtime.application_services();
        let session_store = services
            .session_store
            .expect("access websocket session store facade");
        let workspace_repository = services
            .repositories
            .expect("access websocket repositories")
            .workspace;
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
            } if id == "health-query"
                && health.status == "ok"
                && health.database_connected
                && health.server.is_some()
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

        let live_workspace_id = workspace_repository
            .ensure_id(workspace_path.to_string_lossy().as_ref())
            .await
            .expect("create workspace for live patch");
        let live_session = session_store
            .create_session(agena_storage::store::NewSession {
                workspace_id: live_workspace_id,
                parent_id: None,
                relation_kind: agena_domain::SessionRelationKind::Root,
                cutoff_part_id: None,
                title: "websocket live patch".to_owned(),
                task_id: None,
                config_json: None,
                provider_anchors_json: None,
            })
            .await
            .expect("create session patch fixture");
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
                agena_api::notifications::Notification::SessionChanged { subscription, change }
            ) if subscription == "global-subscription"
                && matches!(*change, agena_api::live::SessionChangeResource::SessionMetaUpdated {
                    session_id,
                    ref title,
                    ..
                } if session_id == live_session.id && title == "websocket live patch")
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

    #[tokio::test]
    async fn notifications_rest_contract_lists_dismisses_and_resolves_actions() {
        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            tracing_reload_handle: None,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let state = AppState::from_application(application_for_test(&runtime));
        let store = state.notifications().clone();
        let emitted = store
            .emit(agena_notification::service::EmitNotificationRequest {
                kind: agena_notification::model::NotificationKind::Notice {
                    code: "contract".into(),
                },
                severity: agena_notification::model::NotificationSeverity::Warning,
                scope: agena_notification::model::NotificationScope::Global,
                source: agena_notification::model::NotificationSource::App,
                surface: None,
                summary: "contract summary".into(),
                detail: Some("contract detail".into()),
                control: agena_notification::model::NotificationControl::Dismiss,
                actions: vec![agena_notification::model::NotificationAction {
                    id: "go".into(),
                    label: "Go".into(),
                    target: agena_notification::model::ActionTarget::Navigate {
                        route: "/settings".into(),
                    },
                }],
                priority: 0,
                dedup_key: None,
                ttl_ms: None,
            })
            .await
            .expect("emit contract notification");

        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/v1/notifications")
                    .body(axum::body::Body::empty())
                    .expect("build list notifications request"),
            )
            .await
            .expect("serve list notifications");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read list notifications body");
        let page: agena_api::pagination::PaginatedResponse<
            agena_api::resource::NotificationResource,
        > = serde_json::from_slice(&body).expect("decode notification page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].summary, "contract summary");
        assert_eq!(
            page.items[0].severity,
            agena_notification::model::NotificationSeverity::Warning
        );
        assert_eq!(
            page.items[0].actions[0].target,
            agena_api::resource::NotificationActionTargetResource::Navigate {
                route: "/settings".into()
            }
        );

        let response = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/notifications/{}/dismiss", emitted.id))
                    .body(axum::body::Body::empty())
                    .expect("build dismiss request"),
            )
            .await
            .expect("serve dismiss notification");
        assert_eq!(response.status(), http::StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(
                Request::get("/api/v1/notifications")
                    .body(axum::body::Body::empty())
                    .expect("build active list request"),
            )
            .await
            .expect("serve active list");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read active list body");
        let page: agena_api::pagination::PaginatedResponse<
            agena_api::resource::NotificationResource,
        > = serde_json::from_slice(&body).expect("decode active page");
        assert!(page.items.is_empty());

        let response = app
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/notifications/{}/actions/go", emitted.id))
                    .body(axum::body::Body::empty())
                    .expect("build resolve action request"),
            )
            .await
            .expect("serve resolve action");
        assert_eq!(response.status(), http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read resolve action body");
        let target: agena_api::resource::NotificationActionTargetResource =
            serde_json::from_slice(&body).expect("decode resolved action target");
        assert_eq!(
            target,
            agena_api::resource::NotificationActionTargetResource::Navigate {
                route: "/settings".into()
            }
        );

        let response = app
            .oneshot(
                Request::post("/api/v1/notifications/missing/dismiss")
                    .body(axum::body::Body::empty())
                    .expect("build missing dismiss request"),
            )
            .await
            .expect("serve missing dismiss");
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn notifications_sse_contract_streams_replay_then_resumed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            tracing_reload_handle: None,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let state = AppState::from_application(application_for_test(&runtime));
        let store = state.notifications().clone();
        store
            .emit(agena_notification::service::EmitNotificationRequest {
                kind: agena_notification::model::NotificationKind::Notice { code: "sse".into() },
                severity: agena_notification::model::NotificationSeverity::Info,
                scope: agena_notification::model::NotificationScope::Global,
                source: agena_notification::model::NotificationSource::Runtime,
                surface: None,
                summary: "sse summary".into(),
                detail: None,
                control: agena_notification::model::NotificationControl::Dismiss,
                actions: Vec::new(),
                priority: 0,
                dedup_key: None,
                ttl_ms: None,
            })
            .await
            .expect("emit sse fixture notification");

        let app = router(state);
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind notification sse listener");
        let address = listener
            .local_addr()
            .expect("read notification sse address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve notification sse router");
        });

        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect notification sse stream");
        stream
            .write_all(
                b"GET /api/v1/notifications/stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write notification sse request");

        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for notification SSE events: {}",
                String::from_utf8_lossy(&received)
            );
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(1), stream.read(&mut chunk))
                    .await
                    .expect("read notification sse chunk")
                    .expect("notification sse stream read");
            if read == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&received);
            if text.contains("event: notification") && text.contains("event: resumed") {
                break;
            }
        }

        let text = String::from_utf8_lossy(&received);
        assert!(
            text.contains("event: notification"),
            "missing notification event: {text}"
        );
        assert!(
            text.contains("sse summary"),
            "missing fixture summary: {text}"
        );
        assert!(
            text.contains("event: resumed"),
            "missing resumed event: {text}"
        );
        server.abort();
    }

    struct FakeProviderPlan {
        events: Vec<serde_json::Value>,
        release: Option<oneshot::Receiver<()>>,
    }

    struct TestServer {
        _workspace: tempfile::TempDir,
        runtime: agena_runtime::RuntimeBootstrapResult,
        server: tokio::task::JoinHandle<()>,
        url: String,
        workspace_id: i64,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.server.abort();
            self.runtime.shutdown();
        }
    }

    async fn spawn_fake_responses_provider(
        plans: Vec<FakeProviderPlan>,
    ) -> (
        String,
        mpsc::UnboundedReceiver<serde_json::Value>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fake provider");
        let address = listener.local_addr().expect("fake provider address");
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let mut plans = VecDeque::from(plans);
            while !plans.is_empty() {
                let (mut stream, _) = listener.accept().await.expect("accept provider request");
                let (request_line, request) = read_http_json_request(&mut stream).await;
                if !request_line.starts_with("POST /v1/responses ") {
                    let body = serde_json::json!({
                        "object": "list",
                        "data": [{"id": "fake-model", "object": "model"}]
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write fake provider discovery response");
                    let _ = stream.shutdown().await;
                    continue;
                }
                let mut plan = plans.pop_front().expect("provider response plan");
                request_tx
                    .send(request)
                    .expect("record fake provider request");
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
                    )
                    .await
                    .expect("write fake provider headers");
                if let Some(release) = plan.release.take() {
                    let _ = release.await;
                }
                for event in plan.events {
                    let frame = format!("data: {}\n\n", event);
                    if stream.write_all(frame.as_bytes()).await.is_err() {
                        break;
                    }
                }
                let _ = stream.shutdown().await;
            }
        });
        (format!("http://{address}/v1"), request_rx, server)
    }

    async fn read_http_json_request(stream: &mut TcpStream) -> (String, serde_json::Value) {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read provider request");
            assert!(read > 0, "provider request closed before headers");
            request.extend_from_slice(&buffer[..read]);
            assert!(
                request.len() <= 2 * 1024 * 1024,
                "provider request too large"
            );
            let Some(header_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let request_line = headers.lines().next().unwrap_or_default().trim().to_owned();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).await.expect("read provider body");
                assert!(read > 0, "provider request closed before body");
                request.extend_from_slice(&buffer[..read]);
            }
            let body = if content_length == 0 {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&request[header_end..header_end + content_length])
                    .expect("decode provider request JSON")
            };
            return (request_line, body);
        }
    }

    fn terminal_response_events(text: &str, response_id: &str) -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({
                "type": "response.output_text.delta",
                "delta": text,
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": response_id,
                    "status": "completed",
                    "usage": {
                        "input_tokens": 4,
                        "output_tokens": 2,
                        "total_tokens": 6
                    }
                }
            }),
        ]
    }

    fn ask_user_response_events() -> Vec<serde_json::Value> {
        let arguments = serde_json::json!({
            "tool": "interaction.ask",
            "input": {
                "title": "Choose a color",
                "questions": [{
                    "header": "Color",
                    "question": "Which color should the run use?",
                    "options": [
                        {"label": "Blue", "description": "Use blue"},
                        {"label": "Green", "description": "Use green"}
                    ]
                }]
            }
        })
        .to_string();
        vec![
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_ask_1",
                    "call_id": "call_ask_1",
                    "name": "tools_call",
                    "arguments": ""
                }
            }),
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_ask_1",
                    "call_id": "call_ask_1",
                    "name": "tools_call",
                    "arguments": arguments
                }
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_ask_1",
                    "status": "completed",
                    "stop_reason": "tool_calls",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8
                    }
                }
            }),
        ]
    }

    fn read_workspace_response_events() -> Vec<serde_json::Value> {
        let arguments = serde_json::json!({
            "tool": "fs.read",
            "input": {
                "path": "permission-fixture.txt"
            }
        })
        .to_string();
        vec![
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_permission_1",
                    "call_id": "call_permission_1",
                    "name": "tools_call",
                    "arguments": ""
                }
            }),
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_permission_1",
                    "call_id": "call_permission_1",
                    "name": "tools_call",
                    "arguments": arguments
                }
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_permission_1",
                    "status": "completed",
                    "stop_reason": "tool_calls",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8
                    }
                }
            }),
        ]
    }

    async fn start_test_server(provider_base_url: &str) -> TestServer {
        let workspace = tempfile::tempdir().expect("create server workspace");
        let project_config_dir = workspace.path().join(".agena");
        std::fs::create_dir_all(&project_config_dir).expect("create project config directory");
        let config = serde_json::json!({
            "permission": {
                "path": {
                    "workspace": {
                        "read": "ask"
                    }
                }
            },
            "providers": {
                "default": "fake",
                "default_selection": {
                    "provider": "fake",
                    "adapter": "openai_responses",
                    "model": "fake-model"
                },
                "fake": {
                    "defaults": {
                        "adapter": "openai_responses",
                        "model": "fake-model"
                    },
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": provider_base_url,
                        "api_key": {"kind": "inline", "value": "fake-test-key"}
                    },
                    "adapters": {
                        "openai_responses": {
                            "enabled": true,
                            "models": {
                                "fake-model": {
                                    "agena_tools": {"mode": "provider_protocol"}
                                }
                            }
                        }
                    }
                }
            }
        });
        std::fs::write(
            project_config_dir.join("agena.json"),
            serde_json::to_vec_pretty(&config).expect("encode project config"),
        )
        .expect("write project config");
        std::fs::write(
            workspace.path().join("permission-fixture.txt"),
            "permission fixture\n",
        )
        .expect("write permission fixture");

        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(workspace.path().to_path_buf()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            tracing_reload_handle: None,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build fake-provider runtime");
        let runtime_config = runtime
            .application_services()
            .configuration
            .runtime_configuration()
            .expect("read isolated runtime configuration");
        assert!(
            runtime_config.project_config_found,
            "fake-provider project config must be loaded before any run"
        );
        assert_eq!(
            runtime_config.default_provider.as_deref(),
            Some("fake"),
            "test isolation failed: refusing to submit through a non-fake provider"
        );
        let app = router(AppState::from_application(application_for_test(&runtime)));
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind server");
        let address = listener.local_addr().expect("server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve server");
        });
        let url = format!("http://{address}");
        let client = AgenaClient::new(url.as_str()).expect("build setup client");
        let workspace_result = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: workspace.path().to_string_lossy().into_owned(),
                create_if_missing: true,
            }))
            .await
            .expect("resolve test workspace");
        let CommandResult::Workspace(workspace_resource) = workspace_result else {
            panic!("server returned the wrong workspace result");
        };
        TestServer {
            _workspace: workspace,
            runtime,
            server,
            url,
            workspace_id: workspace_resource.id,
        }
    }

    async fn submit_test_run(
        client: &AgenaClient,
        workspace_id: i64,
        prompt: &str,
    ) -> SessionExecutionResource {
        let session = client
            .create_session(workspace_id, prompt, None)
            .await
            .expect("create test session");
        client
            .submit_message(SubmitRunParams {
                session_id: session.id,
                options: RunOptions::default(),
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: prompt.to_owned(),
                }]),
            })
            .await
            .expect("submit test run")
    }

    async fn wait_for_execution(
        client: &AgenaClient,
        session_id: i64,
        predicate: impl Fn(&SessionExecutionResource) -> bool,
    ) -> SessionExecutionResource {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let execution = client
                .get_session_state(session_id)
                .await
                .expect("read session execution");
            if predicate(&execution) {
                return execution;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for session {session_id}: state={:?}, workflow={:?}, pending={}, parts={:#?}",
                execution.session.state,
                execution.workflow_state,
                execution.pending_interactive_requests.len(),
                execution.parts,
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    fn execution_text(execution: &SessionExecutionResource) -> String {
        execution
            .parts
            .iter()
            .filter_map(|part| part.content.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("")
    }

    #[tokio::test]
    async fn session_sse_lag_converges_through_an_authoritative_snapshot() {
        let (provider_url, _requests, provider) = spawn_fake_responses_provider(Vec::new()).await;
        let server = start_test_server(provider_url.as_str()).await;
        let client = AgenaClient::new(server.url.as_str()).expect("build snapshot client");
        let session = client
            .create_session(server.workspace_id, "sse lag fixture", None)
            .await
            .expect("create SSE lag session");
        let session_id = session.id;

        // The test-only query controls keep the real HTTP handler subscribed
        // while its one-slot live queue is deliberately flooded before the
        // initial snapshot is read. Production builds expose neither field.
        let stream_url = format!(
            "{}/api/v1/sessions/{session_id}/changes/stream?since_version=0&test_queue_capacity=1&test_snapshot_delay_ms=3000",
            server.url
        );
        let http = reqwest::Client::new();
        let stream_http = http.clone();
        let response_task = tokio::spawn(async move {
            stream_http
                .get(stream_url)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .send()
                .await
                .expect("open delayed session SSE response")
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let update_url = format!("{}/api/v1/sessions/{session_id}", server.url);
        let updates = (0..24).map(|index| {
            let http = http.clone();
            let update_url = update_url.clone();
            async move {
                let response = http
                    .put(update_url)
                    .json(&serde_json::json!({"title": format!("queued-title-{index}")}))
                    .send()
                    .await
                    .expect("send queued session update");
                assert!(
                    response.status().is_success(),
                    "queued session update failed: {}",
                    response.status()
                );
                response
                    .bytes()
                    .await
                    .expect("consume queued update response");
            }
        });
        futures_util::future::join_all(updates).await;
        let final_title = "snapshot-converged-title";
        let final_update = http
            .put(update_url)
            .json(&serde_json::json!({"title": final_title}))
            .send()
            .await
            .expect("send final session update");
        assert!(final_update.status().is_success());
        final_update
            .bytes()
            .await
            .expect("consume final update response");

        let response = tokio::time::timeout(std::time::Duration::from_secs(6), response_task)
            .await
            .expect("delayed SSE handler returns response")
            .expect("SSE response task joins");
        assert!(response.status().is_success());
        let mut chunks = response.bytes_stream();
        let mut received = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for lagged SSE frame: {}",
                String::from_utf8_lossy(&received)
            );
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(1), chunks.next())
                .await
                .expect("read SSE chunk before timeout")
                .expect("session SSE remains open")
                .expect("read session SSE bytes");
            received.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&received).contains("event: lagged") {
                break;
            }
        }

        let text = String::from_utf8(received).expect("session SSE is UTF-8");
        let normalized = text.replace("\r\n", "\n");
        let event_data = |event_name: &str| {
            normalized
                .split("\n\n")
                .filter_map(|block| {
                    let mut event = None;
                    let mut data = None;
                    for line in block.lines() {
                        if let Some(value) = line.strip_prefix("event: ") {
                            event = Some(value);
                        } else if let Some(value) = line.strip_prefix("data: ") {
                            data = Some(value);
                        }
                    }
                    (event == Some(event_name))
                        .then_some(data?)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        };
        let snapshots = event_data("session_snapshot");
        let changes = event_data("session_change");
        let lagged = event_data("lagged");
        assert_eq!(
            snapshots.len(),
            1,
            "one initial snapshot is required: {text}"
        );
        assert!(
            !changes.is_empty(),
            "the queued pre-snapshot patch must drain: {text}"
        );
        assert_eq!(
            lagged.len(),
            1,
            "one coalesced lag signal is required: {text}"
        );
        let skipped = lagged[0]
            .parse::<u64>()
            .expect("lagged payload is a skipped count");
        assert!(skipped > 0);

        let snapshot: agena_api::live::SessionPartsResource =
            serde_json::from_str(&snapshots[0]).expect("decode SSE session snapshot");
        let queued_change: agena_api::live::SessionChangeResource =
            serde_json::from_str(&changes[0]).expect("decode queued session change");
        let queued_version = match queued_change {
            agena_api::live::SessionChangeResource::SessionMetaUpdated { version, .. } => version,
            other => panic!("rename flood must yield a metadata patch, got {other:?}"),
        };
        assert!(
            queued_version < snapshot.version,
            "subscribe-before-snapshot may queue an older patch, which revision guards must ignore"
        );

        let converged = client
            .get_session_state(session_id)
            .await
            .expect("read authoritative post-lag execution snapshot");
        assert_eq!(converged.session.title, final_title);
        assert_eq!(converged.session.version, snapshot.version);
        assert_eq!(converged.session.state, SessionState::Ready);
        provider.await.expect("empty fake provider exits");
    }

    #[tokio::test]
    async fn operator_invoke_is_bound_to_the_server_workspace_id() {
        let (provider_url, _requests, provider) = spawn_fake_responses_provider(Vec::new()).await;
        let server = start_test_server(provider_url.as_str()).await;
        let client = AgenaClient::new(server.url.as_str()).expect("build operator client");

        let matching = client
            .invoke_operator_tool(
                server.workspace_id,
                "fs.read",
                Some(serde_json::json!({"path": "permission-fixture.txt"})),
            )
            .await
            .expect("invoke operator tool in authoritative workspace");
        let output_text = matching
            .get("output_text")
            .and_then(serde_json::Value::as_str)
            .expect("matching operator result contains output text");
        assert!(output_text.contains("permission fixture"));

        let foreign_workspace = tempfile::tempdir().expect("create foreign operator workspace");
        let foreign = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: foreign_workspace.path().to_string_lossy().into_owned(),
                create_if_missing: true,
            }))
            .await
            .expect("register foreign operator workspace");
        let CommandResult::Workspace(foreign) = foreign else {
            panic!("server returned the wrong foreign workspace result");
        };
        assert_ne!(foreign.id, server.workspace_id);
        let marker = "operator-scope-must-not-write.txt";
        let patch =
            format!("*** Begin Patch\n*** Add File: {marker}\n+scope escape\n*** End Patch");
        let mismatch = client
            .invoke_operator_tool(
                foreign.id,
                "fs.apply_patch",
                Some(serde_json::json!({"patch": patch})),
            )
            .await
            .expect_err("foreign workspace must be rejected before tool execution");
        assert_eq!(
            mismatch
                .problem()
                .map(|problem| problem.user.fallback.as_str()),
            Some("The operator workspace does not match this server.")
        );
        assert!(!server._workspace.path().join(marker).exists());
        assert!(!foreign_workspace.path().join(marker).exists());

        let missing = client
            .invoke_operator_tool(
                i64::MAX,
                "fs.read",
                Some(serde_json::json!({"path": marker})),
            )
            .await
            .expect_err("unknown workspace id must be rejected");
        assert_eq!(
            missing
                .problem()
                .map(|problem| problem.user.fallback.as_str()),
            Some("The operator workspace was not found.")
        );
        provider.await.expect("empty fake provider exits");
    }

    #[tokio::test]
    async fn disconnected_client_does_not_cancel_server_owned_completion() {
        let (release_tx, release_rx) = oneshot::channel();
        let (provider_url, mut requests, provider) =
            spawn_fake_responses_provider(vec![FakeProviderPlan {
                events: terminal_response_events("completed after disconnect", "resp_done_1"),
                release: Some(release_rx),
            }])
            .await;
        let server = start_test_server(provider_url.as_str()).await;
        let client_a = AgenaClient::new(server.url.as_str()).expect("build first client");
        let submitted = submit_test_run(&client_a, server.workspace_id, "disconnect test").await;
        let session_id = submitted.session.id;
        let connection = client_a
            .connect_session(session_id)
            .await
            .expect("attach first client stream");
        let provider_request =
            tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv()).await;
        if provider_request.is_err() {
            let state = client_a
                .get_session_state(session_id)
                .await
                .expect("read timed-out provider session");
            panic!("fake provider received no request; session={state:#?}");
        }
        provider_request
            .expect("provider request timeout checked")
            .expect("fake provider request channel remains open");
        drop(connection);
        drop(client_a);

        let client_b = AgenaClient::new(server.url.as_str()).expect("build second client");
        let running = wait_for_execution(&client_b, session_id, |execution| {
            execution.session.state == SessionState::Running
        })
        .await;
        assert!(running.active_execution.is_some());
        let overview = client_b
            .session_overview(Some(server.workspace_id), 10)
            .await
            .expect("read second-client overview");
        assert!(
            overview
                .running
                .iter()
                .any(|session| session.id == session_id)
        );

        release_tx.send(()).expect("release fake provider response");
        let completed = wait_for_execution(&client_b, session_id, |execution| {
            execution.session.state == SessionState::Ready
                && execution_text(execution).contains("completed after disconnect")
        })
        .await;
        assert!(completed.active_execution.is_none());
        assert!(completed.pending_interactive_requests.is_empty());
        provider.await.expect("fake provider exits");
    }

    #[tokio::test]
    async fn another_client_can_answer_and_racing_replies_have_one_winner() {
        let (provider_url, mut requests, provider) = spawn_fake_responses_provider(vec![
            FakeProviderPlan {
                events: ask_user_response_events(),
                release: None,
            },
            FakeProviderPlan {
                events: terminal_response_events("continued after reply", "resp_done_2"),
                release: None,
            },
        ])
        .await;
        let server = start_test_server(provider_url.as_str()).await;
        let client_a = AgenaClient::new(server.url.as_str()).expect("build submitting client");
        let submitted = submit_test_run(&client_a, server.workspace_id, "ask another client").await;
        let session_id = submitted.session.id;
        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("fake provider receives first request")
            .expect("first provider request exists");
        drop(client_a);

        let client_b = AgenaClient::new(server.url.as_str()).expect("build reply client B");
        let pending = wait_for_execution(&client_b, session_id, |execution| {
            !execution.pending_interactive_requests.is_empty()
        })
        .await;
        let request_id = pending.pending_interactive_requests[0]
            .request
            .request_id()
            .to_owned();
        let reply = |answer: &str| ReplyUserInputParams {
            session_id,
            options: RunOptions::default(),
            reply: UserInputReply {
                request_id: request_id.clone(),
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([("0".to_owned(), vec![answer.to_owned()])]),
                reason: None,
            },
        };
        let client_c = AgenaClient::new(server.url.as_str()).expect("build reply client C");
        let (reply_b, reply_c) = tokio::join!(
            client_b.reply_user_input(reply("Blue")),
            client_c.reply_user_input(reply("Green"))
        );
        assert!(
            reply_b.is_ok() || reply_c.is_ok(),
            "one racing client must consume the durable request: B={reply_b:?}, C={reply_c:?}"
        );

        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("fake provider receives continuation")
            .expect("continuation provider request exists");
        let completed = wait_for_execution(&client_c, session_id, |execution| {
            execution.session.state == SessionState::Ready
                && execution_text(execution).contains("continued after reply")
        })
        .await;
        assert!(completed.pending_interactive_requests.is_empty());
        provider.await.expect("fake provider exits");
        assert!(
            requests.recv().await.is_none(),
            "a racing retry must not start a second provider continuation"
        );
        let persisted_answers = completed
            .parts
            .iter()
            .find_map(|part| {
                part.content
                    .pointer("/operation/user_input/requests/0/reply/answers/0")
                    .and_then(serde_json::Value::as_array)
            })
            .expect("one durable user-input reply");
        assert_eq!(persisted_answers.len(), 1);
        assert!(
            persisted_answers[0] == "Blue" || persisted_answers[0] == "Green",
            "one racing answer must win without last-response merging: {persisted_answers:?}"
        );
    }

    #[tokio::test]
    async fn another_client_can_answer_and_racing_permission_replies_have_one_winner() {
        let (provider_url, mut requests, provider) = spawn_fake_responses_provider(vec![
            FakeProviderPlan {
                events: read_workspace_response_events(),
                release: None,
            },
            FakeProviderPlan {
                events: terminal_response_events(
                    "continued after permission reply",
                    "resp_permission_done",
                ),
                release: None,
            },
        ])
        .await;
        let server = start_test_server(provider_url.as_str()).await;
        let client_a = AgenaClient::new(server.url.as_str()).expect("build submitting client");
        let submitted = submit_test_run(
            &client_a,
            server.workspace_id,
            "ask another client for permission",
        )
        .await;
        let session_id = submitted.session.id;
        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("fake provider receives permission-producing request")
            .expect("permission-producing provider request exists");
        drop(client_a);

        let client_b = AgenaClient::new(server.url.as_str()).expect("build permission client B");
        let pending = wait_for_execution(&client_b, session_id, |execution| {
            execution
                .pending_interactive_requests
                .iter()
                .any(|pending| pending.request.as_permission().is_some())
        })
        .await;
        let pending_permission = pending
            .pending_interactive_requests
            .iter()
            .find(|pending| pending.request.as_permission().is_some())
            .expect("one durable permission request");
        assert!(
            pending.active_execution.is_some(),
            "permission wait must remain owned by the original server execution: {pending:#?}"
        );
        assert_eq!(pending_permission.session_id, session_id);
        let permission = pending_permission
            .request
            .as_permission()
            .expect("pending resource contains permission");
        assert!(
            std::iter::once(&permission.action)
                .chain(permission.requested_actions.iter())
                .any(|action| matches!(action, PermissionActionResource::PathAccess { .. })),
            "workspace read must surface as a concrete path permission: {permission:#?}"
        );
        let request_id = permission.request_id.clone();
        let reply = |kind, reason: &str| ReplyPermissionParams {
            session_id,
            options: RunOptions::default(),
            reply: PermissionReply {
                request_id: request_id.clone(),
                kind,
                reason: Some(reason.to_owned()),
                scope: None,
            },
        };
        let client_c = AgenaClient::new(server.url.as_str()).expect("build permission client C");
        let (reply_b, reply_c) = tokio::join!(
            client_b.reply_permission(reply(PermissionReplyKind::AllowOnce, "client B allowed")),
            client_c.reply_permission(reply(PermissionReplyKind::DenyOnce, "client C denied"))
        );
        assert!(
            reply_b.is_ok() || reply_c.is_ok(),
            "one racing client must consume the durable permission: B={reply_b:?}, C={reply_c:?}"
        );

        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("fake provider receives exactly one permission continuation")
            .expect("permission continuation provider request exists");
        let completed = wait_for_execution(&client_c, session_id, |execution| {
            execution.session.state == SessionState::Ready
                && execution_text(execution).contains("continued after permission reply")
        })
        .await;
        assert!(completed.pending_interactive_requests.is_empty());
        provider.await.expect("fake provider exits");
        assert!(
            requests.recv().await.is_none(),
            "a racing permission retry must not start a second provider continuation"
        );

        let persisted_replies = completed
            .parts
            .iter()
            .filter_map(|part| {
                part.content
                    .pointer("/operation/authorization/permissions")
                    .and_then(serde_json::Value::as_array)
            })
            .flatten()
            .filter(|permission| {
                permission
                    .pointer("/request/request_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(request_id.as_str())
            })
            .filter_map(|permission| permission.get("reply"))
            .collect::<Vec<_>>();
        assert_eq!(
            persisted_replies.len(),
            1,
            "exactly one permission reply must be durable: {persisted_replies:#?}"
        );
        let persisted_kind = persisted_replies[0]
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .expect("persisted permission reply kind");
        let persisted_reason = persisted_replies[0]
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .expect("persisted permission reply reason");
        assert!(
            matches!(
                (persisted_kind, persisted_reason),
                ("allow_once", "client B allowed") | ("deny_once", "client C denied")
            ),
            "the winning reply must be stored atomically without field merging: {persisted_replies:#?}"
        );
    }

    #[tokio::test]
    async fn another_client_can_race_cancel_against_natural_completion() {
        let (release_tx, release_rx) = oneshot::channel();
        let (provider_url, mut requests, provider) =
            spawn_fake_responses_provider(vec![FakeProviderPlan {
                events: terminal_response_events("natural completion", "resp_cancel_race"),
                release: Some(release_rx),
            }])
            .await;
        let server = start_test_server(provider_url.as_str()).await;
        let client_a = AgenaClient::new(server.url.as_str()).expect("build submitting client");
        let submitted = submit_test_run(&client_a, server.workspace_id, "cancel race").await;
        let session_id = submitted.session.id;
        let execution_id = submitted
            .active_execution
            .as_ref()
            .expect("submitted run is active")
            .execution_id;
        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("fake provider receives cancel-race request")
            .expect("cancel-race provider request exists");
        drop(client_a);

        let client_b = AgenaClient::new(server.url.as_str()).expect("build cancelling client");
        let (cancel, release) = tokio::join!(
            client_b.cancel_run(session_id, agena_domain::ExecutionId(execution_id)),
            async { release_tx.send(()) }
        );
        release.expect("release natural provider completion");
        let cancel = cancel.expect("cancel request reaches server");
        assert!(matches!(
            cancel,
            agena_domain::CancellationResult::CancellationRequested
                | agena_domain::CancellationResult::AlreadyTerminal
        ));
        provider.await.expect("fake provider exits");

        let terminal = wait_for_execution(&client_b, session_id, |execution| {
            execution.session.state == SessionState::Ready && execution.active_execution.is_none()
        })
        .await;
        let assistant_runs = terminal
            .parts
            .iter()
            .filter(|part| part.kind == "run" && part.role == "assistant")
            .collect::<Vec<_>>();
        assert_eq!(assistant_runs.len(), 1);
        assert!(
            matches!(assistant_runs[0].state.as_str(), "completed" | "cancelled"),
            "cancel and natural completion must converge on one terminal run: {assistant_runs:?}"
        );
    }
}
