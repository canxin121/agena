pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    Ok(items_json(state.plugin_runtime().plugin_statuses()))
}

pub async fn get_plugin_ui_catalog(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_runtime = state.plugin_runtime();
    Ok(Json(PluginUiCatalogResponse {
        catalog: plugin_runtime.plugin_ui_catalog(),
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
    )
    .await?;
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

pub async fn run_plugin_ui_action(
    State(state): State<AppState>,
    Path((plugin_id, action_id)): Path<(String, String)>,
    Json(request): Json<PluginUiRequestContext>,
) -> Result<impl IntoResponse, ServerError> {
    let action = state
        .plugin_runtime()
        .resolve_studio_action(plugin_id.as_str(), action_id.as_str())
        .ok_or_else(|| {
            ServerError::not_found_with_diagnostic(
                "The plugin action was not found.",
                format!("plugin UI action not found: {plugin_id}/{action_id}"),
            )
        })?;

    let result: Option<serde_json::Value> = match &action {
        agena_plugin_host::PluginUiAction::InvokeTool { tool, input, .. } => {
            let output = invoke_plugin_tool_for_ui(
                &state,
                Some(plugin_id.as_str()),
                tool.as_str(),
                merge_plugin_ui_input(input.clone(), request.input),
                request.session_id,
            )
            .await?;
            Some(serde_json::to_value(output).map_err(|error| {
                ServerError::internal(format!("failed to encode plugin UI tool result: {error}"))
            })?)
        }
        agena_plugin_host::PluginUiAction::InvokeCommand { command, input } => {
            let session_id = request.session_id.ok_or_else(|| {
                ServerError::bad_request("This plugin action requires an active session.")
            })?;
            let session_services = state.application().session_execution_services()?;
            let output = session_services
                .plugin_commands
                .invoke_session_plugin_command(agena_runtime::SessionPluginCommandRequest {
                    session_id,
                    plugin_id: plugin_id.clone(),
                    command_id: command.clone(),
                    input: merge_plugin_ui_input(input.clone(), request.input),
                    slash: None,
                    raw: String::new(),
                    workspace_root: None,
                })
                .await
                .map_err(|error| ServerError::internal(error.to_string()))?;
            Some(serde_json::to_value(output).map_err(|error| {
                ServerError::internal(format!("failed to encode plugin command result: {error}"))
            })?)
        }
        agena_plugin_host::PluginUiAction::None
        | agena_plugin_host::PluginUiAction::OpenPluginWorkbench { .. }
        | agena_plugin_host::PluginUiAction::OpenUrl { .. }
        | agena_plugin_host::PluginUiAction::SubmitPrompt { .. } => None,
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

async fn invoke_plugin_tool_for_ui(
    state: &AppState,
    plugin_id: Option<&str>,
    tool_name: &str,
    input: serde_json::Value,
    session_id: Option<i64>,
) -> Result<agena_plugin_host::PluginUiToolInvokeResponse, ServerError> {
    let tool_name = tool_name.trim();
    if tool_name.is_empty() {
        return Err(ServerError::bad_request("The tool name cannot be empty."));
    }

    let session_id = session_id
        .ok_or_else(|| ServerError::bad_request("This plugin tool requires an active session."))?;
    let entry = state
        .plugin_runtime()
        .resolve_plugin_tool(plugin_id, tool_name)
        .ok_or_else(|| {
            let visible_name = plugin_id
                .and_then(|plugin_id| {
                    let normalize = |value: &str| {
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            return "tool".to_string();
                        }
                        let mut out = String::with_capacity(trimmed.len());
                        let mut previous_was_separator = false;
                        for ch in trimmed.chars() {
                            if ch.is_ascii_alphanumeric() || ch == '_' {
                                out.push(ch);
                                previous_was_separator = false;
                            } else if !previous_was_separator {
                                out.push('_');
                                previous_was_separator = true;
                            }
                        }
                        while out.ends_with('_') {
                            out.pop();
                        }
                        while out.starts_with('_') {
                            out.remove(0);
                        }
                        if out.is_empty() {
                            out.push_str("tool");
                        }
                        if out
                            .bytes()
                            .next()
                            .is_some_and(|byte| !byte.is_ascii_alphabetic() && byte != b'_')
                        {
                            out.insert(0, '_');
                        }
                        out
                    };
                    state
                        .plugin_runtime()
                        .plugin_inspect(plugin_id)
                        .and_then(|inspect| inspect.manifest)
                        .map(|manifest| {
                            format!(
                                "{}__{}",
                                normalize(manifest.name.as_str()),
                                normalize(tool_name)
                            )
                        })
                })
                .unwrap_or_else(|| tool_name.to_string());
            ServerError::not_found_with_diagnostic(
                "The plugin tool was not found.",
                format!("plugin tool not found: {visible_name}"),
            )
        })?;

    let input = match input {
        serde_json::Value::Null => serde_json::json!({}),
        serde_json::Value::Object(_) => input,
        other => {
            return Err(ServerError::bad_request_with_diagnostic(
                "The plugin tool input must be an object.",
                format!("plugin UI tool input must be an object, got {other}"),
            ));
        }
    };
    let structured = agena_domain::StructuredObject::try_from(input).map_err(|error| {
        ServerError::bad_request_with_diagnostic("The plugin tool input is invalid.", error)
    })?;
    let invocation = agena_domain::ToolInvocation::plugin_named(
        entry.canonical_name.clone(),
        entry.plugin_id.to_string(),
        structured,
    );
    let session_services = state.application().session_execution_services()?;
    let outcome = session_services
        .tool_execution
        .execute_session_tool(session_id, invocation)
        .await
        .map_err(|error| match error {
            agena_runtime::SessionToolExecutionError::Execution(error) => {
                ServerError::internal(error)
            }
        })?;
    let (status, title, output_text, payload, metadata) = match outcome {
        agena_runtime::SessionToolExecutionOutcome::Completed(summary) => (
            agena_plugin_host::PluginUiToolInvokeStatus::Completed,
            summary.title,
            summary.output_text,
            summary.payload,
            summary.metadata,
        ),
        agena_runtime::SessionToolExecutionOutcome::CapabilityUnavailable(unavailable) => (
            agena_plugin_host::PluginUiToolInvokeStatus::CapabilityUnavailable,
            "Capability unavailable".to_string(),
            format!(
                "The operation was not executed because the current runtime does not provide the required capability: {}",
                unavailable.reason
            ),
            Some(serde_json::json!({
                "status": "capability_unavailable",
                "code": "capability_unavailable",
                "retryable": unavailable.retryable,
                "unavailable": unavailable,
            })),
            Default::default(),
        ),
        agena_runtime::SessionToolExecutionOutcome::ToolUnavailable(unavailable) => (
            agena_plugin_host::PluginUiToolInvokeStatus::ToolUnavailable,
            "Tool unavailable".to_string(),
            format!(
                "The operation was not executed because the requested tool is unavailable: {}",
                unavailable.reason
            ),
            Some(serde_json::json!({
                "status": "tool_unavailable",
                "code": "tool_unavailable",
                "retryable": unavailable.retryable,
                "unavailable": unavailable,
            })),
            Default::default(),
        ),
    };
    Ok(agena_plugin_host::PluginUiToolInvokeResponse {
        plugin_id: entry.plugin_id,
        tool: entry.canonical_name,
        status,
        title,
        output_text,
        payload,
        metadata,
    })
}
use super::{
    AppState, AxumQuery, Deserialize, IntoResponse, Json, Path, PluginInspectResponse,
    PluginLogListQuery, PluginLogListResponse, PluginUiCatalogResponse, PluginUiInvokeToolRequest,
    PluginUiRequestContext, ServerError, State, items_json,
};
