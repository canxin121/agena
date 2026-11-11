use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/mcp-connections",
            get(rest::list_mcp_connections).post(rest::create_mcp_connection),
        )
        .route(
            "/api/v1/mcp-connections/import/preview",
            post(rest::preview_mcp_import).layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
        )
        .route(
            "/api/v1/mcp-connections/import",
            post(rest::import_mcp_connections).layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
        )
        .route(
            "/api/v1/mcp-connections/bridge/enabled",
            axum::routing::put(rest::set_mcp_bridge_enabled),
        )
        .route(
            "/api/v1/mcp-connections/{id}",
            get(rest::get_mcp_connection)
                .put(rest::update_mcp_connection)
                .delete(rest::delete_mcp_connection),
        )
        .route(
            "/api/v1/mcp-connections/{id}/enabled",
            axum::routing::put(rest::set_mcp_connection_enabled),
        )
        .route(
            "/api/v1/mcp-connections/{id}/reconnect",
            post(rest::reconnect_mcp_connection),
        )
        .route(
            "/api/v1/mcp-connections/{id}/test",
            post(rest::test_mcp_connection),
        )
        .route(
            "/api/v1/mcp-connections/{id}/capabilities",
            get(rest::mcp_connection_capabilities),
        )
        .route(
            "/api/v1/mcp-connections/{id}/resource",
            post(rest::read_mcp_connection_resource),
        )
        .route(
            "/api/v1/mcp-connections/{id}/prompt",
            post(rest::get_mcp_connection_prompt),
        )
        .route(
            "/api/v1/mcp-connections/{id}/credential",
            axum::routing::put(rest::set_mcp_connection_credential)
                .delete(rest::clear_mcp_connection_credential),
        )
        .route(
            "/api/v1/mcp-connections/{id}/auth/start",
            post(rest::start_mcp_connection_auth),
        )
        .route(
            "/api/v1/mcp-connections/{id}/auth/complete",
            post(rest::complete_mcp_connection_auth),
        )
        .route(
            "/api/v1/mcp-connections/{id}/auth",
            axum::routing::delete(rest::clear_mcp_connection_auth),
        )
        .route("/plugin-rpc/{plugin_id}", post(rest::plugin_rpc))
}
