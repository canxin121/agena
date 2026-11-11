use axum::Router;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/memories", get(rest::list_memories))
        .route("/api/v1/memories/batch-delete", post(rest::delete_memories))
        .route("/api/v1/memories/overview", get(rest::get_memory_overview))
        .route("/api/v1/memories/index", post(rest::ensure_memory_index))
        .route(
            "/api/v1/memories/{name}",
            get(rest::get_memory)
                .put(rest::save_memory)
                .delete(rest::delete_memory),
        )
}
