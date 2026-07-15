use super::{AppError, StructuredObject, ToolInvocation};
use crate::tool::GatewayToolBinding;
use crate::tool_protocol::GatewayFunction;

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
    available_tools: &[GatewayToolBinding],
) -> ToolInvocation {
    let requested_name = name.filter(|value| !value.is_empty()).unwrap_or("unknown");
    let Some(tool) = available_tools
        .iter()
        .find(|tool| tool.protocol_name() == requested_name)
    else {
        return ToolInvocation {
            gateway_function: None,
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
    available_tools: &[GatewayToolBinding],
) -> Result<ToolInvocation, AppError> {
    let tool = gateway_binding_for_protocol_name(name, available_tools)
        .ok_or_else(|| AppError::Provider(format!("unsupported tool call from model: {name:?}")))?;

    let parsed = parse_custom_input(arguments_json)?;
    Ok(tool_invocation_for_definition(tool, parsed))
}

pub(crate) fn parse_tool_invocation_lossy(
    session_id: i64,
    name: &str,
    arguments_json: &str,
    available_tools: &[GatewayToolBinding],
) -> Result<ToolInvocation, AppError> {
    if gateway_binding_for_protocol_name(name, available_tools).is_none() {
        return Err(AppError::Provider(format!(
            "provider returned undeclared gateway function {name:?} in session {session_id}"
        )));
    }

    match parse_tool_invocation(name, arguments_json, available_tools) {
        Ok(invocation) => Ok(invocation),
        Err(err) => {
            tracing::warn!(
                target: "agena::session::processor",
                session_id,
                tool = %name,
                error = %err,
                arguments_len = arguments_json.len(),
                "tool arguments could not be parsed; falling back to empty input for tool-failure handling"
            );
            Ok(placeholder_tool_invocation(Some(name), available_tools))
        }
    }
}

pub(crate) fn gateway_binding_for_protocol_name<'a>(
    name: &str,
    available_tools: &'a [GatewayToolBinding],
) -> Option<&'a GatewayToolBinding> {
    available_tools
        .iter()
        .find(|tool| tool.protocol_name() == name)
}

pub(crate) fn gateway_definition_identity(
    name: &str,
    available_tools: &[GatewayToolBinding],
) -> Option<String> {
    gateway_binding_for_protocol_name(name, available_tools)
        .map(|tool| tool.handler().definition_identity())
}

pub(crate) fn tool_invocation_for_definition(
    tool: &GatewayToolBinding,
    input: StructuredObject,
) -> ToolInvocation {
    ToolInvocation {
        gateway_function: Some(tool.function()),
        name: tool.canonical_name(),
        plugin_name: Some(tool.handler().plugin_full_name()),
        input,
    }
}

pub(crate) fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    if let Some(gateway_name) = invocation
        .gateway_function
        .or_else(|| GatewayFunction::from_handler_name(invocation.name.as_str()))
        .map(GatewayFunction::protocol_name)
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

#[cfg(test)]
mod tests {
    use super::parse_tool_invocation_lossy;
    use crate::plugin::registry::RegisteredTool;
    use crate::plugin::sdk::{PluginKey, ToolDefinition};
    use crate::tool::GatewayToolBinding;
    use crate::tool_protocol::GatewayFunction;

    fn tools_help() -> GatewayToolBinding {
        GatewayToolBinding::from_registered_tool(RegisteredTool::new(
            PluginKey::new("agena", "tools").expect("plugin key"),
            ToolDefinition {
                name: "help".to_owned(),
                contract: Default::default(),
                model: Default::default(),
                docs: Default::default(),
                runtime: Default::default(),
                permissions: Default::default(),
                display: Default::default(),
                capabilities: Vec::new(),
            },
        ))
        .expect("gateway provider tool")
    }

    #[test]
    fn provider_function_names_are_matched_exactly() {
        let available = vec![tools_help()];
        let invocation = parse_tool_invocation_lossy(13, "tools_help", "{}", &available)
            .expect("declared exact name");
        assert_eq!(
            invocation.gateway_function,
            Some(GatewayFunction::ToolsHelp)
        );
        assert_eq!(invocation.name, "agena.tools.help");

        for invalid in [
            "agena.tools.help",
            "tools.help",
            " tools_help",
            "tools_help ",
        ] {
            let error = parse_tool_invocation_lossy(13, invalid, "{}", &available)
                .expect_err("aliases and whitespace must not be accepted");
            assert!(error.to_string().contains("undeclared gateway function"));
        }
    }
}
