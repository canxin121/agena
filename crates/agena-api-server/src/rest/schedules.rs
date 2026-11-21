use agena_api::resource::ScheduledJobResource;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{AppState, error::ServerError};

#[derive(Deserialize)]
pub struct SchedulePauseRequest {
    paused: bool,
}

pub async fn list_session_schedules(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let read =
        crate::revisions::ConditionalRead::new(&state, &format!("session:{session_id}:schedules"))
            .await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    read.json(
        state
            .application()
            .list_session_schedules(session_id)
            .await?,
    )
    .await
}

pub async fn set_session_schedule_paused(
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(i64, Uuid)>,
    Json(request): Json<SchedulePauseRequest>,
) -> Result<Json<ScheduledJobResource>, ServerError> {
    Ok(Json(
        state
            .application()
            .set_session_schedule_paused(session_id, schedule_id, request.paused)
            .await?,
    ))
}

pub async fn delete_session_schedule(
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(i64, Uuid)>,
) -> Result<StatusCode, ServerError> {
    state
        .application()
        .delete_session_schedule(session_id, schedule_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
