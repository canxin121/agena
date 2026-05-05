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
//!   and REST handler share identical semantics.

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
        // REST queries
        .route("/api/v1/health", get(rest::health))
        .route("/api/v1/sessions", get(rest::list_sessions))
        .route("/api/v1/sessions/{session_id}", get(rest::get_session))
        .route(
            "/api/v1/sessions/{session_id}/messages",
            get(rest::list_messages),
        )
        .route("/api/v1/events", get(rest::list_events))
        // REST commands
        .route("/api/v1/sessions", post(rest::create_session))
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
        // Streaming transports
        .route("/api/v1/ws", get(ws::handler))
        .route("/api/v1/events/stream", get(sse::handler))
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
