use super::{AppState, Json, ServerError, State};
use agena_api::bot::*;
use axum::extract::{Path, Query};
use uuid::Uuid;

pub use agena_api::business::BotListQuery;
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
/// Open this Bot's one local conversation, creating it when absent.
/// The optional title applies only to that first creation; reopening is idempotent.
pub async fn create_bot_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<BotConversationCreate>>,
) -> Result<Json<BotSession>, ServerError> {
    let title = body.and_then(|Json(request)| request.title);
    state
        .application()
        .create_bot_conversation(bot_id(&id)?, title)
        .await
        .map(Json)
        .map_err(Into::into)
}
