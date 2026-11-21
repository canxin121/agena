use agena_domain::ProcessSummary;
use axum::{
    Json,
    extract::{Path, State},
};

use crate::{AppState, error::ServerError};

fn process_response(process: Option<ProcessSummary>) -> Result<Json<ProcessSummary>, ServerError> {
    process
        .map(Json)
        .ok_or_else(|| ServerError::not_found("The process or subscription was not found."))
}

async fn stop_session_process(
    state: AppState,
    session_id: i64,
    id: String,
    websocket: bool,
) -> Result<Json<ProcessSummary>, ServerError> {
    let process = state
        .runtime_control()
        .stop_session_process(session_id, &id, websocket)
        .await
        .map_err(agena_application::ApplicationError::from)?;
    process_response(process)
}

pub async fn list_shell_processes(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Json<Vec<ProcessSummary>> {
    Json(state.runtime_control().session_processes(session_id, false))
}

pub async fn list_monitor_subscriptions(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Json<Vec<ProcessSummary>> {
    Json(state.runtime_control().session_processes(session_id, true))
}

pub async fn get_shell_process(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, String)>,
) -> Result<Json<ProcessSummary>, ServerError> {
    process_response(
        state
            .runtime_control()
            .session_process(session_id, &id, false),
    )
}

pub async fn stop_shell_process(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, String)>,
) -> Result<Json<ProcessSummary>, ServerError> {
    stop_session_process(state, session_id, id, false).await
}

pub async fn get_monitor_subscription(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, String)>,
) -> Result<Json<ProcessSummary>, ServerError> {
    process_response(
        state
            .runtime_control()
            .session_process(session_id, &id, true),
    )
}

pub async fn stop_monitor_subscription(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, String)>,
) -> Result<Json<ProcessSummary>, ServerError> {
    stop_session_process(state, session_id, id, true).await
}
