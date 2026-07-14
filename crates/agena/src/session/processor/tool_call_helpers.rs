use super::{AppError, RegisteredTool, StructuredObject, ToolInvocation};

pub(crate) fn tool_execution_title(name: Option<&str>) -> String {
    format!("Tool {}", name.unwrap_or("unknown").trim())
}

pub(crate) fn provider_tool_execution_title(
    title: &str,
    tool_name: &str,
    input: &StructuredObject,
) -> String {
    let trimmed = title.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }

    let invocation = ToolInvocation::new(tool_name.to_owned(), input.clone());
    tool_invocation_label(&invocation)
}

pub(crate) fn placeholder_tool_invocation(
    name: Option<&str>,
    available_tools: &[RegisteredTool],
) -> ToolInvocation {
    let requested_name = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let Some(tool) = available_tools
        .iter()
        .find(|tool| crate::tool::tool_matches_model_name(tool, requested_name))
    else {
        return ToolInvocation {
            name: requested_name.to_string(),
            plugin_name: None,
            input: StructuredObject::default(),
        };
    };

    tool_invocation_for_definition(tool, StructuredObject::default())
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[RegisteredTool],
) -> Result<ToolInvocation, AppError> {
    let trimmed_name = name.trim();
    let tool = tool_for_model_name(trimmed_name, available_tools).ok_or_else(|| {
        AppError::Provider(format!("unsupported tool call from model: {trimmed_name}"))
    })?;

    let parsed = parse_custom_input(arguments_json)?;
    Ok(tool_invocation_for_definition(tool, parsed))
}

pub(crate) fn parse_tool_invocation_lossy(
    session_id: i64,
    name: &str,
    arguments_json: &str,
    available_tools: &[RegisteredTool],
) -> ToolInvocation {
    let trimmed_name = name.trim();
    if tool_for_model_name(trimmed_name, available_tools).is_none() {
        tracing::debug!(
            target: "agena::session::processor",
            session_id,
            tool = %trimmed_name,
            "model requested unsupported tool; preserving call for tool-failure handling"
        );
        return placeholder_tool_invocation(Some(trimmed_name), available_tools);
    }

    match parse_tool_invocation(trimmed_name, arguments_json, available_tools) {
        Ok(invocation) => invocation,
        Err(err) => {
            tracing::warn!(
                target: "agena::session::processor",
                session_id,
                tool = %trimmed_name,
                error = %err,
                arguments_len = arguments_json.len(),
                "tool arguments could not be parsed; falling back to empty input for tool-failure handling"
            );
            placeholder_tool_invocation(Some(trimmed_name), available_tools)
        }
    }
}

pub(crate) fn tool_for_model_name<'a>(
    name: &str,
    available_tools: &'a [RegisteredTool],
) -> Option<&'a RegisteredTool> {
    available_tools
        .iter()
        .find(|tool| crate::tool::tool_matches_model_name(tool, name))
}

pub(crate) fn tool_definition_identity_from_model_name(
    name: &str,
    available_tools: &[RegisteredTool],
) -> Option<String> {
    tool_for_model_name(name, available_tools).map(RegisteredTool::definition_identity)
}

pub(crate) fn canonical_tool_name_from_model_name(
    name: &str,
    available_tools: &[RegisteredTool],
) -> String {
    tool_for_model_name(name, available_tools)
        .map(RegisteredTool::model_name)
        .unwrap_or_else(|| name.trim().to_owned())
}

pub(crate) fn tool_invocation_for_definition(
    tool: &RegisteredTool,
    input: StructuredObject,
) -> ToolInvocation {
    ToolInvocation {
        name: tool.model_name(),
        plugin_name: Some(tool.plugin_full_name().clone()),
        input,
    }
}

pub(crate) fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    if let Some(gateway_name) = gateway_model_tool_name(invocation.name.as_str())
        && let Some(target) = input.get("tool").and_then(serde_json::Value::as_str)
        && !target.trim().is_empty()
    {
        return format!("{gateway_name} {}", target.trim());
    }
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "action",
        "id",
        "expression",
        "notebook_path",
    ] {
        if let Some(value) = input.get(key).and_then(serde_json::Value::as_str)
            && !value.trim().is_empty()
        {
            return format!("{} {}", invocation.name, value.trim());
        }
    }
    invocation.name.clone()
}

pub(crate) fn gateway_model_tool_name(name: &str) -> Option<&'static str> {
    match name.trim() {
        "agena.tools.list" | "tools.list" | "tools_list" => Some("tools_list"),
        "agena.tools.search" | "tools.search" | "tools_search" => Some("tools_search"),
        "agena.tools.help" | "tools.help" | "tools_help" => Some("tools_help"),
        "agena.tools.tags" | "tools.tags" | "tools_tags" => Some("tools_tags"),
        "agena.tools.call" | "tools.call" | "tools_call" => Some("tools_call"),
        _ => None,
    }
}

pub(crate) fn parse_custom_input(arguments_json: &str) -> Result<StructuredObject, AppError> {
    let value = parse_json_body::<serde_json::Value>(arguments_json)?;
    StructuredObject::try_from(value)
        .map_err(|err| AppError::Internal(format!("invalid custom tool input: {err}")))
}

pub(crate) fn parse_json_body<T>(arguments_json: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    let body = if arguments_json.trim().is_empty() {
        "{}"
    } else {
        arguments_json
    };

    let mut deserializer = serde_json::Deserializer::from_str(body);
    let parsed =
        <T as serde::Deserialize>::deserialize(&mut deserializer).map_err(AppError::from)?;

    if let Err(err) = deserializer.end() {
        tracing::warn!(
            error = %err,
            arguments_len = body.len(),
            "tool arguments included trailing content; ignored suffix after valid JSON prefix"
        );
    }

    Ok(parsed)
}
