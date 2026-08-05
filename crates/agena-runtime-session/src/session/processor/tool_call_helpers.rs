use super::{AppError, StructuredObject, ToolInvocation};
use agena_domain::{ToolApiCall, ToolApiFunction};
use agena_provider::ToolApiDefinition;

pub(crate) fn tool_execution_title(name: Option<&str>) -> String {
    name.unwrap_or("unknown").trim().to_owned()
}

/// Compose the call-start Operation title once the full invocation (including
/// streamed arguments) is available: "fs.read · README.md". The invocation's
/// most informative string argument supplies the summary so a running Activity
/// reads as a real call rather than a bare tool name.
pub(crate) fn tool_execution_title_for_invocation(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    let summary = agena_tool::invocation_call_summary(&input);
    agena_tool::compose_tool_title(invocation.name.as_str(), summary)
}

pub(crate) fn provider_native_tool_execution_title(
    title: &str,
    tool_name: &str,
    input: &StructuredObject,
) -> String {
    let trimmed = title.trim();
    if !trimmed.is_empty() {
        agena_tool::compose_tool_title(tool_name, trimmed)
    } else {
        let input = serde_json::Value::from(input.clone());
        let summary = agena_tool::invocation_call_summary(&input);
        agena_tool::compose_tool_title(tool_name, summary)
    }
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
            tool_api_call: None,
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
    let Some(function) = ToolApiFunction::from_function_name(tool.name.as_str()) else {
        return ToolInvocation::new(tool.name.clone(), input);
    };
    if function == ToolApiFunction::Call {
        let arguments = serde_json::Value::from(input.clone());
        // A `tools_call` without a usable `tool` target keeps the gateway's
        // own function name so the executor can reject it as an invalid call
        // instead of fabricating a phantom execution-tool name. `prepare_invocation`
        // turns this shape into `ToolError::InvalidInput` (missing `tool`).
        let target = arguments
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .unwrap_or_else(|| function.function_name())
            .to_owned();
        let target_input = arguments
            .get("input")
            .cloned()
            .and_then(|value| StructuredObject::try_from(value).ok())
            .unwrap_or_default();
        return ToolInvocation {
            tool_api_call: Some(ToolApiCall {
                function,
                arguments: input,
            }),
            name: target,
            plugin_name: None,
            input: target_input,
        };
    }
    ToolInvocation {
        tool_api_call: Some(ToolApiCall {
            function,
            arguments: input.clone(),
        }),
        name: tool.name.clone(),
        plugin_name: None,
        input,
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
                    tags: Vec::new(),
                },
            )
            .expect("registered tool"),
        )
        .expect("Tool API handler")
        .definition()
    }

    fn tools_call() -> agena_provider::ToolApiDefinition {
        ToolApiBinding::call_gateway().definition()
    }

    #[test]
    fn provider_function_names_are_matched_exactly() {
        let available = vec![tools_help()];
        let invocation = parse_tool_invocation_lossy(13, "tools_help", "{}", &available)
            .expect("declared exact name");
        assert_eq!(
            invocation.tool_api_call.as_ref().map(|call| call.function),
            Some(ToolApiFunction::Help)
        );
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
    fn tool_arguments_reject_trailing_non_json_content() {
        let error = parse_json_body::<serde_json::Value>(r#"{"tool":"session.rename"} trailing"#)
            .expect_err("trailing content must be rejected");
        assert!(error.to_string().contains("trailing characters"));
    }

    #[test]
    fn tools_call_resolves_to_the_real_execution_invocation() {
        let available = vec![tools_call()];
        let invocation = parse_tool_invocation_lossy(
            13,
            "tools_call",
            r#"{"tool":"fs.write","input":{"path":"notes.txt","content":"hello"}}"#,
            &available,
        )
        .expect("valid tools_call");

        assert_eq!(invocation.name, "fs.write");
        assert_eq!(
            invocation
                .input
                .get("path")
                .and_then(|value| value.as_text()),
            Some("notes.txt")
        );
        let call = invocation.tool_api_call.expect("provider envelope");
        assert_eq!(call.function, ToolApiFunction::Call);
        assert_eq!(
            call.arguments.get("tool").and_then(|value| value.as_text()),
            Some("fs.write")
        );
    }

    #[test]
    fn tools_call_without_a_tool_target_keeps_the_gateway_name() {
        let available = vec![tools_call()];
        for arguments_json in [
            "{}",
            r#"{"input":{}}"#,
            r#"{"tool":""}"#,
            r#"{"tool":"   "}"#,
            r#"{"tool":null,"input":{}}"#,
        ] {
            let invocation =
                parse_tool_invocation_lossy(13, "tools_call", arguments_json, &available)
                    .expect("tools_call args must parse");

            assert_eq!(
                invocation.name, "tools_call",
                "missing `tool` must keep the gateway name, not fabricate a phantom tool: {arguments_json}"
            );
            let call = invocation.tool_api_call.expect("provider envelope");
            assert_eq!(call.function, ToolApiFunction::Call);
        }
    }
}
