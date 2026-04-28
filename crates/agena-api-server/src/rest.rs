//! REST handlers. Each handler builds the corresponding [`Command`] / [`Query`]
//! and routes through [`crate::dispatch`].

use agena_api::{
    commands::{
        CancelTurnParams, Command, ContinueRunParams, CreateSessionParams, ReplyPermissionParams,
        ReplyUserInputParams, SubmitTurnParams,
    },
    queries::{
        GetSessionParams, ListEventsParams, ListMessagesParams, ListSessionsParams, Query,
    },
    resource::RunOptions,
};
use axum::{
    Json,
    extract::{Path, Query as AxumQuery, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{dispatch, error::ServerError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct SubmitTurnBody {
    #[serde(default)]
    pub options: RunOptions,
    pub parts: Vec<agena::message::PartContent>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ContinueBody {
    #[serde(default)]
    pub options: RunOptions,
}

#[derive(Debug, Deserialize)]
pub struct ReplyPermissionBody {
    #[serde(default)]
    pub options: RunOptions,
    pub reply: agena::permission::PermissionReply,
}

#[derive(Debug, Deserialize)]
pub struct ReplyUserInputBody {
    #[serde(default)]
    pub options: RunOptions,
    pub reply: agena::message::UserInputReply,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionBody {
    pub workspace_id: i64,
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

// ─── queries ────────────────────────────────────────────────────────────

pub async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    let result = dispatch::dispatch_query(&state, Query::Health).await?;
    Ok(Json(result))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListSessionsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let result = dispatch::dispatch_query(&state, Query::ListSessions(params)).await?;
    Ok(Json(result))
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let result =
        dispatch::dispatch_query(&state, Query::GetSession(GetSessionParams { session_id }))
            .await?;
    Ok(Json(result))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let result = dispatch::dispatch_query(
        &state,
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor: None,
            limit: None,
            parts: agena_api::resource::PartLoadMode::Full,
        }),
    )
    .await?;
    Ok(Json(result))
}

pub async fn list_events(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListEventsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let result = dispatch::dispatch_query(&state, Query::ListEvents(params)).await?;
    Ok(Json(result))
}

// ─── commands ───────────────────────────────────────────────────────────

pub async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<CreateSessionBody>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::CreateSession(CreateSessionParams {
        workspace_id: body.workspace_id,
        title: body.title,
        parent_id: body.parent_id,
    });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}

pub async fn submit_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(body): Json<SubmitTurnBody>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::SubmitTurn(SubmitTurnParams {
        session_id,
        options: body.options,
        parts: body.parts,
    });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(body): Json<ContinueBody>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::ContinueRun(ContinueRunParams {
        session_id,
        options: body.options,
    });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}

pub async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::CancelTurn(CancelTurnParams { session_id });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(body): Json<ReplyPermissionBody>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::ReplyPermission(ReplyPermissionParams {
        session_id,
        options: body.options,
        reply: body.reply,
    });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(body): Json<ReplyUserInputBody>,
) -> Result<impl IntoResponse, ServerError> {
    let cmd = Command::ReplyUserInput(ReplyUserInputParams {
        session_id,
        options: body.options,
        reply: body.reply,
    });
    let result = dispatch::dispatch_command(&state, cmd).await?;
    Ok(Json(result))
}
