use agena_plugin_host::sdk::browser::BrowserControlCommand;
use axum::http::header::CACHE_CONTROL;
use axum::response::Response;

use super::{AppState, IntoResponse, Json, Path, ServerError, State};

pub async fn list_browser_tabs(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<Response, ServerError> {
    browser_command(&state, session_id, BrowserControlCommand::List).await
}

pub async fn control_browser(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(command): Json<BrowserControlCommand>,
) -> Result<Response, ServerError> {
    browser_command(&state, session_id, command).await
}

async fn browser_command(
    state: &AppState,
    session_id: i64,
    command: BrowserControlCommand,
) -> Result<Response, ServerError> {
    if session_id <= 0 {
        return Err(ServerError::bad_request(
            "A conversation session is required",
        ));
    }
    let result = state
        .runtime_control()
        .browser_control(session_id, command)
        .await
        .map_err(|error| ServerError::from_failure(*error.failure))?;
    Ok(([(CACHE_CONTROL, "no-store")], Json(result)).into_response())
}
