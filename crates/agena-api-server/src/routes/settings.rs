use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
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
        .route("/api/v1/settings/sources", get(rest::get_settings_sources))
        .route("/api/v1/settings/validate", post(rest::validate_settings))
        .route("/api/v1/config/resolved", get(rest::get_resolved_config))
}
