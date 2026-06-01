use agena::{
    message::PartContent,
    permission::PermissionReply,
    session::{
        Session, SessionExecutionReplyRequest, SessionExecutionRequest, SessionManager,
        SessionPermissionReplyRequest, SessionUserMessageRequest,
    },
};
use agena_api::resource::{RunOptions, SessionExecutionResource};

use crate::{error::ServerError, state::AppState};

pub(crate) fn server_error_from_http(error: crate::local_api::ApiError) -> ServerError {
    match error.status_code() {
        axum::http::StatusCode::BAD_REQUEST => ServerError::BadRequest(error.message().to_owned()),
        axum::http::StatusCode::NOT_FOUND => ServerError::NotFound(error.message().to_owned()),
        axum::http::StatusCode::CONFLICT => ServerError::Conflict(error.message().to_owned()),
        axum::http::StatusCode::SERVICE_UNAVAILABLE => {
            ServerError::ServiceUnavailable(error.message().to_owned())
        }
        _ => ServerError::Internal(error.message().to_owned()),
    }
}

pub(crate) async fn resolve_session_run_options(
    state: &AppState,
    session_id: i64,
    request: RunOptions,
) -> Result<agena::session::SessionRunOptions, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let default_model = snapshot
        .resolve_default_model()
        .map_err(ServerError::Core)?;
    let manager = state.session_manager()?;
    state
        .service()
        .resolve_run_options(
            snapshot.provider_registry().as_ref(),
            default_model,
            manager.as_ref(),
            session_id,
            request,
        )
        .await
        .map_err(server_error_from_http)
}

pub(crate) async fn session_execution_request(
    state: &AppState,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionRequest, ServerError> {
    Ok(SessionExecutionRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, request).await?,
    ))
}

pub(crate) async fn session_execution_reply_request<T>(
    state: &AppState,
    session_id: i64,
    options: RunOptions,
    reply: T,
) -> Result<SessionExecutionReplyRequest<T>, ServerError> {
    Ok(SessionExecutionReplyRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        reply,
    ))
}

pub(crate) async fn session_permission_reply_request(
    state: &AppState,
    session_id: i64,
    options: RunOptions,
    reply: PermissionReply,
    source: Option<String>,
) -> Result<SessionPermissionReplyRequest, ServerError> {
    Ok(SessionPermissionReplyRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        reply,
        source,
    ))
}

pub(crate) async fn session_user_message_request(
    state: &AppState,
    session_id: i64,
    options: RunOptions,
    parts: Vec<PartContent>,
) -> Result<SessionUserMessageRequest, ServerError> {
    Ok(SessionUserMessageRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        parts,
    ))
}

pub(crate) async fn session_execution_resource(
    state: &AppState,
    manager: &SessionManager,
    session: &Session,
) -> Result<SessionExecutionResource, ServerError> {
    state
        .service()
        .session_execution_resource(manager, session)
        .await
        .map_err(server_error_from_http)
}
