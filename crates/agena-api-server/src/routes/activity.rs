use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
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
}
