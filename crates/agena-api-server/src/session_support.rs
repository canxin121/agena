use agena::{
    message::PartContent,
    permission::PermissionReply,
    session::{
        GoalStatus, Session, SessionExecutionReplyRequest, SessionExecutionRequest, SessionGoal,
        SessionGoalCreateRequest, SessionGoalUpdateRequest, SessionManager,
        SessionPermissionReplyRequest, SessionUserMessageRequest,
    },
};
use agena_api::resource::{RunOptions, SessionExecutionResource, SessionGoalResource};

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

pub(crate) async fn clear_session_goal_or_not_found(
    manager: &SessionManager,
    session_id: i64,
) -> Result<(), ServerError> {
    let cleared = manager
        .clear_goal(session_id)
        .await
        .map_err(ServerError::Core)?;
    if cleared {
        Ok(())
    } else {
        Err(ServerError::NotFound(format!(
            "session {session_id} goal not found"
        )))
    }
}

pub(crate) async fn set_session_goal(
    manager: &SessionManager,
    session_id: i64,
    objective: Option<String>,
    status: Option<GoalStatus>,
    clear: bool,
) -> Result<Option<SessionGoal>, ServerError> {
    if clear {
        clear_session_goal_or_not_found(manager, session_id).await?;
        return Ok(None);
    }

    let goal = if manager
        .get_goal(session_id)
        .await
        .map_err(ServerError::Core)?
        .is_some()
    {
        manager
            .update_goal(SessionGoalUpdateRequest {
                session_id,
                objective,
                status,
                expected_goal_id: None,
            })
            .await
            .map_err(ServerError::Core)?
    } else {
        if !matches!(status, None | Some(GoalStatus::Active)) {
            return Err(ServerError::BadRequest(format!(
                "session {session_id} goal must be created with status active"
            )));
        }
        let objective = objective.ok_or_else(|| {
            ServerError::BadRequest(format!(
                "session {session_id} goal objective is required when creating a goal"
            ))
        })?;
        manager
            .create_goal(SessionGoalCreateRequest {
                session_id,
                objective,
            })
            .await
            .map_err(ServerError::Core)?
    };

    Ok(Some(goal))
}

pub(crate) async fn session_goal_resource_for_session(
    state: &AppState,
    manager: &SessionManager,
    session: &Session,
    goal: &SessionGoal,
) -> Result<SessionGoalResource, ServerError> {
    state
        .service()
        .session_goal_resource(manager, session, goal)
        .await
        .map_err(server_error_from_http)
}

pub(crate) async fn session_goal_resource(
    state: &AppState,
    manager: &SessionManager,
    session_id: i64,
    goal: &SessionGoal,
) -> Result<SessionGoalResource, ServerError> {
    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    session_goal_resource_for_session(state, manager, &session, goal).await
}
