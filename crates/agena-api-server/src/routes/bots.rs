use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::{AppState, rest};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/bot/session", post(rest::ensure_bot_session))
        // Persona instructions may be a full character sheet, and JSON
        // escaping roughly doubles multibyte text. Raise only these two
        // routes above axum's 2 MiB default; the persona ceiling itself is
        // validated in `agena_assistant`.
        .route(
            "/api/v1/bots",
            get(rest::list_bots)
                .post(rest::create_bot)
                .layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route(
            "/api/v1/bots/{id}",
            get(rest::get_bot)
                .put(rest::update_bot)
                .layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/api/v1/bots/{id}/archive", post(rest::archive_bot))
        .route(
            "/api/v1/bots/{id}/session",
            post(rest::ensure_named_bot_session),
        )
        .route(
            "/api/v1/bots/{id}/sessions",
            post(rest::create_bot_conversation),
        )
}
