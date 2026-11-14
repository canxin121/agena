//! HTTP adaptation of the shared notification use cases.

use super::{AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State};
use agena_api::resource::NotificationFilterParams;

pub async fn list_notifications(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<NotificationFilterParams>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.application().list_notifications(query).await?))
}

pub async fn dismiss_notification(
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    state
        .application()
        .dismiss_notification(notification_id)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Resolve an action entry point; clients execute the returned target.
pub async fn resolve_notification_action(
    State(state): State<AppState>,
    Path((notification_id, action_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .resolve_notification_action(notification_id, action_id)
            .await?,
    ))
}
