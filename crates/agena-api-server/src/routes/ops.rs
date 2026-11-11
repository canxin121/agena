use axum::Router;
use axum::routing::get;

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(rest::healthz))
        .route("/readyz", get(rest::readyz))
        .route("/metrics", get(rest::metrics))
}
