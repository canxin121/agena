//! REST handlers for the unified notification surface (Phase 3).

use agena_api::pagination::{PageInfo, PaginatedResponse, normalize_limit};
use agena_api::resource::{
    NotificationActionTargetResource, NotificationFilterParams, NotificationResource,
};
use agena_notification::NotificationService;
use agena_notification::service::NotificationError;

use super::{AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State};

/// `GET /api/v1/notifications` — cursor-paginated active notifications.
pub async fn list_notifications(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<NotificationFilterParams>,
) -> Result<impl IntoResponse, ServerError> {
    let mut filter = query.clone().into_filter().map_err(|diagnostic| {
        ServerError::bad_request_with_diagnostic("The notification filter is invalid.", diagnostic)
    })?;
    let limit = normalize_limit(query.limit) as usize;
    filter.limit = Some(limit);
    let items = state
        .notifications()
        .list(filter)
        .await
        .map_err(notification_error)?;
    let returned = items.len() as u64;
    let next_cursor = items
        .last()
        .map(|notification| notification.created_at_ms.to_string());
    Ok(Json(PaginatedResponse {
        items: items.iter().map(NotificationResource::from).collect(),
        page: PageInfo {
            next_cursor,
            has_more: returned >= limit as u64,
            returned,
        },
    }))
}

/// `POST /api/v1/notifications/{notification_id}/dismiss` — dismiss a notification.
pub async fn dismiss_notification(
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    state
        .notifications()
        .dismiss(notification_id, None)
        .await
        .map_err(notification_error)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /api/v1/notifications/{notification_id}/actions/{action_id}` — resolve
/// an action entry point to its external target.
pub async fn resolve_notification_action(
    State(state): State<AppState>,
    Path((notification_id, action_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServerError> {
    let target = state
        .notifications()
        .resolve_target(notification_id, action_id)
        .await
        .map_err(notification_error)?;
    Ok(Json(NotificationActionTargetResource::from(&target)))
}

pub(crate) fn notification_error(error: NotificationError) -> ServerError {
    match error {
        NotificationError::NotFound(id) => ServerError::not_found_with_diagnostic(
            "The notification was not found.",
            format!("notification id: {id}"),
        ),
        NotificationError::Validation(message) => ServerError::bad_request_with_diagnostic(
            "The notification request is invalid.",
            message,
        ),
        NotificationError::Conflict(message) => ServerError::conflict_with_diagnostic(
            "The notification operation conflicts with the current state.",
            message,
        ),
        NotificationError::Unavailable(message) => ServerError::service_unavailable(message),
        other => ServerError::internal(other.to_string()),
    }
}
