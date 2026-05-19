use super::*;

pub async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_messages(manager.as_ref(), session_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_message(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageDetailQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let message = state
        .service()
        .get_message(manager.as_ref(), message_id, query.parts)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("message not found: {message_id}")))?;
    Ok(Json(message))
}

pub async fn list_message_parts(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessagePartsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_message_parts(manager.as_ref(), message_id, query.mode)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_message_part(
    State(state): State<AppState>,
    Path(part_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let part = state
        .service()
        .get_message_part(manager.as_ref(), part_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("message part not found: {part_id}")))?;
    Ok(Json(part))
}
