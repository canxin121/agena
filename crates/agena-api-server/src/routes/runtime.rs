use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/runtime", get(rest::get_runtime_status))
        .route("/api/v1/runtime/reload", post(rest::reload_runtime))
        .route(
            "/api/v1/runtime/tasks",
            get(rest::list_runtime_background_tasks),
        )
        .route(
            "/api/v1/runtime/tasks/{task_id}/cancel",
            post(rest::cancel_runtime_background_task),
        )
        .route("/api/v1/usage", get(rest::get_usage_stats))
}
