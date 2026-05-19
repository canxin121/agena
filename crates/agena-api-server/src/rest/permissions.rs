use super::*;

pub async fn list_permission_rules(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<PermissionRuleListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_permission_rules(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let rule = state
        .service()
        .get_permission_rule(rule_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("permission rule not found: {rule_id}")))?;
    Ok(Json(rule))
}

pub async fn create_permission_rule(
    State(state): State<AppState>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_permission_rule(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_permission_rule(rule_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn revoke_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleRevokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .revoke_permission_rule(rule_id, request.reason)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_permission_rule(rule_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}
