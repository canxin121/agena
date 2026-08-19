pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    Ok(items_json(state.plugin_runtime().plugin_statuses()))
}

pub async fn get_plugin_architecture_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.plugin_runtime().plugin_architecture_catalog()))
}

pub async fn get_plugin_surface_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_runtime = state.plugin_runtime();
    Ok(Json(PluginSurfaceCatalogResponse {
        catalog: plugin_runtime.plugin_surface_catalog(),
        permission_tools: plugin_runtime
            .permission_tool_catalog()
            .into_iter()
            .map(
                |tool| agena_application::dto::PermissionToolCatalogResource {
                    name: tool.name,
                    summary: tool.summary,
                    tags: tool.tags,
                },
            )
            .collect(),
        notifications: plugin_runtime.host_notifications(),
        activity_kinds: state.application().activity_kind_catalog(),
        tool_registry_generation: plugin_runtime.tool_registry_generation(),
        tool_registry_last_event: plugin_runtime
            .tool_registry_events_since(None, 1)
            .into_iter()
            .next(),
    }))
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for plugin tool registry changes.
pub struct PluginToolRegistryChangesQuery {
    #[serde(default)]
    pub after_generation: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn list_plugin_tool_registry_changes(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<PluginToolRegistryChangesQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_runtime = state.plugin_runtime();
    Ok(Json(serde_json::json!({
        "generation": plugin_runtime.tool_registry_generation(),
        "events": plugin_runtime.tool_registry_events_since(
            query.after_generation,
            query.limit.unwrap_or(100),
        ),
    })))
}

pub async fn invoke_plugin_tool(
    State(state): State<AppState>,
    Json(request): Json<PluginToolInvokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_id = request.plugin_id.as_deref().ok_or_else(|| {
        ServerError::bad_request("A plugin id is required for explicit plugin tool invocation.")
    })?;
    let response = state
        .application()
        .invoke_plugin_tool(
            plugin_id,
            request.tool.as_str(),
            request.context.input,
            request.context.session_id,
        )
        .await
        .map_err(server_error_from_application)?;
    Ok(Json(response))
}

pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin = state
        .plugin_runtime()
        .plugin_inspect(plugin_id.as_str())
        .ok_or_else(|| {
            ServerError::not_found_with_diagnostic(
                "The plugin was not found.",
                format!("plugin not found: {plugin_id}"),
            )
        })?;
    Ok(Json(PluginInspectResponse { plugin }))
}

pub async fn get_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let settings = state
        .application()
        .plugin_settings(plugin_id.as_str())
        .map_err(server_error_from_application)?;
    Ok(Json(settings))
}

pub async fn update_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginSettingsUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let response = state
        .application()
        .update_plugin_settings(plugin_id.as_str(), request.value)
        .await
        .map_err(server_error_from_application)?;
    Ok(Json(response))
}

pub async fn run_plugin_operation(
    State(state): State<AppState>,
    Path((plugin_id, operation_id)): Path<(String, String)>,
    Json(request): Json<PluginOperationRequestContext>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace_root = operation_workspace_root(&state, request.session_id).await?;
    let result = state
        .application()
        .invoke_plugin_operation(
            plugin_id.as_str(),
            operation_id.as_str(),
            request.input,
            request.session_id,
            workspace_root,
            request.slash,
            request.raw,
        )
        .await
        .map_err(server_error_from_application)?;
    Ok(Json(serde_json::json!({
        "plugin_id": plugin_id,
        "operation_id": operation_id,
        "result": result,
    })))
}

async fn operation_workspace_root(
    state: &AppState,
    session_id: Option<i64>,
) -> Result<Option<String>, ServerError> {
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let session = state
        .service()
        .get_session(session_id)
        .await
        .map_err(server_error_from_application)?
        .ok_or_else(|| {
            ServerError::not_found_with_diagnostic(
                "The session was not found.",
                format!("session not found: {session_id}"),
            )
        })?;
    let workspace = state
        .service()
        .get_workspace(session.workspace_id)
        .await
        .map_err(server_error_from_application)?
        .ok_or_else(|| {
            ServerError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {}", session.workspace_id),
            )
        })?;
    Ok(Some(workspace.path))
}

pub async fn list_plugin_logs(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AxumQuery(query): AxumQuery<PluginLogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_runtime = state.plugin_runtime();
    if !plugin_runtime
        .plugin_statuses()
        .iter()
        .any(|status| status.plugin_id.to_string() == plugin_id)
    {
        return Err(ServerError::not_found_with_diagnostic(
            "The plugin was not found.",
            format!("plugin not found: {plugin_id}"),
        ));
    }
    Ok(Json(PluginLogListResponse {
        plugin_id: plugin_id.clone(),
        logs: plugin_runtime.plugin_logs(
            plugin_id.as_str(),
            query.after_seq,
            query.limit.unwrap_or(50),
        ),
    }))
}

use super::{
    AppState, AxumQuery, Deserialize, IntoResponse, Json, Path, PluginInspectResponse,
    PluginLogListQuery, PluginLogListResponse, PluginOperationRequestContext,
    PluginSettingsUpdateRequest, PluginSurfaceCatalogResponse, PluginToolInvokeRequest,
    ServerError, State, items_json, server_error_from_application,
};
