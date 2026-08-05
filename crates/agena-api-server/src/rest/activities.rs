//! REST handlers for the unified background-activity surface.

use agena_api::queries::{ActivityLogsParams, GetActivityParams, ListActivitiesParams};
use agena_api::resource::BackgroundActivityResource;
use serde::Deserialize;

use super::{
    AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State, query_json,
    server_error_from_application,
};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListActivitiesQuery {
    /// Comma-separated kind filters (`shell`, `task`, `runtime`, `browser`).
    #[serde(default)]
    pub kinds: Option<String>,
    /// Comma-separated status filters (`running`, `succeeded`, `failed`, …).
    #[serde(default)]
    pub statuses: Option<String>,
    #[serde(default)]
    pub session_id: Option<i64>,
    #[serde(default)]
    pub active_only: bool,
}

impl From<ListActivitiesQuery> for ListActivitiesParams {
    fn from(query: ListActivitiesQuery) -> Self {
        Self {
            kinds: query.kinds,
            statuses: query.statuses,
            session_id: query.session_id,
            active_only: query.active_only,
        }
    }
}

pub async fn list_activities(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<ListActivitiesQuery>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::ListActivities(query.into()),
        |result| match result {
            agena_api::queries::QueryResult::Activities(items) => Some(items),
            _ => None,
        },
        "list_activities returned unexpected result",
    )
    .await
}

pub async fn get_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::GetActivity(GetActivityParams { activity_id }),
        |result| match result {
            agena_api::queries::QueryResult::Activity(activity) => Some(activity),
            _ => None,
        },
        "get_activity returned unexpected result",
    )
    .await
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ActivityLogsQuery {
    /// Cursor: lines with `seq > since_seq` are returned.
    #[serde(default)]
    pub since_seq: u64,
    /// Max lines to return (clamped server-side).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Block for fresh output up to this many ms when no new lines exist.
    #[serde(default)]
    pub wait_ms: u64,
}

pub async fn get_activity_logs(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
    AxumQuery(query): AxumQuery<ActivityLogsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::ActivityLogs(ActivityLogsParams {
            activity_id,
            since_seq: query.since_seq,
            limit: query.limit,
            wait_ms: query.wait_ms,
        }),
        |result| match result {
            agena_api::queries::QueryResult::ActivityLogs(logs) => Some(logs),
            _ => None,
        },
        "activity_logs returned unexpected result",
    )
    .await
}

pub async fn stop_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .stop_activity(&activity_id)
        .await
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityResource::from(&activity)))
}

pub async fn dismiss_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .dismiss_activity(&activity_id)
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityResource::from(&activity)))
}

pub async fn clear_finished_activities(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let count = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .clear_finished();
    Ok(Json(count))
}

fn activity_control_error(error: agena_runtime::ActivityControlError) -> ServerError {
    ServerError::bad_request_with_diagnostic(
        "The background activity operation failed.",
        error.to_string(),
    )
}
