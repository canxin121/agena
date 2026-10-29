use super::{AppError, StructuredObject};
use agena_domain::{ToolApiCall, ToolApiFunction};
use agena_provider::ToolApiDefinition;
use agena_tool_api::{ToolApiError, normalize_tool_call_input};

#[derive(Clone)]
pub(crate) struct ParsedToolApiCall {
    pub call: ToolApiCall,
    pub error: Option<ToolApiError>,
}

/// Decode only the model protocol. Executable plugin invocations are created
/// later, at the explicit tools_call dispatch boundary.
pub(crate) fn parse_tool_api_call_lossy(
    name: &str,
    arguments_json: &str,
    available_functions: &[ToolApiDefinition],
) -> Result<ParsedToolApiCall, AppError> {
    let function = ToolApiFunction::from_function_name(name)
        .filter(|_| {
            available_functions
                .iter()
                .any(|definition| definition.name == name)
        })
        .ok_or_else(|| {
            AppError::Provider(format!(
                "unsupported Tool API function from model: {name:?}"
            ))
        })?;
    let parsed = parse_json_body::<serde_json::Value>(arguments_json);
    let error = if function == ToolApiFunction::Call {
        tools_call_arguments_diagnostic(&parsed, arguments_json).map(ToolApiError)
    } else {
        match &parsed {
            Ok(value) if value.is_object() => None,
            Ok(value) => Some(ToolApiError(format!(
                "{name} arguments must be a JSON object; received a JSON {}: {}",
                json_value_type_name(value),
                bounded_json_preview(value),
            ))),
            Err(error) => Some(ToolApiError(format!(
                "{name} arguments did not parse as valid JSON ({error}). Pass one JSON object. Received: {}",
                bounded_text_preview(arguments_json),
            ))),
        }
    };
    let arguments = parsed
        .ok()
        .and_then(|value| StructuredObject::try_from(value).ok())
        .unwrap_or_default();
    Ok(ParsedToolApiCall {
        call: ToolApiCall {
            function,
            arguments,
        },
        error,
    })
}

fn tools_call_arguments_diagnostic(
    parsed: &Result<serde_json::Value, AppError>,
    arguments_json: &str,
) -> Option<String> {
    match parsed {
        // The arguments parsed but were not the required object - for example
        // a JSON-encoded string. The model must pass the `{ tool, input }`
        // object itself, never a string that merely contains that JSON.
        Ok(value) if !value.is_object() => {
            let type_name = json_value_type_name(value);
            let raw = bounded_json_preview(value);
            if value.is_string() {
                Some(format!(
                    "tools_call arguments must be a JSON object, not a JSON {type_name}: pass \
                     `{{\"tool\": \"<execution-tool-name>\", \"input\": {{...}}}}` directly and \
                     never JSON-encode or stringify the arguments. Received: {raw}"
                ))
            } else {
                Some(format!(
                    "tools_call arguments must be a JSON object with a string `tool` field and an \
                     `input` object or JSON string encoding that object; received a JSON \
                     {type_name} instead of an object: {raw}"
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
        // Validate string-shaped `input` without changing the raw protocol call.
        // Dispatch normalizes the execution input, which is still subject to the
        // selected tool's schema validation.
        Ok(value) => {
            let object = value.as_object().expect("non-object handled above");
            let input = object.get("input")?;
            if input.is_object() {
                return None;
            }
            let tool = object
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("<unknown>");
            match normalize_tool_call_input(tool, input.clone()) {
                Ok(_) => None,
                Err(error) => Some(format!(
                    "{error}. Received: {}",
                    bounded_json_preview(input)
                )),
            }
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
mod protocol_parsing_tests {
    use super::*;

    #[test]
    fn protocol_parsing_keeps_the_envelope_until_dispatch() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        let parsed = parse_tool_api_call_lossy(
            "tools_call",
            r#"{"tool":"fs.read","input":{"file_path":"a.txt"}}"#,
            &definitions,
        )
        .unwrap();
        assert_eq!(parsed.call.function, ToolApiFunction::Call);
        assert!(parsed.error.is_none());
        assert_eq!(
            serde_json::Value::from(parsed.call.arguments)["tool"],
            "fs.read"
        );
        assert!(parse_tool_api_call_lossy("fs.read", "{}", &definitions).is_err());
    }

    #[test]
    fn malformed_arguments_produce_a_protocol_failure_without_an_execution_placeholder() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        for arguments in [r#""{}""#, "[]", r#"{"tool":"fs.read""#] {
            let parsed = parse_tool_api_call_lossy("tools_call", arguments, &definitions).unwrap();
            assert_eq!(parsed.call.function, ToolApiFunction::Call);
            assert!(parsed.error.is_some());
            assert!(parsed.call.arguments.is_empty());
        }
    }

    #[test]
    fn malformed_discovery_arguments_report_the_correct_protocol_function() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        for function in [
            ToolApiFunction::List,
            ToolApiFunction::Help,
            ToolApiFunction::PluginsList,
        ] {
            for arguments in ["[]", r#"{"tool":"fs.read""#] {
                let parsed =
                    parse_tool_api_call_lossy(function.function_name(), arguments, &definitions)
                        .unwrap();
                let error = parsed.error.unwrap().to_string();
                assert!(error.contains(function.function_name()));
                assert!(!error.contains("tools_call"));
            }
        }
    }

    #[test]
    fn stringified_execution_input_preserves_raw_protocol_and_dispatches_an_object() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        let input = serde_json::json!({
            "command": "printf '%s\\n' '\"quoted\"'\nprintf '%s' '你好'",
            "nested": {"items": [null, true, 7, "\\|"]},
        });
        let arguments = serde_json::json!({"tool":"shell.exec", "input":input.to_string()});
        let parsed =
            parse_tool_api_call_lossy("tools_call", &arguments.to_string(), &definitions).unwrap();
        assert!(parsed.error.is_none());
        assert_eq!(
            serde_json::Value::from(parsed.call.arguments.clone()),
            arguments,
        );
        let agena_tool_api::ToolApiAction::Execute(request) =
            agena_tool_api::ToolApiCatalog::default()
                .dispatch(&parsed.call)
                .unwrap()
        else {
            panic!("expected execution boundary");
        };
        assert_eq!(request.tool, "shell.exec");
        assert_eq!(serde_json::Value::from(request.input), input);
    }

    #[test]
    fn invalid_stringified_input_reports_json_failure_and_preserves_raw_protocol() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        for input in ["", "{", r#"{"command":"\|"}"#, "{} trailing"] {
            let arguments = serde_json::json!({"tool":"shell.exec", "input":input});
            let parsed =
                parse_tool_api_call_lossy("tools_call", &arguments.to_string(), &definitions)
                    .unwrap();
            assert_eq!(serde_json::Value::from(parsed.call.arguments), arguments);
            let error = parsed.error.unwrap().to_string();
            assert!(error.contains("shell.exec"), "{error}");
            assert!(error.contains("invalid JSON"), "{error}");
        }
    }

    #[test]
    fn input_strings_must_decode_once_to_objects() {
        let definitions = agena_tool_api::ToolApiRegistry.definitions();
        for input in ["[]", "null", "true", "7", r#""{}""#] {
            let arguments = serde_json::json!({"tool":"shell.exec", "input":input});
            let parsed =
                parse_tool_api_call_lossy("tools_call", &arguments.to_string(), &definitions)
                    .unwrap();
            let error = parsed.error.unwrap().to_string();
            assert!(error.contains("must decode to a JSON object"), "{error}");
        }
        for input in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(7),
            serde_json::json!([]),
        ] {
            let arguments = serde_json::json!({"tool":"shell.exec", "input":input});
            let parsed =
                parse_tool_api_call_lossy("tools_call", &arguments.to_string(), &definitions)
                    .unwrap();
            let error = parsed.error.unwrap().to_string();
            assert!(
                error.contains("JSON object or a JSON string encoding an object"),
                "{error}"
            );
        }
    }
}
