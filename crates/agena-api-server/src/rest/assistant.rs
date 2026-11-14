//! Assistant inspection and correction through the shared application use cases.
use crate::{error::ServerError, state::AppState};
use agena_api::assistant::*;
use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;

pub async fn get_assistant(
    State(state): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<AssistantSnapshot>, ServerError> {
    state
        .application()
        .assistant_snapshot(source)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn put_assistant(
    State(state): State<AppState>,
    Path(source): Path<i64>,
    Json(request): Json<AssistantUpdate>,
) -> Result<Json<AssistantProfile>, ServerError> {
    state
        .application()
        .update_assistant(source, request)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn control_assistant_reminder(
    State(state): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(request): Json<ReminderControl>,
) -> Result<Json<Reminder>, ServerError> {
    state
        .application()
        .control_assistant_reminder(source, id, request)
        .await
        .map(Json)
        .map_err(Into::into)
}

pub async fn edit_assistant_memory(
    State(state): State<AppState>,
    Path(source): Path<i64>,
    Json(request): Json<AssistantMemoryUpdate>,
) -> Result<Json<Option<AssistantMemory>>, ServerError> {
    state
        .application()
        .change_assistant_memory(source, request)
        .await
        .map(Json)
        .map_err(Into::into)
}
