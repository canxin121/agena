use axum::Router;
use axum::routing::get;

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/v1/health", get(rest::health))
}
