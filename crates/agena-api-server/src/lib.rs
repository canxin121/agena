// macOS ld can emit a non-actionable compact-unwind diagnostic for this large
// test binary; keep test output focused on actionable Rust diagnostics.

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
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
mod json_codec;
#[cfg(feature = "jsonrpc")]
pub mod jsonrpc;
#[cfg(any(feature = "http", feature = "ws", feature = "sse"))]
mod live;
#[cfg(feature = "http")]
pub mod rest;
#[cfg(feature = "http")]
mod revisions;
#[cfg(feature = "http")]
pub mod route_catalog;
#[cfg(feature = "http")]
mod routes;
#[cfg(feature = "sse")]
pub mod sse;
pub mod state;
#[cfg(feature = "ws")]
pub mod ws;

pub use state::AppState;

use axum::Router;
#[cfg(feature = "http")]
use axum::middleware::{self, Next};
#[cfg(any(feature = "ws", feature = "sse"))]
use axum::routing::get;

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
    let router = routes::router().layer(middleware::from_fn(count_request));

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
