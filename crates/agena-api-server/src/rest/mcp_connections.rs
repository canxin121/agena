//! Authenticated management of Agena's connections to external MCP services.
use super::*;
use agena_api::mcp_connections::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpRevisionQuery {
    pub expected_revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCapabilitiesQuery {
    pub kind: String,
    pub cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAuthQuery {
    pub flow_id: Option<String>,
}

pub async fn list_mcp_connections(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().mcp_connections()).await
}
pub async fn get_mcp_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().mcp_connection(id)).await
}
pub async fn create_mcp_connection(
    State(state): State<AppState>,
    Json(request): Json<McpConnectionCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().create_mcp_connection(request)).await
}
pub async fn update_mcp_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpConnectionUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().update_mcp_connection(id, request)).await
}
pub async fn delete_mcp_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<McpRevisionQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .remove_mcp_connection(id, query.expected_revision),
    )
    .await
}
pub async fn set_mcp_connection_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpConnectionEnabledRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().set_mcp_connection_enabled(id, request)).await
}
pub async fn set_mcp_bridge_enabled(
    State(state): State<AppState>,
    Json(request): Json<McpBridgeEnabledRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().set_mcp_bridge_enabled(request)).await
}
pub async fn preview_mcp_import(
    State(state): State<AppState>,
    Json(request): Json<McpImportPreviewRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().preview_mcp_import(request)).await
}
pub async fn import_mcp_connections(
    State(state): State<AppState>,
    Json(request): Json<McpImportRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().import_mcp_connections(request)).await
}
pub async fn reconnect_mcp_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().reconnect_mcp_connection(id)).await
}
pub async fn test_mcp_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().test_mcp_connection(id)).await
}
pub async fn mcp_connection_capabilities(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<McpCapabilitiesQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .mcp_connection_capabilities(id, query.kind, query.cursor),
    )
    .await
}
pub async fn read_mcp_connection_resource(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpResourceReadRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .read_mcp_connection_resource(id, request),
    )
    .await
}
pub async fn get_mcp_connection_prompt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpPromptGetRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().get_mcp_connection_prompt(id, request)).await
}
pub async fn set_mcp_connection_credential(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpCredentialRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .set_mcp_connection_credential(id, Some(request.token)),
    )
    .await
}
pub async fn clear_mcp_connection_credential(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().set_mcp_connection_credential(id, None)).await
}
pub async fn start_mcp_connection_auth(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpOAuthStartRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().start_mcp_connection_auth(id, request)).await
}
pub async fn complete_mcp_connection_auth(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpOAuthCompleteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .complete_mcp_connection_auth(id, request),
    )
    .await
}
pub async fn clear_mcp_connection_auth(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<McpAuthQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .clear_mcp_connection_auth(id, query.flow_id),
    )
    .await
}
