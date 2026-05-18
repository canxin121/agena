//! # agena-api-server
//!
//! Unified transport crate for Agena surfaces. It wires the shared [`agena_api`]
//! protocol and adjacent local protocols over feature-gated transports including
//! HTTP/REST, WebSocket, SSE, Unix-socket IPC, and JSON-RPC app-server entrypoints,
//! all backed by the same `agena::session::SessionManager` and unified event bus.
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
//! - The transports never poll the database. They subscribe to the in-process
//!   broadcast bus exposed by `SessionManager::event_bus()`. Resume from the
//!   persisted store happens on initial subscribe / on `Lagged` recovery via
//!   `EventStore::range`.
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
pub mod local_api;
mod provider_queries;
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
            .route(
                "/api/v1/model-catalog/entries",
                axum::routing::put(rest::upsert_model_catalog_entry)
                    .delete(rest::delete_model_catalog_entry),
            )
            .route("/api/v1/git/status", get(rest::get_git_status))
            .route("/api/v1/project/git/init", post(rest::init_git_repository))
            .route("/api/v1/vcs/diff/raw", get(rest::get_vcs_diff_raw))
            .route("/api/v1/plugins", get(rest::list_plugins))
            .route("/api/v1/plugins/ui", get(rest::get_plugin_ui_catalog))
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
                "/api/v1/auth/providers/atomgit/browser/start",
                post(rest::start_atomgit_browser_auth),
            )
            .route(
                "/api/v1/auth/providers/atomgit/browser/poll",
                post(rest::poll_atomgit_browser_auth),
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
                "/api/v1/sessions/{session_id}/goal",
                get(rest::get_session_goal)
                    .put(rest::set_session_goal)
                    .delete(rest::clear_session_goal),
            )
            .route(
                "/api/v1/sessions/{session_id}/goal/complete",
                post(rest::complete_session_goal),
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
                get(rest::list_messages),
            )
            .route("/api/v1/messages/{message_id}", get(rest::get_message))
            .route(
                "/api/v1/messages/{message_id}/parts",
                get(rest::list_message_parts),
            )
            .route(
                "/api/v1/message-parts/{part_id}",
                get(rest::get_message_part),
            )
            .route(
                "/api/v1/sessions/{session_id}/turns",
                post(rest::submit_turn),
            )
            .route(
                "/api/v1/sessions/{session_id}/continue",
                post(rest::continue_run),
            )
            .route(
                "/api/v1/sessions/{session_id}/fork",
                post(rest::fork_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/cancel",
                post(rest::cancel_turn),
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
                "/api/v1/sessions/{session_id}/unrewind",
                post(rest::unrewind_session),
            )
            .route(
                "/api/v1/sessions/{session_id}/export",
                get(rest::export_session),
            )
            .route("/api/v1/sessions/import", post(rest::import_session))
            .route(
                "/api/v1/sessions/tree/{root_id}",
                get(rest::list_session_tree),
            )
            .route(
                "/api/v1/sessions/{session_id}/rewind-checkpoints",
                get(rest::list_rewind_checkpoints),
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
