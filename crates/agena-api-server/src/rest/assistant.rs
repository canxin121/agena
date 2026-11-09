//! Authenticated operator inspection/correction. Normal conversation uses the
//! scoped companion tool, not these direct control endpoints.
use crate::{error::ServerError, state::AppState};
use agena_api::assistant::*;
use axum::{
    Json,
    extract::{Path, State},
};
use uuid::Uuid;
fn error(e: agena_assistant::Error) -> ServerError {
    match e {
        agena_assistant::Error::Invalid(s) => ServerError::bad_request(s),
        agena_assistant::Error::Conflict(s) => ServerError::conflict(s),
        agena_assistant::Error::NotFound => {
            ServerError::not_found("assistant resource not found in this workspace")
        }
        agena_assistant::Error::Unavailable(s) => ServerError::service_unavailable(s),
        agena_assistant::Error::Policy(s) => ServerError::bad_request_with_diagnostic(
            "invalid channel access policy; check identities, owners, role capabilities and limits",
            s,
        ),
        agena_assistant::Error::Storage(s) => ServerError::internal(s),
    }
}
pub async fn get_assistant(
    State(s): State<AppState>,
    Path(source): Path<i64>,
) -> Result<Json<AssistantSnapshot>, ServerError> {
    s.application()
        .assistant_service()?
        .assistant(source)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn put_assistant(
    State(s): State<AppState>,
    Path(source): Path<i64>,
    Json(r): Json<AssistantUpdate>,
) -> Result<Json<AssistantProfile>, ServerError> {
    s.application()
        .assistant_service()?
        .update_assistant(source, r)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn control_assistant_reminder(
    State(s): State<AppState>,
    Path((source, id)): Path<(i64, Uuid)>,
    Json(r): Json<ReminderControl>,
) -> Result<Json<Reminder>, ServerError> {
    s.application()
        .assistant_service()?
        .control_reminder(source, id, r)
        .await
        .map(Json)
        .map_err(error)
}
pub async fn edit_assistant_memory(
    State(s): State<AppState>,
    Path(source): Path<i64>,
    Json(r): Json<AssistantMemoryUpdate>,
) -> Result<Json<Option<AssistantMemory>>, ServerError> {
    s.application()
        .assistant_service()?
        .change_assistant_memory(source, r)
        .await
        .map(Json)
        .map_err(error)
}
