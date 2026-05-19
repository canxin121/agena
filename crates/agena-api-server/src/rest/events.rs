use super::*;

pub async fn list_events(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListEventsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let publisher = state.event_publisher()?;
    let store: &Arc<dyn EventStore<agena::event::EventKind>> = publisher.store();
    let filter = agena::event::EventFilter {
        scope: params.scope,
        kinds: params.kinds,
        since_seq_global: params.since_seq_global,
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let range = StoreRange {
        after_seq_global: params.since_seq_global.unwrap_or(0),
        limit,
    };
    let events = store
        .range(&filter, range)
        .await
        .map_err(|error| ServerError::Internal(error.to_string()))?;
    let returned = events.len() as u64;
    let next_cursor = events.last().map(|event| event.meta.seq_global.to_string());
    Ok(Json(serde_json::json!({
        "items": events,
        "page": {
            "next_cursor": next_cursor,
            "has_more": (returned as usize) >= limit,
            "returned": returned,
        }
    })))
}
