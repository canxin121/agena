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

    // The placeholder uses a freshly constructed object, so this can only
    // fail if an internal Tool API binding violates its own schema. Keep the
    // placeholder path total, but record the invariant failure instead of
    // silently substituting a different invocation.
    match tool_invocation_for_definition(tool, StructuredObject::default()) {
        Ok(invocation) => invocation,
        Err(error) => {
            tracing::error!(
                tool = %tool.name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "construct a placeholder Tool API invocation",
                    &error,
                ),
                "Tool API placeholder invocation fell back to an unbound shape"
            );
            ToolInvocation {
                tool_api_call: None,
                name: requested_name.to_string(),
                plugin_name: None,
                input: StructuredObject::default(),
            }
        }
    }
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[ToolApiDefinition],
) -> Result<ToolInvocation, AppError> {
    let tool = tool_api_binding_for_name(name, available_tools)
        .ok_or_else(|| AppError::Provider(format!("unsupported tool call from model: {name:?}")))?;

    let parsed = parse_custom_input(arguments_json)?;
    tool_invocation_for_definition(tool, parsed)
}

pub(crate) fn parse_tool_invocation_lossy(
    session_id: i64,
    name: &str,
    arguments_json: &str,
    available_tools: &[ToolApiDefinition],
) -> Result<ToolInvocation, AppError> {
    if tool_api_binding_for_name(name, available_tools).is_none() {
        tracing::warn!(
            target: "agena::session::processor",
            session_id,
            tool = %name,
            "provider returned an unknown Tool API function; executing it as an unknown tool failure"
        );
        return Ok(placeholder_tool_invocation(Some(name), available_tools));
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
        // A parseable object with an object-shaped `input` falls through to the
        // executor's generic missing-`tool` rejection. When `input` itself has
        // the wrong JSON shape, surface that instead: a stringified `input` is
        // the common model mistake, and the generic missing-`tool` message
        // would hide it.
        Ok(value) => {
            let object = value.as_object().expect("non-object handled above");
            let input = object.get("input")?;
            if input.is_object() {
                return None;
            }
            let type_name = json_value_type_name(input);
            let raw = bounded_json_preview(input);
            let tool = object
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("<unknown>");
            Some(if input.is_string() {
                format!(
                    "tools_call `input` for tool `{tool}` must be a JSON object, not a \
                     JSON-encoded string. Pass the tool parameters directly as an object inside \
                     `{{\"tool\": \"{tool}\", \"input\": {{...}}}}`; never stringify the inner \
                     arguments. Received: {raw}"
                )
            } else {
                format!(
                    "tools_call `input` for tool `{tool}` must be a JSON object with the tool \
                     parameters; received a JSON {type_name} instead. Received: {raw}"
                )
            })
        }
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
    match serde_json::to_string(value) {
        Ok(value) => bounded_text_preview(&value),
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize malformed tool-call value for diagnostic preview",
                    &error,
                ),
                "tool-call diagnostic preview could not be serialized"
            );
            "<tool-call value could not be serialized>".to_owned()
        }
    }
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

pub(crate) fn tool_invocation_for_definition(
    tool: &ToolApiDefinition,
    input: StructuredObject,
) -> Result<ToolInvocation, AppError> {
    let Some(function) = ToolApiFunction::from_function_name(tool.name.as_str()) else {
        return Ok(ToolInvocation::new(tool.name.clone(), input));
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
        let target_input = match arguments.get("input").cloned() {
            Some(value) => StructuredObject::try_from(value).map_err(|error| {
                AppError::Internal(format!(
                    "decode the nested input of a tools_call invocation: {error}"
                ))
            })?,
            None => StructuredObject::default(),
        };
        return Ok(ToolInvocation {
            tool_api_call: Some(ToolApiCall {
                function,
                arguments: input,
            }),
            name: target,
            plugin_name: None,
            input: target_input,
        });
    }
    Ok(ToolInvocation {
        tool_api_call: Some(ToolApiCall {
            function,
            arguments: input.clone(),
        }),
        name: tool.name.clone(),
        plugin_name: None,
        input,
    })
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
