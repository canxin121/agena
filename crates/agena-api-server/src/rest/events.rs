pub async fn list_events(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListEventsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let events = state.application().event_query_service()?;
    let filter = agena_domain::EventFilter {
        scope: match params.scope {
            agena_api::Scope::Global => agena_domain::EventScope::Global,
            agena_api::Scope::Workspace { workspace_id } => {
                agena_domain::EventScope::Workspace { workspace_id }
            }
            agena_api::Scope::Session { session_id } => {
                agena_domain::EventScope::Session { session_id }
            }
        },
        kinds: params.kinds,
        since_seq_global: params.since_seq_global,
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let range = agena_runtime::RuntimeEventRange {
        after_seq_global: params.since_seq_global.unwrap_or(0),
        limit,
    };
    let events = events
        .list_events(&filter, range)
        .await
        .map_err(|error| ServerError::from_failure(*error.failure))?;
    let returned = events.len() as u64;
    let next_cursor = events.last().map(|event| event.meta.seq_global.to_string());
    Ok(Json(serde_json::json!({
        "items": events.iter().map(agena_application::event_projection::event_resource_from_runtime).collect::<Vec<_>>(),
        "page": {
            "next_cursor": next_cursor,
            "has_more": (returned as usize) >= limit,
            "returned": returned,
        }
    })))
}
use super::{AppState, AxumQuery, IntoResponse, Json, ListEventsParams, ServerError, State};
