use super::*;
use agena_api::mcp_apps::{
    McpAppCatalog, McpAppRequest, McpAppServerRequest, McpAppViewPublication,
};
use agena_domain::mcp_app_tools::{McpAppToolReply, McpAppViewBinding};
use axum::http::{StatusCode, header::CACHE_CONTROL};
use uuid::Uuid;

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
    let revision = app_revision(&headers)?;
    let result = state
        .application()
        .mcp_app_request(session_id, part_id, revision, request)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}

pub async fn request_mcp_app_server(
    State(state): State<AppState>,
    Path((session_id, part_id, view)): Path<(i64, i64, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<McpAppServerRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    let result = state
        .application()
        .mcp_app_server_request(binding, view, request)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppCatalogQuery {
    after: String,
}

pub async fn watch_mcp_app_catalog(
    State(state): State<AppState>,
    Path((session_id, part_id, catalog)): Path<(i64, i64, McpAppCatalog)>,
    AxumQuery(query): AxumQuery<McpAppCatalogQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let revision = app_revision(&headers)?;
    if !valid_revision(&query.after) {
        return Err(ServerError::bad_request(
            "The MCP App catalog revision is required.",
        ));
    }
    let result = state
        .application()
        .mcp_app_catalog_change(session_id, part_id, revision, catalog, &query.after)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}

pub async fn open_mcp_app_view(
    State(state): State<AppState>,
    Path((session_id, part_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    Json(publication): Json<McpAppViewPublication>,
) -> Result<impl IntoResponse, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    let result = state
        .application()
        .open_mcp_app_view(binding, publication)
        .await?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}

pub async fn update_mcp_app_view(
    State(state): State<AppState>,
    Path((session_id, part_id, view)): Path<(i64, i64, Uuid)>,
    headers: HeaderMap,
    Json(publication): Json<McpAppViewPublication>,
) -> Result<StatusCode, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    state
        .application()
        .mcp_app_views()?
        .publish(&binding, view, publication.tools)
        .await
        .map_err(agena_application::mcp_app_view_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn close_mcp_app_view(
    State(state): State<AppState>,
    Path((session_id, part_id, view)): Path<(i64, i64, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    state.application().mcp_app_views()?.close(&binding, view);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppViewCallsQuery {
    after: Option<u64>,
}

pub async fn poll_mcp_app_view_calls(
    State(state): State<AppState>,
    Path((session_id, part_id, view)): Path<(i64, i64, Uuid)>,
    AxumQuery(query): AxumQuery<McpAppViewCallsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    let result = state
        .application()
        .mcp_app_views()?
        .poll(&binding, view, query.after)
        .await
        .map_err(agena_application::mcp_app_view_error)?;
    Ok(([(CACHE_CONTROL, "private, no-store")], Json(result)))
}

pub async fn reply_mcp_app_view_call(
    State(state): State<AppState>,
    Path((session_id, part_id, view, call)): Path<(i64, i64, Uuid, Uuid)>,
    headers: HeaderMap,
    Json(reply): Json<McpAppToolReply>,
) -> Result<StatusCode, ServerError> {
    let binding = view_binding(session_id, part_id, &headers)?;
    state
        .application()
        .mcp_app_views()?
        .reply(&binding, view, call, reply)
        .map_err(agena_application::mcp_app_view_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn view_binding(
    session_id: i64,
    part_id: i64,
    headers: &HeaderMap,
) -> Result<McpAppViewBinding, ServerError> {
    Ok(McpAppViewBinding {
        session_id,
        part_id,
        revision: app_revision(headers)?.to_owned(),
    })
}

fn app_revision(headers: &HeaderMap) -> Result<&str, ServerError> {
    headers
        .get(IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix('"'))
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| valid_revision(value))
        .ok_or_else(|| ServerError::bad_request("The MCP App revision is required."))
}

fn valid_revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
