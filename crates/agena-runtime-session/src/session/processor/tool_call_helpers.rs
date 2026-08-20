use super::{AppError, StructuredObject, ToolInvocation};
use agena_domain::{TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD, ToolApiCall, ToolApiFunction};
use agena_provider::ToolApiDefinition;

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
            if name == ToolApiFunction::Call.function_name()
                && let Some(diagnostic) = tools_call_arguments_diagnostic(arguments_json)
            {
                return Ok(tool_invocation_with_gateway_diagnostic(diagnostic));
            }
            Ok(placeholder_tool_invocation(Some(name), available_tools))
        }
    }
}

/// Build a `tools_call` placeholder that carries a precise corrective message
/// for the executor's gateway rejection. The message is stamped into the
/// provider envelope's arguments under
/// [`TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD`] so the executor can surface it
/// instead of the generic missing-`tool` text.
fn tool_invocation_with_gateway_diagnostic(message: String) -> ToolInvocation {
    let arguments = StructuredObject::try_from(serde_json::json!({
        TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD: message,
    }))
    .expect("single text field is a valid structured object");
    ToolInvocation {
        tool_api_call: Some(ToolApiCall {
            function: ToolApiFunction::Call,
            arguments,
        }),
        name: ToolApiFunction::Call.function_name().to_owned(),
        plugin_name: None,
        input: StructuredObject::default(),
    }
}

/// Produce a precise corrective message for malformed `tools_call` arguments,
/// or `None` when the executor's generic missing-`tool` rejection is the right
/// feedback (a parseable object that simply lacks a usable `tool` target).
fn tools_call_arguments_diagnostic(arguments_json: &str) -> Option<String> {
    match parse_json_body::<serde_json::Value>(arguments_json) {
        // The arguments parsed but were not the required object - for example
        // a JSON-encoded string. The model must pass the `{ tool, input }`
        // object itself, never a string that merely contains that JSON.
        Ok(value) if !value.is_object() => {
            let type_name = json_value_type_name(&value);
            let raw = bounded_json_preview(&value);
            if value.is_string() {
                Some(format!(
                    "tools_call arguments must be a JSON object, not a JSON {type_name}: pass \
                     `{{\"tool\": \"<execution-tool-name>\", \"input\": {{...}}}}` directly and \
                     never JSON-encode or stringify the arguments. Received: {raw}"
                ))
            } else {
                Some(format!(
                    "tools_call arguments must be a JSON object with a string `tool` field and an \
                     `input` object; received a JSON {type_name} instead of an object: {raw}"
                ))
            }
        }
        // The arguments did not parse as JSON at all (invalid escapes such as
        // `\|`, truncation, trailing content, ...). Surface the actual parse
        // error instead of the generic missing-`tool` message.
        Err(parse_error) => Some(format!(
            "tools_call arguments did not parse as valid JSON ({parse_error}). Arguments must be \
             one JSON object `{{\"tool\": \"<execution-tool-name>\", \"input\": {{...}}}}` with \
             correct quoting and escapes - never a JSON-encoded string, no invalid escapes (such \
             as \\|), no truncation. Received: {raw}",
            raw = bounded_text_preview(arguments_json)
        )),
        // A parseable object falls through to the executor's generic
        // missing-`tool` rejection.
        Ok(_) => None,
    }
}

fn json_value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn bounded_json_preview(value: &serde_json::Value) -> String {
    bounded_text_preview(&serde_json::to_string(value).unwrap_or_default())
}

fn bounded_text_preview(text: &str) -> String {
    const MAX: usize = 160;
    if text.chars().count() <= MAX {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(MAX).collect();
        format!("{truncated}...")
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

    #[test]
    fn tools_call_with_string_arguments_carries_a_shape_diagnostic() {
        let available = vec![tools_call()];
        // The model passed a JSON-encoded string instead of the { tool, input }
        // object itself.
        let arguments_json = r#""{\"tool\":\"fs.read\",\"input\":{\"path\":\"README.md\"}}""#;
        let invocation = parse_tool_invocation_lossy(13, "tools_call", arguments_json, &available)
            .expect("string arguments must produce a diagnostic invocation");
        assert_eq!(invocation.name, "tools_call");
        let call = invocation.tool_api_call.expect("provider envelope");
        let diagnostic = call
            .arguments
            .get(agena_domain::TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD)
            .and_then(|value| value.as_text())
            .expect("shape diagnostic must be stamped");
        assert!(
            diagnostic.contains("JSON string"),
            "expected string-shape diagnostic, got: {diagnostic}"
        );
        assert!(
            diagnostic.contains("never JSON-encode"),
            "expected stringify guidance, got: {diagnostic}"
        );
    }

    #[test]
    fn tools_call_with_invalid_json_arguments_carries_a_parse_diagnostic() {
        let available = vec![tools_call()];
        // Session 66 regression: `\|` is an invalid JSON escape inside the
        // command string, so the whole arguments value fails to parse.
        let arguments_json = r#"{"tool":"shell.run","input":{"command":"grep -rn 'a\|b' src"}}"#;
        let invocation = parse_tool_invocation_lossy(13, "tools_call", arguments_json, &available)
            .expect("malformed arguments must produce a diagnostic invocation");
        assert_eq!(invocation.name, "tools_call");
        let call = invocation.tool_api_call.expect("provider envelope");
        let diagnostic = call
            .arguments
            .get(agena_domain::TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD)
            .and_then(|value| value.as_text())
            .expect("parse diagnostic must be stamped");
        assert!(
            diagnostic.contains("did not parse as valid JSON"),
            "expected parse diagnostic, got: {diagnostic}"
        );
        assert!(
            diagnostic.contains("invalid escape"),
            "expected the serde escape detail, got: {diagnostic}"
        );
    }

    #[test]
    fn tools_call_with_array_arguments_carries_a_shape_diagnostic() {
        let available = vec![tools_call()];
        let invocation = parse_tool_invocation_lossy(13, "tools_call", "[1,2,3]", &available)
            .expect("non-object arguments must produce a diagnostic invocation");
        let call = invocation.tool_api_call.expect("provider envelope");
        let diagnostic = call
            .arguments
            .get(agena_domain::TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD)
            .and_then(|value| value.as_text())
            .expect("shape diagnostic must be stamped");
        assert!(
            diagnostic.contains("array"),
            "expected array-shape diagnostic, got: {diagnostic}"
        );
    }
}
