use super::{
    AppState, AxumQuery, CommandRequestContext, Deserialize, IntoResponse, Json, Path,
    PluginLogListQuery, PluginSettingsUpdateRequest, PluginToolInvokeRequest, ServerError, State,
    json_http, server_error_from_application,
};
use agena_api::management::{PluginCommand, PluginQuery};

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for plugin tool registry changes.
pub struct PluginToolRegistryChangesQuery {
    #[serde(default)]
    pub after_generation: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::List),
    )
    .await
}

pub async fn get_plugin_architecture_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::Architecture),
    )
    .await
}

pub async fn get_plugin_surface_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::Surface),
    )
    .await
}

pub async fn list_plugin_tool_registry_changes(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<PluginToolRegistryChangesQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::ToolChanges {
                after_generation: query.after_generation,
                limit: query.limit,
            }),
    )
    .await
}

pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::Get { plugin_id }),
    )
    .await
}

pub async fn get_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::Settings { plugin_id }),
    )
    .await
}

pub async fn update_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginSettingsUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_plugin_management(PluginCommand::SetSettings {
                plugin_id,
                settings: request.value,
            }),
    )
    .await
}

pub async fn run_plugin_command(
    State(state): State<AppState>,
    Path((plugin_id, command_id)): Path<(String, String)>,
    Json(request): Json<CommandRequestContext>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_plugin_management(PluginCommand::InvokeCommand {
                plugin_id,
                command_id,
                input: request.input,
                session_id: request.session_id,
                slash: request.slash,
                raw: request.raw,
            }),
    )
    .await
}

pub async fn list_plugin_logs(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AxumQuery(query): AxumQuery<PluginLogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_plugin_management(PluginQuery::Logs {
                plugin_id,
                after_seq: query.after_seq,
                limit: query.limit,
            }),
    )
    .await
}

pub async fn invoke_plugin_tool(
    State(state): State<AppState>,
    Json(request): Json<PluginToolInvokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_id = request.plugin_id.as_deref().ok_or_else(|| {
        ServerError::bad_request("A plugin id is required for explicit plugin tool invocation.")
    })?;
    // Install the mutation observer before invoking a plugin that may publish
    // a change. No-op tools must not manufacture a new resource timestamp.
    let _revisions = state.revisions()?;
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
