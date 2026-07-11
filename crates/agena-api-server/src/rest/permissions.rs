pub async fn list_permission_rules(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SearchPaginationQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_permission_rules(query)).await
}

pub async fn get_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http_found(state.service().get_permission_rule(rule_id), || {
        format!("permission rule not found: {rule_id}")
    })
    .await
}

pub async fn create_permission_rule(
    State(state): State<AppState>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().create_permission_rule(request)).await
}

pub async fn replace_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().replace_permission_rule(rule_id, request)).await
}

pub async fn revoke_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleRevokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .service()
            .revoke_permission_rule(rule_id, request.reason),
    )
    .await
}

pub async fn delete_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().delete_permission_rule(rule_id)).await
}
use super::{
    AppState, AxumQuery, IntoResponse, Json, Path, PermissionRuleRevokeRequest,
    PermissionRuleWriteRequest, SearchPaginationQuery, ServerError, State, json_http,
    json_http_found,
};
