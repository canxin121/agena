use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/permission-rules",
            get(rest::list_permission_rules).post(rest::create_permission_rule),
        )
        .route(
            "/api/v1/permission-rules/batch",
            post(rest::batch_permission_rules),
        )
        .route(
            "/api/v1/permission-rules/{rule_id}",
            get(rest::get_permission_rule)
                .put(rest::replace_permission_rule)
                .delete(rest::delete_permission_rule),
        )
        .route(
            "/api/v1/permission-rules/{rule_id}/revoke",
            post(rest::revoke_permission_rule),
        )
}
