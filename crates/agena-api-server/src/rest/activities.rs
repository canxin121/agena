//! REST handlers for the unified background-activity surface.

use agena_api::queries::ListActivitiesParams;
use agena_api::resource::{BackgroundActivityLogResource, BackgroundActivityResource};
use serde::Deserialize;

use super::{
    AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State,
    server_error_from_application,
};

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing activities over REST.
pub struct ListActivitiesQuery {
    /// Comma-separated kind filters (`shell`, `monitor`, `task`, `cron`,
    /// `runtime`, `browser`).
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
    let service = state
        .application()
        .runtime_activities()
        .map_err(server_error_from_application)?;
    let params: ListActivitiesParams = query.into();
    let activities = service
        .list_activities(&agena_application::service::activity_filter_from_params(
            &params,
        ))
        .await
        .map_err(activity_control_error)?;
    let resources: Vec<BackgroundActivityResource> = activities
        .iter()
        .map(BackgroundActivityResource::from)
        .collect();
    Ok(Json(resources))
}

pub async fn get_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .application()
        .runtime_activities()
        .map_err(server_error_from_application)?
        .get_activity(&activity_id)
        .await
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityResource::from(&activity)))
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for reading activity logs over REST.
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
    let read = state
        .application()
        .runtime_activities()
        .map_err(server_error_from_application)?
        .activity_logs(&activity_id, query.since_seq, query.limit, query.wait_ms)
        .await
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityLogResource::from(read)))
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

pub async fn pause_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .pause_activity(&activity_id)
        .await
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityResource::from(&activity)))
}

pub async fn resume_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .resume_activity(&activity_id)
        .await
        .map_err(activity_control_error)?;
    Ok(Json(BackgroundActivityResource::from(&activity)))
}

pub async fn delete_activity(
    State(state): State<AppState>,
    Path(activity_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let activity = state
        .runtime_activities()
        .map_err(server_error_from_application)?
        .delete_activity(&activity_id)
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
        .clear_finished()
        .await
        .map_err(activity_control_error)?;
    Ok(Json(count))
}

fn activity_control_error(error: agena_runtime::ActivityControlError) -> ServerError {
    ServerError::bad_request_with_diagnostic(
        "The background activity operation failed.",
        error.to_string(),
    )
}
