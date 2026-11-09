use super::{AppState, Json, ServerError, State};
use agena_api::bot::*;
use axum::extract::{Path, Query};
use uuid::Uuid;

#[derive(Default, serde::Deserialize)]
pub struct BotListQuery {
    #[serde(default)]
    pub include_archived: bool,
}
fn bot_id(id: &str) -> Result<Uuid, ServerError> {
    id.parse()
        .map_err(|_| ServerError::bad_request("invalid Bot UUID"))
}
pub async fn ensure_bot_session(
    State(state): State<AppState>,
) -> Result<Json<BotSession>, ServerError> {
    state
        .application()
        .ensure_bot_session()
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn list_bots(
    State(state): State<AppState>,
    Query(query): Query<BotListQuery>,
) -> Result<Json<BotList>, ServerError> {
    state
        .application()
        .list_bots(query.include_archived)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn create_bot(
    State(state): State<AppState>,
    Json(request): Json<BotCreate>,
) -> Result<Json<BotProfile>, ServerError> {
    state
        .application()
        .create_bot(request)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn get_bot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BotProfile>, ServerError> {
    state
        .application()
        .get_bot(bot_id(&id)?)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn update_bot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<BotUpdate>,
) -> Result<Json<BotProfile>, ServerError> {
    state
        .application()
        .update_bot(bot_id(&id)?, request)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn archive_bot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<BotArchive>,
) -> Result<Json<BotProfile>, ServerError> {
    state
        .application()
        .archive_bot(bot_id(&id)?, request)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub async fn ensure_named_bot_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BotSession>, ServerError> {
    state
        .application()
        .ensure_named_bot_session(bot_id(&id)?)
        .await
        .map(Json)
        .map_err(Into::into)
}
