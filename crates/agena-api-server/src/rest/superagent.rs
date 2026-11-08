//! Uses the existing operator-authenticated API boundary. These endpoints must
//! not be mounted as unauthenticated third-party chatbot webhooks.
use agena_superagent::{ChatMessage, ChatReply, CreateTask, ResumeTask, Task, TaskQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ServerError, state::AppState};

fn task_error(error: agena_superagent::Error) -> ServerError {
    match error {
        agena_superagent::Error::Invalid(message) => ServerError::bad_request(message),
        agena_superagent::Error::NotFound => {
            ServerError::not_found("The task was not found in this scope.")
        }
        agena_superagent::Error::Conflict(message) => ServerError::conflict(message),
        agena_superagent::Error::Unavailable(message) => ServerError::service_unavailable(message),
        agena_superagent::Error::Policy(s) => ServerError::bad_request_with_diagnostic(
            "invalid channel access policy; check identities, owners, role capabilities and limits",
            s,
        ),
        agena_superagent::Error::Storage(error) => ServerError::internal(error),
    }
}

pub async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTask>,
) -> Result<Json<Task>, ServerError> {
    state
        .application()
        .superagent()?
        .create(request)
        .await
        .map(Json)
        .map_err(task_error)
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, ServerError> {
    state
        .application()
        .superagent()?
        .get(id)
        .await
        .map(Json)
        .map_err(task_error)
}

pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<Task>>, ServerError> {
    state
        .application()
        .superagent()?
        .list(query)
        .await
        .map(Json)
        .map_err(task_error)
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    request: Option<Json<CancelTask>>,
) -> Result<Json<Task>, ServerError> {
    state
        .application()
        .superagent()?
        .cancel(id, request.and_then(|Json(body)| body.expected_revision))
        .await
        .map(Json)
        .map_err(task_error)
}

pub async fn resume_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<ResumeTask>,
) -> Result<Json<Task>, ServerError> {
    state
        .application()
        .superagent()?
        .resume(id, request)
        .await
        .map(Json)
        .map_err(task_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptTask {
    expected_revision: i64,
}

pub async fn accept_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<AcceptTask>,
) -> Result<Json<Task>, ServerError> {
    state
        .application()
        .superagent()?
        .accept(id, request.expected_revision)
        .await
        .map(Json)
        .map_err(task_error)
}

pub async fn chatbot_message(
    State(state): State<AppState>,
    Json(message): Json<ChatMessage>,
) -> Result<Json<ChatReply>, ServerError> {
    state
        .application()
        .superagent()?
        .chat(message)
        .await
        .map(Json)
        .map_err(task_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelTask {
    #[serde(default)]
    expected_revision: Option<i64>,
}
