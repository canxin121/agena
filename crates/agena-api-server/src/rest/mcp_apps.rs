use super::*;
use agena_api::mcp_apps::McpAppRequest;
use axum::http::header::CACHE_CONTROL;

pub async fn get_mcp_app_document(
    State(state): State<AppState>,
    Path((session_id, part_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, ServerError> {
    let document = state
        .application()
        .mcp_app_document(session_id, part_id)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(document)))
}

pub async fn request_mcp_app(
    State(state): State<AppState>,
    Path((session_id, part_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    Json(request): Json<McpAppRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let revision = headers
        .get(IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix('"'))
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| ServerError::bad_request("The MCP App revision is required."))?;
    let result = state
        .application()
        .mcp_app_request(session_id, part_id, revision, request)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}
