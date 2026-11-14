use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .merge(crate::plugin_web::operation_routes())
        .route("/api/v1/plugins", get(rest::list_plugins))
        .route(
            "/api/v1/plugins/surface",
            get(rest::get_plugin_surface_catalog),
        )
        .route(
            "/api/v1/plugins/architecture",
            get(rest::get_plugin_architecture_catalog),
        )
        .route(
            "/api/v1/plugins/tools/changes",
            get(rest::list_plugin_tool_registry_changes),
        )
        .route(
            "/api/v1/plugins/tools/invoke",
            post(rest::invoke_plugin_tool),
        )
        .route("/api/v1/plugins/{plugin_id}", get(rest::get_plugin))
        .route(
            "/api/v1/plugins/{plugin_id}/settings",
            get(rest::get_plugin_settings).put(rest::update_plugin_settings),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/commands/{command_id}/invoke",
            post(rest::run_plugin_command),
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
}
