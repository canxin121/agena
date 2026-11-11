use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/skills",
            get(rest::list_skills).post(rest::create_skill),
        )
        .route("/api/v1/skills/validate", post(rest::validate_skill))
        .route(
            "/api/v1/skills/install/preview",
            post(rest::preview_skill_install).layer(DefaultBodyLimit::max(90 * 1024 * 1024)),
        )
        .route(
            "/api/v1/skills/install",
            post(rest::install_skills).layer(DefaultBodyLimit::max(90 * 1024 * 1024)),
        )
        .route(
            "/api/v1/skills/{id}",
            get(rest::get_skill)
                .put(rest::update_skill)
                .delete(rest::delete_skill),
        )
        .route(
            "/api/v1/skills/{id}/enabled",
            axum::routing::put(rest::set_skill_enabled),
        )
        .route(
            "/api/v1/skills/{id}/resource",
            get(rest::get_skill_resource)
                .put(rest::write_skill_resource)
                .delete(rest::delete_skill_resource),
        )
}
