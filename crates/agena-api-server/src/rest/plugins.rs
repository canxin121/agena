use super::*;

pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    Ok(items_json(
        state
            .runtime()
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses(),
    ))
}

pub async fn get_plugin_ui_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let plugin_manager = snapshot.plugin_manager();
    Ok(Json(PluginUiCatalogResponse {
        catalog: plugin_manager.ui_catalog(),
        tool_registry_generation: plugin_manager.tool_registry_generation(),
        tool_registry_last_event: plugin_manager
            .tool_registry_events_since(None, 1)
            .into_iter()
            .next(),
    }))
}

#[derive(Debug, Clone, Deserialize, Default)]
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
    let plugin_manager = state.runtime().current_snapshot().plugin_manager();
    Ok(Json(serde_json::json!({
        "generation": plugin_manager.tool_registry_generation(),
        "events": plugin_manager.tool_registry_events_since(
            query.after_generation,
            query.limit.unwrap_or(100),
        ),
    })))
}

pub async fn invoke_plugin_ui_tool(
    State(state): State<AppState>,
    Json(request): Json<PluginUiInvokeToolRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let response = invoke_plugin_tool_for_ui(
        &state,
        request.plugin_id.as_deref(),
        request.tool.as_str(),
        request
            .context
            .input
            .unwrap_or_else(|| serde_json::json!({})),
        request.context.session_id,
    )?;
    Ok(Json(response))
}

pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin = state
        .runtime()
        .current_snapshot()
        .plugin_manager()
        .plugin_inspect(plugin_id.as_str())
        .ok_or_else(|| ServerError::NotFound(format!("plugin not found: {plugin_id}")))?;
    Ok(Json(PluginInspectResponse { plugin }))
}

pub async fn run_plugin_ui_action(
    State(state): State<AppState>,
    Path((plugin_id, action_id)): Path<(String, String)>,
    Json(request): Json<PluginUiRequestContext>,
) -> Result<impl IntoResponse, ServerError> {
    let host = state.runtime().current_snapshot().plugin_manager();
    let action = host
        .resolve_studio_action(plugin_id.as_str(), action_id.as_str())
        .ok_or_else(|| {
            ServerError::NotFound(format!(
                "plugin UI action not found: {plugin_id}/{action_id}"
            ))
        })?;

    let result = match &action {
        agena::plugin::PluginUiAction::InvokeTool { tool, input, .. } => {
            Some(invoke_plugin_tool_for_ui(
                &state,
                Some(plugin_id.as_str()),
                tool.as_str(),
                merge_plugin_ui_input(input.clone(), request.input),
                request.session_id,
            )?)
        }
        agena::plugin::PluginUiAction::None
        | agena::plugin::PluginUiAction::OpenRoute { .. }
        | agena::plugin::PluginUiAction::OpenUrl { .. }
        | agena::plugin::PluginUiAction::SubmitPrompt { .. } => None,
    };

    Ok(Json(serde_json::json!({
        "plugin_id": plugin_id,
        "action_id": action_id,
        "action": action,
        "result": result,
    })))
}

pub async fn list_plugin_logs(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AxumQuery(query): AxumQuery<PluginLogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_manager = state.runtime().current_snapshot().plugin_manager();
    if plugin_manager.plugin_status(plugin_id.as_str()).is_none() {
        return Err(ServerError::NotFound(format!(
            "plugin not found: {plugin_id}"
        )));
    }
    Ok(Json(PluginLogListResponse {
        plugin_id: plugin_id.clone(),
        logs: plugin_manager.plugin_logs(
            plugin_id.as_str(),
            query.after_seq,
            query.limit.unwrap_or(50),
        ),
    }))
}

fn merge_plugin_ui_input(
    base: Option<serde_json::Value>,
    overlay: Option<serde_json::Value>,
) -> serde_json::Value {
    match (base, overlay) {
        (Some(serde_json::Value::Object(mut base)), Some(serde_json::Value::Object(overlay))) => {
            base.extend(overlay);
            serde_json::Value::Object(base)
        }
        (_, Some(value)) => value,
        (Some(value), None) => value,
        (None, None) => serde_json::json!({}),
    }
}

fn invoke_plugin_tool_for_ui(
    state: &AppState,
    plugin_id: Option<&str>,
    tool_name: &str,
    input: serde_json::Value,
    session_id: Option<i64>,
) -> Result<agena::plugin::PluginUiToolInvokeResponse, ServerError> {
    let tool_name = tool_name.trim();
    if tool_name.is_empty() {
        return Err(ServerError::BadRequest("tool cannot be empty".to_string()));
    }

    let manager = state.session_manager()?;
    let snapshot = state.runtime().current_snapshot();
    let host = snapshot.plugin_manager();
    let entry = match plugin_id {
        Some(plugin_id) => host.resolve_registered_tool_for_plugin_tool(plugin_id, tool_name),
        None => host.lookup_tool(tool_name),
    }
    .ok_or_else(|| {
        let visible_name = plugin_id
            .and_then(|plugin_id| {
                state
                    .runtime()
                    .current_snapshot()
                    .plugin_manager()
                    .plugin_inspect(plugin_id)
                    .and_then(|inspect| inspect.manifest)
                    .map(|manifest| {
                        format!(
                            "{}__{}",
                            agena::plugin::registry::model_tool_name_segment(
                                manifest.name.as_str()
                            ),
                            agena::plugin::registry::model_tool_name_segment(tool_name),
                        )
                    })
            })
            .unwrap_or_else(|| tool_name.to_string());
        ServerError::NotFound(format!("plugin tool not found: {visible_name}"))
    })?;

    let input = match input {
        serde_json::Value::Null => serde_json::json!({}),
        serde_json::Value::Object(_) => input,
        other => {
            return Err(ServerError::BadRequest(format!(
                "plugin UI tool input must be an object, got {other}"
            )));
        }
    };
    let structured =
        agena::message::StructuredObject::try_from(input).map_err(ServerError::BadRequest)?;
    let invocation = agena::message::ToolInvocation::with_plugin_name(
        entry.model_name.clone(),
        entry.target.plugin_name.clone(),
        structured,
    );
    let executor = manager.tool_executor();

    for check in executor
        .collect_permission_checks_for_invocation(&invocation)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
    {
        match check.decision {
            agena::permission::PermissionDecision::Allow => {}
            agena::permission::PermissionDecision::Ask { reason } => {
                return Err(ServerError::Conflict(format!(
                    "plugin UI tool requires permission confirmation: {reason}"
                )));
            }
            agena::permission::PermissionDecision::Deny { reason } => {
                return Err(ServerError::Conflict(format!(
                    "plugin UI tool denied by permission policy: {reason}"
                )));
            }
        }
    }

    let execution = executor
        .execute_invocation_detailed(&invocation, session_id.unwrap_or(-1), -1)
        .map_err(|error| ServerError::Internal(error.to_string()))?;
    Ok(agena::plugin::PluginUiToolInvokeResponse {
        plugin_id: entry.target.plugin_name,
        tool: entry.model_name,
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload: execution.output.to_json_payload(),
        metadata: execution.view.metadata,
    })
}
