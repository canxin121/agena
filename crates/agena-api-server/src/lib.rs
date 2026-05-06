//! # agena-api-server
//!
//! axum-based v1 API server. Wires the unified [`agena_api`] protocol over
//! REST + WebSocket + SSE + Unix-socket transports, all backed by the same
//! `agena::session::SessionManager` and the unified
//! [`agena::event::EventPublisher`] / [`agena_event::EventBus`].
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
pub mod ipc;
pub mod rest;
pub mod sse;
pub mod state;
pub mod ws;

pub use state::AppState;

use axum::{
    Router,
    routing::{get, post},
};

/// Build the v1 axum router with every transport mounted.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(rest::health))
        .route("/api/v1/runtime", get(rest::get_runtime_status))
        .route("/api/v1/runtime/reload", post(rest::reload_runtime))
        .route("/api/v1/plugins", get(rest::list_plugins))
        .route("/api/v1/plugins/{plugin_id}", get(rest::get_plugin))
        .route("/api/v1/plugins/{plugin_id}/logs", get(rest::list_plugin_logs))
        .route("/api/v1/auth/providers", get(rest::list_auth_providers))
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
            "/api/v1/providers/{provider_id}/models",
            get(rest::list_provider_models),
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
        .route(
            "/api/v1/sessions/{session_id}/turns",
            post(rest::submit_turn),
        )
        .route(
            "/api/v1/sessions/{session_id}/continue",
            post(rest::continue_run),
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
            "/api/v1/permission-rules",
            get(rest::list_permission_rules).post(rest::create_permission_rule),
        )
        .route(
            "/api/v1/permission-rules/{rule_id}",
            get(rest::get_permission_rule)
                .put(rest::replace_permission_rule)
                .delete(rest::delete_permission_rule),
        )
        .route("/api/v1/events", get(rest::list_events))
        .route("/api/v1/ws", get(ws::handler))
        .route("/api/v1/events/stream", get(sse::handler))
        .route("/plugin-rpc/{plugin_id}", post(rest::plugin_rpc))
        .with_state(state)
}

/// Build the streaming-only transport router for hosts that already mount
/// overlapping REST endpoints via another API surface.
pub fn transport_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/ws", get(ws::handler))
        .route("/api/v1/events/stream", get(sse::handler))
        .with_state(state)
}
