use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
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
