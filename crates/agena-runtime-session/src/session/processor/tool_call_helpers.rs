use super::{AppError, StructuredObject, ToolInvocation};
use agena_domain::ToolApiFunction;
use agena_provider::ToolApiDefinition;

pub(crate) fn tool_execution_title(name: Option<&str>) -> String {
    format!("Tool {}", name.unwrap_or("unknown").trim())
}

pub(crate) fn provider_native_tool_execution_title(
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
    available_tools: &[ToolApiDefinition],
) -> ToolInvocation {
    let requested_name = name.filter(|value| !value.is_empty()).unwrap_or("unknown");
    let Some(tool) = available_tools
        .iter()
        .find(|tool| tool.name == requested_name)
    else {
        return ToolInvocation {
            tool_api_function: None,
            provider_function_name: None,
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
    available_tools: &[ToolApiDefinition],
) -> Result<ToolInvocation, AppError> {
    let tool = tool_api_binding_for_name(name, available_tools)
        .ok_or_else(|| AppError::Provider(format!("unsupported tool call from model: {name:?}")))?;

    let parsed = parse_custom_input(arguments_json)?;
    Ok(tool_invocation_for_definition(tool, parsed))
}

pub(crate) fn parse_tool_invocation_lossy(
    session_id: i64,
    name: &str,
    arguments_json: &str,
    available_tools: &[ToolApiDefinition],
) -> Result<ToolInvocation, AppError> {
    if tool_api_binding_for_name(name, available_tools).is_none() {
        return Err(AppError::Provider(format!(
            "provider returned unknown Tool API function {name:?} in session {session_id}"
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

pub(crate) fn tool_api_binding_for_name<'a>(
    name: &str,
    available_tools: &'a [ToolApiDefinition],
) -> Option<&'a ToolApiDefinition> {
    available_tools.iter().find(|tool| tool.name == name)
}

pub(crate) fn tool_api_definition_identity(
    name: &str,
    available_tools: &[ToolApiDefinition],
) -> Option<String> {
    tool_api_binding_for_name(name, available_tools).map(|tool| tool.definition_identity.clone())
}

pub(crate) fn tool_invocation_for_definition(
    tool: &ToolApiDefinition,
    input: StructuredObject,
) -> ToolInvocation {
    let function = ToolApiFunction::from_function_name(tool.name.as_str());
    if let Some(execution_tool) = tool.execution_tool.as_deref() {
        return ToolInvocation {
            tool_api_function: None,
            provider_function_name: Some(tool.name.clone()),
            name: execution_tool.to_string(),
            plugin_name: None,
            input,
        };
    }
    ToolInvocation {
        tool_api_function: function,
        provider_function_name: None,
        name: tool.name.clone(),
        plugin_name: None,
        input,
    }
}

pub(crate) fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    if let Some(function_name) = invocation
        .tool_api_function
        .map(ToolApiFunction::function_name)
        && let Some(tool_name) = input.get("tool").and_then(serde_json::Value::as_str)
        && !tool_name.trim().is_empty()
    {
        return format!("{function_name} {}", tool_name.trim());
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

    deserializer.end().map_err(AppError::from)?;

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{parse_json_body, parse_tool_invocation_lossy};
    use crate::tool::ToolApiBinding;
    use agena_domain::ToolApiFunction;
    use agena_plugin_host::registry::RegisteredTool;
    use agena_plugin_host::sdk::{PluginKey, ToolDefinition};

    fn tools_help() -> agena_provider::ToolApiDefinition {
        ToolApiBinding::from_registered_tool(
            RegisteredTool::new(
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
            )
            .expect("registered tool"),
        )
        .expect("Tool API handler")
        .definition()
    }

    #[test]
    fn provider_function_names_are_matched_exactly() {
        let available = vec![tools_help()];
        let invocation = parse_tool_invocation_lossy(13, "tools_help", "{}", &available)
            .expect("declared exact name");
        assert_eq!(invocation.tool_api_function, Some(ToolApiFunction::Help));
        assert_eq!(invocation.name, "tools_help");
        assert_eq!(invocation.plugin_name, None);

        for invalid in [
            "agena.tools.help",
            "tools.help",
            " tools_help",
            "tools_help ",
        ] {
            let error = parse_tool_invocation_lossy(13, invalid, "{}", &available)
                .expect_err("aliases and whitespace must not be accepted");
            assert!(error.to_string().contains("unknown Tool API function"));
        }
    }

    #[test]
    fn direct_provider_function_maps_to_execution_tool_and_keeps_replay_name() {
        let mut direct = tools_help();
        direct.name = "fs_read".to_string();
        direct.execution_tool = Some("fs.read".to_string());
        let invocation =
            parse_tool_invocation_lossy(13, "fs_read", r#"{"path":"Cargo.toml"}"#, &[direct])
                .expect("direct tool invocation");
        assert_eq!(invocation.name, "fs.read");
        assert_eq!(
            invocation.provider_function_name.as_deref(),
            Some("fs_read")
        );
        assert_eq!(invocation.tool_api_function, None);
    }

    #[test]
    fn tool_arguments_reject_trailing_non_json_content() {
        let error = parse_json_body::<serde_json::Value>(r#"{"tool":"session.rename"} trailing"#)
            .expect_err("trailing content must be rejected");
        assert!(error.to_string().contains("trailing characters"));
    }
}
