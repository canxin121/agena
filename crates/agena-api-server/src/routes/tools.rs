use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/operator/tools", get(rest::list_operator_tools))
        .route(
            "/api/v1/operator/tools/invoke",
            post(rest::invoke_operator_tool),
        )
}
