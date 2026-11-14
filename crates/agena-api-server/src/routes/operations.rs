//! A transport adapter for every shared Query/Command operation.
use crate::{AppState, error::ServerError};
use agena_api::{commands::Command, queries::Query};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/queries", post(query))
        .route("/api/v1/commands", post(command))
        .layer(DefaultBodyLimit::max(96 * 1024 * 1024))
}

async fn query(
    State(state): State<AppState>,
    Json(query): Json<Query>,
) -> Result<axum::response::Response, ServerError> {
    crate::json_codec::response(
        agena_application::dispatch::dispatch_query(state.application(), query).await?,
    )
    .await
}

async fn command(
    State(state): State<AppState>,
    Json(command): Json<Command>,
) -> Result<axum::response::Response, ServerError> {
    crate::json_codec::response(
        agena_application::dispatch::dispatch_command(state.application(), command).await?,
    )
    .await
}
