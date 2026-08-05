use futures_util::StreamExt;
use std::collections::{BTreeMap, BTreeSet};

use agena_provider::{CompletionToolCall, CompletionUsage, ProviderCompactionOutput};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, Instant, ModelRef, ModelRuntime,
    ProviderError, ProviderRegistry, Stream, elapsed_ms, hydrate_usage_cost_from_provider_metadata,
    retry_reason, stream_resume_policy_label, validate_request_capabilities,
};

const MAX_TOOL_API_REPAIRS: usize = 2;
const MAX_REJECTED_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_REJECTED_CALLS_IN_REPAIR: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RejectedToolApiCall {
    name: String,
    arguments_json: String,
}

fn completion_tool_api_calls(response: &CompletionResponse) -> Vec<RejectedToolApiCall> {
    response
        .tool_calls
        .iter()
        .map(|call| match call {
            CompletionToolCall::Function {
                name,
                arguments_json,
                ..
            } => RejectedToolApiCall {
                name: name.clone(),
                arguments_json: arguments_json.clone(),
            },
        })
        .collect()
}

fn stream_tool_api_calls(
    calls: &BTreeMap<String, StreamToolApiCallState>,
) -> Vec<RejectedToolApiCall> {
    calls
        .values()
        .map(|call| RejectedToolApiCall {
            name: call.name.clone().unwrap_or_else(|| "<missing>".to_owned()),
            arguments_json: call.arguments_json.clone(),
        })
        .collect()
}

fn bounded_arguments(arguments_json: &str) -> String {
    if arguments_json.len() <= MAX_REJECTED_ARGUMENT_BYTES {
        return arguments_json.to_owned();
    }
    let mut end = MAX_REJECTED_ARGUMENT_BYTES;
    while !arguments_json.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated]", &arguments_json[..end])
}

fn rejected_calls_json(calls: &[RejectedToolApiCall]) -> String {
    let omitted = calls.len() > MAX_REJECTED_CALLS_IN_REPAIR;
    let calls = calls
        .iter()
        .take(MAX_REJECTED_CALLS_IN_REPAIR)
        .map(|call| {
            serde_json::json!({
                "name": call.name,
                "arguments_json": bounded_arguments(call.arguments_json.as_str()),
            })
        })
        .collect::<Vec<_>>();
    let mut rendered = serde_json::to_string(&calls).unwrap_or_else(|_| "[]".to_owned());
    if omitted {
        rendered.push_str(" [additional rejected calls omitted]");
    }
    rendered
}

fn tool_name_repair_guidance(
    calls: &[RejectedToolApiCall],
    declared: &BTreeSet<String>,
) -> Vec<String> {
    calls
        .iter()
        .filter_map(|call| {
            if call.name == "tools_call" {
                let arguments = serde_json::from_str::<serde_json::Value>(
                    call.arguments_json.as_str(),
                )
                .ok()?;
                let tool_name = arguments.get("tool")?.as_str()?;
                let api_function =
                    agena_domain::ToolApiFunction::from_function_name(tool_name)?;
                let function_name = api_function.function_name();
                if !declared.contains(function_name) {
                    return None;
                }
                let direct_arguments = arguments
                    .get("input")
                    .filter(|value| value.is_object())
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                return Some(format!(
                    "- `{tool_name}` identifies Tool API function `{function_name}`, not an execution tool. Call function `{function_name}` directly with arguments {}; never put a Tool API function name inside `tools_call.arguments.tool`.",
                    serde_json::to_string(&direct_arguments)
                        .unwrap_or_else(|_| "{}".to_owned())
                ));
            }
            if let Some(api_function) = tool_api_identity(call.name.as_str()) {
                let function_name = api_function.function_name();
                if !declared.contains(function_name) {
                    return None;
                }
                let direct_arguments = serde_json::from_str::<serde_json::Value>(
                    call.arguments_json.as_str(),
                )
                .ok()
                .filter(serde_json::Value::is_object)
                .unwrap_or_else(|| serde_json::json!({}));
                return Some(format!(
                    "- `{}` is an internal handler key for Tool API function `{function_name}`. Retry with function name `{function_name}` and arguments {}; never route a Tool API function through `tools_call`.",
                    call.name,
                    serde_json::to_string(&direct_arguments)
                        .unwrap_or_else(|_| "{}".to_owned())
                ));
            }
            None
        })
        .collect()
}

fn append_tool_api_repair_turn(
    request: &mut CompletionRequest,
    error: &ProviderError,
    calls: &[RejectedToolApiCall],
    declared: &BTreeSet<String>,
) {
    let guidance = tool_name_repair_guidance(calls, declared);
    let declared = declared.iter().cloned().collect::<Vec<_>>().join(", ");
    let guidance = if guidance.is_empty() {
        String::new()
    } else {
        format!(
            "\nRequired routing for the rejected execution-tool name(s):\n{}",
            guidance.join("\n")
        )
    };
    request.messages.push(agena_provider::CompletionInputMessage {
        role: agena_domain::Role::User,
        parts: vec![agena_provider::CompletionInputPart::Text { text: format!(
            "Trusted Agena transport correction: the original user's task is still unresolved. The preceding Tool API call was rejected before execution. It produced no tool result and must not be reported as successful.\nError: {error}\nRejected calls: {}\nThe only allowed Tool API function names are: [{declared}].{guidance}\nRetry the unresolved tool step now. Emit an exact declared Tool API function call; do not answer the user's task, narrate a call, invent a result, or repeat an execution-tool name as `function.name`.",
            rejected_calls_json(calls),
        ) }],
        provider_state: Default::default(),
    });
    request.previous_response_id = None;
    request.temperature = Some(0.0);
}

fn merge_completion_usage(
    target: &mut Option<CompletionUsage>,
    additional: Option<CompletionUsage>,
) {
    let Some(additional) = additional else {
        return;
    };
    let Some(target) = target.as_mut() else {
        *target = Some(additional);
        return;
    };
    target.add_assign(&additional);
}

fn tool_api_repair_exhausted(provider_id: &str) -> ProviderError {
    ProviderError::Provider(format!(
        "provider `{provider_id}` could not produce a valid Agena Tool API call after {MAX_TOOL_API_REPAIRS} internal repair attempts"
    ))
}

fn declared_tool_api_functions(request: &CompletionRequest) -> BTreeSet<String> {
    request
        .tool_api_functions
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

fn apply_configured_tool_mode(
    model: &ModelRef,
    provider: &dyn ModelRuntime,
    request: &mut CompletionRequest,
) {
    let mode = provider.agena_tool_mode_for_adapter(model.adapter_id.as_ref(), &model.model_id);
    let provider_native = provider
        .provider_native_tools_config_for_adapter(model.adapter_id.as_ref(), &model.model_id);
    agena_provider::apply_configured_tool_request(mode, &provider_native, request);
}

fn validate_provider_native_tool_definition_boundary(
    request: &CompletionRequest,
) -> Result<(), ProviderError> {
    const RESERVED_FIELDS: [&str; 2] = ["tools", "functions"];
    let overridden = RESERVED_FIELDS
        .into_iter()
        .filter(|field| request.request_override.body_patch.contains_key(*field))
        .collect::<Vec<_>>();
    if !overridden.is_empty() {
        return Err(ProviderError::Config(format!(
            "request_override.body_patch cannot override provider-native tool-definition field(s) {}; declare Agena Tool API functions through CompletionRequest.tool_api_functions and provider-hosted tools through provider_native_tools",
            overridden
                .into_iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let mut declared = BTreeSet::new();
    for tool in &request.tool_api_functions {
        let is_gateway =
            agena_domain::ToolApiFunction::from_function_name(tool.name.as_str()).is_some();
        if !is_gateway {
            return Err(ProviderError::Config(format!(
                "provider-bound tool {:?} is not one of the five Tool API gateway functions",
                tool.name
            )));
        }
        if !has_valid_provider_function_name(tool.name.as_str()) {
            return Err(ProviderError::Config(format!(
                "provider-bound tool name {:?} contains unsupported characters",
                tool.name
            )));
        }
        if !declared.insert(tool.name.clone()) {
            return Err(ProviderError::Config(format!(
                "Tool API function `{}` is declared more than once",
                tool.name
            )));
        }
        let schema = tool.input_schema.as_object().ok_or_else(|| {
            ProviderError::Config(format!(
                "provider-bound Tool API function `{}` must use an object input schema",
                tool.name
            ))
        })?;
        if schema.get("type").and_then(serde_json::Value::as_str) != Some("object") {
            return Err(ProviderError::Config(format!(
                "provider-bound Tool API function `{}` must use an object input schema",
                tool.name
            )));
        }
        if schema
            .get("properties")
            .is_some_and(|properties| !properties.is_object())
        {
            return Err(ProviderError::Config(format!(
                "provider-bound Tool API function `{}` has non-object schema properties",
                tool.name
            )));
        }
        if schema.get("required").is_some_and(|required| {
            required
                .as_array()
                .is_none_or(|items| items.iter().any(|item| !item.is_string()))
        }) {
            return Err(ProviderError::Config(format!(
                "provider-bound Tool API function `{}` has a non-string schema required list",
                tool.name
            )));
        }
    }
    Ok(())
}

fn has_valid_provider_function_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn validate_returned_tool_api_function(
    provider_id: &str,
    name: &str,
    declared: &BTreeSet<String>,
) -> Result<(), ProviderError> {
    if declared.contains(name) {
        return Ok(());
    }
    let declared = declared.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(ProviderError::Provider(format!(
        "provider `{provider_id}` returned unknown Tool API function {name:?}; declared functions: [{declared}]"
    )))
}

/// Parse Tool API arguments with the same tolerance the session processor
/// applies when it materializes a call. Providers (notably deepseek through
/// an OpenAI-compatible gateway) frequently emit arguments with a stray
/// invalid escape such as `\d`; the strict serde parse rejects the whole
/// call and sends it back to the model for protocol repair, which interrupts
/// the agent loop and, after repeated failures, ends the run early. Repairing
/// the most common defect (escaping a backslash that precedes a character
/// JSON strings do not allow) keeps the call flowing into normal tool
/// execution, where a genuinely malformed input fails as an ordinary tool
/// result instead of aborting the run.
fn parse_tool_api_arguments_tolerant(arguments_json: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments_json) {
        return Some(value);
    }

    let mut repaired = String::with_capacity(arguments_json.len());
    let mut chars = arguments_json.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            repaired.push(ch);
            continue;
        }
        match chars.peek() {
            Some(&next) if matches!(next, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') => {
                repaired.push(ch);
                repaired.push(next);
                chars.next();
            }
            _ => {
                // Invalid escape: keep the backslash literal so the string
                // value is preserved instead of dropping the argument.
                repaired.push('\\');
                repaired.push('\\');
            }
        }
    }

    serde_json::from_str::<serde_json::Value>(&repaired).ok()
}

fn validate_tool_api_arguments(
    provider_id: &str,
    name: &str,
    arguments_json: &str,
) -> Result<(), ProviderError> {
    let Some(arguments) = parse_tool_api_arguments_tolerant(arguments_json) else {
        let detail = serde_json::from_str::<serde_json::Value>(arguments_json)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unparseable arguments".to_owned());
        return Err(ProviderError::Provider(format!(
            "provider `{provider_id}` returned invalid JSON arguments for Tool API function `{name}`: {detail}"
        )));
    };
    let arguments = arguments.as_object().ok_or_else(|| {
        ProviderError::Provider(format!(
            "provider `{provider_id}` returned non-object arguments for Tool API function `{name}`"
        ))
    })?;
    validate_tool_api_argument_semantics(provider_id, name, arguments)
}

fn tool_api_identity(value: &str) -> Option<agena_domain::ToolApiFunction> {
    agena_domain::ToolApiFunction::from_function_name(value)
}

fn has_valid_tool_name_syntax(name: &str) -> bool {
    name == name.trim()
        && name.contains('.')
        && name.split('.').all(|segment| !segment.is_empty())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_execution_tool_name(
    provider_id: &str,
    function: &str,
    tool_name: &str,
) -> Result<(), ProviderError> {
    if let Some(api_function) = tool_api_identity(tool_name) {
        return Err(ProviderError::Provider(format!(
            "provider `{provider_id}` placed Tool API function `{}` inside `{function}.arguments.tool`; call `{}` directly instead",
            api_function.function_name(),
            api_function.function_name(),
        )));
    }
    if has_valid_tool_name_syntax(tool_name) {
        return Ok(());
    }
    Err(ProviderError::Provider(format!(
        "provider `{provider_id}` returned invalid execution-tool name {tool_name:?} for Tool API function `{function}`; use an exact name such as `fs.read` returned by `tools_list` or `tools_search`"
    )))
}

fn validate_tool_api_argument_semantics(
    provider_id: &str,
    name: &str,
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ProviderError> {
    use agena_domain::ToolApiFunction;

    let Some(function) = ToolApiFunction::from_function_name(name) else {
        return Ok(());
    };
    match function {
        ToolApiFunction::Help => {
            let tool_name = arguments
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    ProviderError::Provider(format!(
                        "provider `{provider_id}` returned `tools_help` without a string execution-tool name in `arguments.tool`"
                    ))
                })?;
            validate_execution_tool_name(provider_id, name, tool_name)
        }
        ToolApiFunction::Call => {
            let tool_name = arguments
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    ProviderError::Provider(format!(
                        "provider `{provider_id}` returned `tools_call` without a string execution-tool name in `arguments.tool`"
                    ))
                })?;
            validate_execution_tool_name(provider_id, name, tool_name)?;
            if !arguments
                .get("input")
                .is_some_and(serde_json::Value::is_object)
            {
                return Err(ProviderError::Provider(format!(
                    "provider `{provider_id}` returned `tools_call` without a complete object in `arguments.input`"
                )));
            }
            Ok(())
        }
        ToolApiFunction::List
        | ToolApiFunction::Search
        | ToolApiFunction::Tags
        | ToolApiFunction::PluginsList
        | ToolApiFunction::PluginsSearch
        | ToolApiFunction::PluginsTags => Ok(()),
    }
}

fn validate_completion_tool_calls(
    provider_id: &str,
    response: &CompletionResponse,
    declared: &BTreeSet<String>,
) -> Result<(), ProviderError> {
    for call in &response.tool_calls {
        let CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } = call;
        validate_returned_tool_api_function(provider_id, name, declared)?;
        validate_tool_api_arguments(provider_id, name, arguments_json)?;
    }
    Ok(())
}

#[derive(Default)]
struct StreamToolApiCallState {
    name: Option<String>,
    arguments_json: String,
}

fn validate_stream_tool_api_event(
    provider_id: &str,
    event: &CompletionStreamEvent,
    declared: &BTreeSet<String>,
    calls: &mut BTreeMap<String, StreamToolApiCallState>,
) -> Result<(), ProviderError> {
    match event {
        CompletionStreamEvent::ToolCallDelta {
            stream_key,
            name,
            arguments_delta,
            ..
        } => {
            let state = calls.entry(stream_key.clone()).or_default();
            if let Some(name) = name {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` changed Tool API function name for stream `{stream_key}`"
                    )));
                }
                state.name = Some(name.clone());
                state.arguments_json.push_str(arguments_delta);
                return Ok(());
            }
            state.arguments_json.push_str(arguments_delta);
            Ok(())
        }
        CompletionStreamEvent::ToolCallSnapshot {
            stream_key,
            name,
            arguments_json,
            ..
        } => {
            let state = calls.entry(stream_key.clone()).or_default();
            if let Some(name) = name {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` changed Tool API function name for stream `{stream_key}`"
                    )));
                }
                state.name = Some(name.clone());
                state.arguments_json.clone_from(arguments_json);
                return Ok(());
            }
            state.arguments_json.clone_from(arguments_json);
            Ok(())
        }
        CompletionStreamEvent::Completed { .. } => {
            for (stream_key, state) in calls.iter() {
                let name = state.name.as_deref().ok_or_else(|| {
                    ProviderError::Provider(format!(
                        "provider `{provider_id}` completed Tool API call `{stream_key}` without a function name"
                    ))
                })?;
                validate_returned_tool_api_function(provider_id, name, declared)?;
                validate_tool_api_arguments(provider_id, name, state.arguments_json.as_str())?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl ProviderRegistry {
    pub async fn execute_image(
        &self,
        model: &ModelRef,
        request: agena_provider::ProviderImageRequest,
    ) -> Result<agena_provider::ProviderImageResponse, ProviderError> {
        let provider = self.provider_for_model_ref(model)?;
        let configured_route = provider
            .provider_native_tools_config_for_adapter(model.adapter_id.as_ref(), &model.model_id)
            .routes
            .route_for(agena_provider::ProviderNativeToolKind::ImageGeneration);
        if configured_route != Some(agena_provider::ProviderNativeToolRoute::ProviderHosted) {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{}` does not enable the provider-hosted image_generation route",
                model.provider_id, model.model_id
            )));
        }
        let capabilities = provider
            .image_capabilities_for_adapter(model.adapter_id.as_ref(), &model.model_id)
            .ok_or_else(|| {
                ProviderError::Config(format!(
                    "provider `{}` model `{}` has no active direct image route",
                    model.provider_id, model.model_id
                ))
            })?;
        if !capabilities.supports(request.operation) {
            return Err(ProviderError::Config(format!(
                "provider `{}` model `{}` does not support the requested direct image operation",
                model.provider_id, model.model_id
            )));
        }
        self.call_with_retry(model.provider_id.as_ref(), "execute_image", {
            let provider = provider.clone();
            let adapter_id = model.adapter_id.clone();
            let model_id = model.model_id.clone();
            move || {
                let provider = provider.clone();
                let adapter_id = adapter_id.clone();
                let model_id = model_id.clone();
                let request = request.clone();
                async move {
                    provider
                        .execute_image_for_adapter(adapter_id.as_ref(), &model_id, request)
                        .await
                }
            }
        })
        .await
    }

    pub async fn complete(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let provider = self.provider_for_model_ref(model)?;
        apply_configured_tool_mode(model, provider.as_ref(), &mut request);
        validate_provider_native_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_native_tool_input_history(
            &request.messages,
        )?;
        let declared_tool_api_functions = declared_tool_api_functions(&request);
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
        let mut repair_count = 0_usize;
        let mut discarded_usage = None;
        loop {
            let response = self
                .call_with_retry(model.provider_id.as_ref(), "complete", {
                    let provider = provider.clone();
                    let request = request.clone();
                    let adapter_id = model.adapter_id.clone();
                    move || {
                        let provider = provider.clone();
                        let request = request.clone();
                        let adapter_id = adapter_id.clone();
                        async move {
                            provider
                                .complete_for_adapter(adapter_id.as_ref(), request)
                                .await
                        }
                    }
                })
                .await?;
            let mut response = response;
            hydrate_usage_cost_from_provider_metadata(
                provider.as_ref(),
                model,
                &mut response.usage,
            );
            match validate_completion_tool_calls(
                model.provider_id.as_ref(),
                &response,
                &declared_tool_api_functions,
            ) {
                Ok(()) => {
                    merge_completion_usage(&mut response.usage, discarded_usage.take());
                    return Ok(response);
                }
                Err(error) if repair_count < MAX_TOOL_API_REPAIRS => {
                    merge_completion_usage(&mut discarded_usage, response.usage.take());
                    let calls = completion_tool_api_calls(&response);
                    append_tool_api_repair_turn(
                        &mut request,
                        &error,
                        calls.as_slice(),
                        &declared_tool_api_functions,
                    );
                    repair_count += 1;
                    tracing::warn!(
                        provider_id = model.provider_id.as_ref(),
                        model_id = model.model_id.as_ref(),
                        repair_count,
                        error = %error,
                        "returning rejected Tool API call to the model for protocol repair"
                    );
                }
                Err(error) => {
                    tracing::error!(
                        provider_id = model.provider_id.as_ref(),
                        model_id = model.model_id.as_ref(),
                        repair_count,
                        error = %error,
                        "provider exhausted internal Tool API repairs"
                    );
                    return Err(tool_api_repair_exhausted(model.provider_id.as_ref()));
                }
            }
        }
    }

    pub async fn compact_conversation(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
        let provider = self.provider_for_model_ref(model)?;
        apply_configured_tool_mode(model, provider.as_ref(), &mut request);
        validate_provider_native_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_native_tool_input_history(
            &request.messages,
        )?;
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
        self.call_with_retry(model.provider_id.as_ref(), "compact_conversation", {
            let provider = provider.clone();
            let request = request.clone();
            let adapter_id = model.adapter_id.clone();
            move || {
                let provider = provider.clone();
                let request = request.clone();
                let adapter_id = adapter_id.clone();
                async move {
                    provider
                        .compact_conversation_for_adapter(adapter_id.as_ref(), request)
                        .await
                }
            }
        })
        .await
    }

    pub async fn complete_stream(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let provider = self.provider_for_model_ref(model)?;
        apply_configured_tool_mode(model, provider.as_ref(), &mut request);
        validate_provider_native_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_native_tool_input_history(
            &request.messages,
        )?;
        let declared_tool_api_functions = declared_tool_api_functions(&request);
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
        let provider_id = model.provider_id.to_string();
        let model_id = model.model_id.to_string();
        let adapter_id = model.adapter_id.clone();
        let model_ref = model.clone();
        let provider_for_usage = provider.clone();
        let retry_policy = self.retry_policy;
        let replay_policy = self.stream_replay_policy;
        let provider_resume_policy = provider.stream_resume_policy();
        let replay_safe_enabled = replay_policy.enabled(provider_resume_policy);

        let stream = async_stream::try_stream! {
            let request_span = tracing::info_span!(
                "provider.request",
                provider_id = provider_id.as_str(),
                model_id = model_id.as_str(),
                operation = "complete_stream",
            );
            request_span.in_scope(|| tracing::debug!("provider stream request started"));
            let mut retry_index = 0_u32;
            let mut replay_retry_index = 0_u32;
            let mut emitted_history: Vec<CompletionStreamEvent> = Vec::new();
            let mut replay_buffer_exhausted = false;
            let mut protocol_repair_count = 0_usize;
            let mut discarded_usage = None;

            loop {
                let attempt = retry_index + 1;
                let attempt_started_at = Instant::now();
                let replay_mode_enabled = replay_safe_enabled && !emitted_history.is_empty();
                tracing::info!(
                    provider_id = provider_id.as_str(),
                    operation = "complete_stream",
                    attempt,
                    retries = retry_index,
                    status = "attempt_started",
                    resume_policy = stream_resume_policy_label(provider_resume_policy),
                    replay_mode = replay_mode_enabled,
                    tracked_events = emitted_history.len() as u64,
                    "provider stream attempt started"
                );

                let mut inner_stream = match provider
                    .complete_stream_for_adapter(adapter_id.as_ref(), request.clone())
                    .await
                {
                    Ok(stream) => {
                        tracing::debug!(
                            provider_id = provider_id.as_str(),
                            operation = "complete_stream",
                            attempt,
                            retries = retry_index,
                            latency_ms = elapsed_ms(attempt_started_at),
                            status = "startup_ok",
                            resume_policy = stream_resume_policy_label(provider_resume_policy),
                            "provider stream startup succeeded"
                        );
                        stream
                    }
                    Err(err) => {
                        let can_retry = err.retryable() && retry_index < retry_policy.max_retries;
                        let reason = retry_reason(&err);
                        if can_retry {
                            let delay = retry_policy.delay_for_retry(retry_index);
                            tracing::warn!(
                                provider_id = provider_id.as_str(),
                                operation = "complete_stream",
                                attempt,
                                retries = retry_index,
                                stage = "startup",
                                max_retries = retry_policy.max_retries,
                                latency_ms = elapsed_ms(attempt_started_at),
                                status = "retry_scheduled",
                                retry_reason = reason,
                                delay_ms = delay.as_millis() as u64,
                                error = %err,
                                "provider stream startup failed with retryable error; scheduling retry"
                            );
                            yield CompletionStreamEvent::ProviderRetry {
                                provider_id: model_ref.provider_id.clone(),
                                model: model_ref.model_id.clone(),
                                message: err.to_string(),
                                retry_index,
                                attempt,
                                max_retries: retry_policy.max_retries,
                                delay_ms: delay.as_millis() as u64,
                            };
                            tokio::time::sleep(delay).await;
                            retry_index += 1;
                            continue;
                        }

                        tracing::error!(
                            provider_id = provider_id.as_str(),
                            operation = "complete_stream",
                            attempt,
                            retries = retry_index,
                            stage = "startup",
                            latency_ms = elapsed_ms(attempt_started_at),
                            status = "failed",
                            retry_reason = reason,
                            error = %err,
                            "provider stream startup failed"
                        );

                        Err(err)?;
                        continue;
                    }
                };

                let mut emitted_event_in_attempt = false;
                let mut should_restart_stream = false;
                let mut replay_cursor = 0_usize;
                let mut replay_mode = replay_mode_enabled;
                let mut emitted_events_in_attempt = 0_u64;
                let mut replayed_events_in_attempt = 0_u64;
                let mut terminal_event_in_attempt = false;
                let mut should_restart_for_protocol_repair = false;
                let mut tool_api_calls = BTreeMap::<String, StreamToolApiCallState>::new();
                // Once a Tool API call begins, retain the rest of that provider
                // turn until Completed validates the whole batch. This both
                // prevents invalid calls from reaching session state and
                // preserves the provider's original event ordering.
                let mut buffered_tool_api_turn = Vec::<CompletionStreamEvent>::new();
                let mut tool_api_error: Option<ProviderError> = None;

                while let Some(item) = inner_stream.next().await {
                    match item {
                        Ok(mut event) => {
                            if let CompletionStreamEvent::Completed { usage, .. } = &mut event {
                                hydrate_usage_cost_from_provider_metadata(
                                    provider_for_usage.as_ref(),
                                    &model_ref,
                                    usage,
                                );
                            }
                            if replay_mode && replay_cursor < emitted_history.len() {
                                if event == emitted_history[replay_cursor] {
                                    if matches!(event, CompletionStreamEvent::Completed { .. }) {
                                        terminal_event_in_attempt = true;
                                    }
                                    replay_cursor += 1;
                                    replayed_events_in_attempt += 1;
                                    if replay_cursor == emitted_history.len() {
                                        replay_mode = false;
                                        tracing::debug!(
                                            provider_id = provider_id.as_str(),
                                            operation = "complete_stream",
                                            attempt,
                                            status = "replay_prefix_aligned",
                                            replayed_events = replayed_events_in_attempt,
                                            "provider stream replay prefix aligned"
                                        );
                                    }
                                    continue;
                                }

                                let err = ProviderError::Provider(format!(
                                    "provider stream replay prefix diverged at event index {replay_cursor}"
                                ));
                                tracing::error!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "replay_prefix",
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "failed",
                                    retry_reason = "replay_prefix_diverged",
                                    replayed_events = replayed_events_in_attempt,
                                    "provider stream replay prefix diverged; aborting to avoid duplicate output"
                                );
                                Err(err)?;
                            }

                            replay_mode = false;
                            if terminal_event_in_attempt {
                                Err(ProviderError::Provider(format!(
                                    "provider `{provider_id}` emitted a stream event after Completed"
                                )))?;
                            }
                            if matches!(event, CompletionStreamEvent::Completed { .. }) {
                                terminal_event_in_attempt = true;
                            }

                            let tool_api_event = matches!(
                                event,
                                CompletionStreamEvent::ToolCallDelta { .. }
                                    | CompletionStreamEvent::ToolCallSnapshot { .. }
                            );
                            if tool_api_event {
                                if let Err(error) = validate_stream_tool_api_event(
                                    provider_id.as_str(),
                                    &event,
                                    &declared_tool_api_functions,
                                    &mut tool_api_calls,
                                ) {
                                    tool_api_error.get_or_insert(error);
                                }
                                buffered_tool_api_turn.push(event);
                                continue;
                            }

                            if !buffered_tool_api_turn.is_empty()
                                && !matches!(event, CompletionStreamEvent::Completed { .. })
                            {
                                buffered_tool_api_turn.push(event);
                                continue;
                            }

                            if matches!(event, CompletionStreamEvent::Completed { .. }) {
                                if let Err(error) = validate_stream_tool_api_event(
                                    provider_id.as_str(),
                                    &event,
                                    &declared_tool_api_functions,
                                    &mut tool_api_calls,
                                ) {
                                    tool_api_error.get_or_insert(error);
                                }
                                if let Some(error) = tool_api_error.take() {
                                    if let CompletionStreamEvent::Completed { usage, .. } = &mut event {
                                        merge_completion_usage(&mut discarded_usage, usage.take());
                                    }
                                    if protocol_repair_count < MAX_TOOL_API_REPAIRS {
                                        let calls = stream_tool_api_calls(&tool_api_calls);
                                        append_tool_api_repair_turn(
                                            &mut request,
                                            &error,
                                            calls.as_slice(),
                                            &declared_tool_api_functions,
                                        );
                                        protocol_repair_count += 1;
                                        tracing::warn!(
                                            provider_id = provider_id.as_str(),
                                            model_id = model_id.as_str(),
                                            protocol_repair_count,
                                            error = %error,
                                            "returning rejected Tool API stream call to the model for protocol repair"
                                        );
                                        retry_index = 0;
                                        replay_retry_index = 0;
                                        emitted_history.clear();
                                        replay_buffer_exhausted = false;
                                        should_restart_for_protocol_repair = true;
                                        break;
                                    }
                                    tracing::error!(
                                        provider_id = provider_id.as_str(),
                                        model_id = model_id.as_str(),
                                        protocol_repair_count,
                                        error = %error,
                                        "provider exhausted internal Tool API stream repairs"
                                    );
                                    Err(tool_api_repair_exhausted(
                                        provider_id.as_str(),
                                    ))?;
                                }
                                if let CompletionStreamEvent::Completed { usage, .. } = &mut event {
                                    merge_completion_usage(usage, discarded_usage.take());
                                }

                                for buffered_event in buffered_tool_api_turn.drain(..) {
                                    emitted_events_in_attempt += 1;
                                    if replay_safe_enabled && !replay_buffer_exhausted {
                                        if emitted_history.len() < replay_policy.max_tracked_events {
                                            emitted_history.push(buffered_event.clone());
                                        } else {
                                            replay_buffer_exhausted = true;
                                        }
                                    }
                                    yield buffered_event;
                                }
                            }

                            emitted_event_in_attempt = true;
                            emitted_events_in_attempt += 1;

                            if replay_safe_enabled && !replay_buffer_exhausted {
                                if emitted_history.len() < replay_policy.max_tracked_events {
                                    emitted_history.push(event.clone());
                                } else {
                                    replay_buffer_exhausted = true;
                                    tracing::warn!(
                                        provider_id = provider_id.as_str(),
                                        operation = "complete_stream",
                                        attempt,
                                        status = "replay_buffer_exhausted",
                                        tracked_events = emitted_history.len() as u64,
                                        max_tracked_events = replay_policy.max_tracked_events as u64,
                                        "provider stream replay buffer exhausted; disabling post-output replay-safe restart"
                                    );
                                }
                            }

                            yield event;
                        }
                        Err(err) => {
                            let can_retry_now = err.retryable() && retry_index < retry_policy.max_retries;
                            let can_retry_early_stream_error = !emitted_event_in_attempt
                                && can_retry_now;

                            let can_retry_after_output = emitted_event_in_attempt
                                && can_retry_now
                                && replay_safe_enabled
                                && !replay_buffer_exhausted
                                && replay_retry_index < replay_policy.max_retries_after_output;

                            let reason = retry_reason(&err);

                            if can_retry_early_stream_error {
                                let delay = retry_policy.delay_for_retry(retry_index);
                                tracing::warn!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "before_first_event",
                                    max_retries = retry_policy.max_retries,
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "retry_scheduled",
                                    retry_reason = reason,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %err,
                                    "provider stream failed before first event with retryable error; restarting stream"
                                );
                                yield CompletionStreamEvent::ProviderRetry {
                                    provider_id: model_ref.provider_id.clone(),
                                    model: model_ref.model_id.clone(),
                                    message: err.to_string(),
                                    retry_index,
                                    attempt,
                                    max_retries: retry_policy.max_retries,
                                    delay_ms: delay.as_millis() as u64,
                                };
                                tokio::time::sleep(delay).await;
                                retry_index += 1;
                                should_restart_stream = true;
                                break;
                            }

                            if can_retry_after_output {
                                let delay = retry_policy.delay_for_retry(retry_index);
                                tracing::warn!(
                                    provider_id = provider_id.as_str(),
                                    operation = "complete_stream",
                                    attempt,
                                    retries = retry_index,
                                    stage = "after_output",
                                    max_retries = retry_policy.max_retries,
                                    replay_restarts = replay_retry_index,
                                    max_replay_restarts = replay_policy.max_retries_after_output,
                                    latency_ms = elapsed_ms(attempt_started_at),
                                    status = "replay_restart_scheduled",
                                    retry_reason = reason,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %err,
                                    "provider stream failed after output with replay-safe provider; scheduling replay-aware restart"
                                );
                                yield CompletionStreamEvent::ProviderRetry {
                                    provider_id: model_ref.provider_id.clone(),
                                    model: model_ref.model_id.clone(),
                                    message: err.to_string(),
                                    retry_index,
                                    attempt,
                                    max_retries: replay_policy.max_retries_after_output,
                                    delay_ms: delay.as_millis() as u64,
                                };
                                tokio::time::sleep(delay).await;
                                retry_index += 1;
                                replay_retry_index += 1;
                                should_restart_stream = true;
                                break;
                            }

                            tracing::error!(
                                provider_id = provider_id.as_str(),
                                operation = "complete_stream",
                                attempt,
                                retries = retry_index,
                                stage = if emitted_event_in_attempt { "after_output" } else { "before_first_event" },
                                latency_ms = elapsed_ms(attempt_started_at),
                                status = "failed",
                                retry_reason = reason,
                                replay_restarts = replay_retry_index,
                                error = %err,
                                "provider stream failed"
                            );

                            Err(err)?;
                        }
                    }
                }

                if should_restart_for_protocol_repair {
                    continue;
                }

                if replay_mode && replay_cursor < emitted_history.len() {
                    let err = ProviderError::Provider(
                        "provider stream replay ended before replay prefix alignment completed"
                            .to_owned(),
                    );
                    tracing::error!(
                        provider_id = provider_id.as_str(),
                        operation = "complete_stream",
                        attempt,
                        retries = retry_index,
                        stage = "replay_prefix",
                        latency_ms = elapsed_ms(attempt_started_at),
                        status = "failed",
                        retry_reason = "replay_prefix_incomplete",
                        replayed_events = replayed_events_in_attempt,
                        expected_events = emitted_history.len() as u64,
                        "provider stream replay ended before matching emitted prefix"
                    );
                    Err(err)?;
                }

                if should_restart_stream {
                    continue;
                }

                if !terminal_event_in_attempt {
                    Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` stream ended without a Completed event"
                    )))?;
                }

                tracing::info!(
                    provider_id = provider_id.as_str(),
                    operation = "complete_stream",
                    attempt,
                    retries = retry_index,
                    replay_restarts = replay_retry_index,
                    latency_ms = elapsed_ms(attempt_started_at),
                    status = "completed",
                    emitted_events = emitted_events_in_attempt,
                    replayed_events = replayed_events_in_attempt,
                    "provider stream attempt completed"
                );

                break;
            }
        };

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tool_api_function_validation_tests {
    use std::{
        collections::BTreeSet,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::{
        RejectedToolApiCall, StreamToolApiCallState, parse_tool_api_arguments_tolerant,
        stream_tool_api_calls, validate_provider_native_tool_definition_boundary,
        validate_returned_tool_api_function, validate_stream_tool_api_event,
        validate_tool_api_arguments,
    };
    use crate::provider::{CompletionResponse, ModelRuntime};
    use agena_domain::Role;
    use agena_domain::StructuredObject;
    use agena_domain::TimeRange;
    use agena_domain::ToolApiFunction;
    use agena_domain::ToolInvocation;
    use agena_domain::ToolOutput;
    use agena_domain::{Model, ModelId, ModelRef, ProviderId};
    use agena_plugin_host::registry::RegisteredTool;
    use agena_plugin_host::sdk::{PluginKey, ToolDefinition};
    use agena_provider::CompletionStreamEvent;
    use agena_provider::{
        CompletionFinishReason, CompletionRequest, CompletionToolCall, CompletionUsage,
    };
    use agena_runtime_contracts::message::{Message, OperationPart, PartContent};
    use agena_runtime_tools::tool::ToolApiBinding;
    use async_trait::async_trait;
    use futures_util::{StreamExt, stream};

    struct ProtocolRepairProvider {
        calls: AtomicUsize,
        requests: Mutex<Vec<CompletionRequest>>,
        model: ModelId,
        repair_after: usize,
        rejected: RejectedToolApiCall,
        repaired: RejectedToolApiCall,
    }

    impl ProtocolRepairProvider {
        fn new() -> Self {
            Self::repair_after(1)
        }

        fn repair_after(repair_after: usize) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                model: ModelId::new("test-model"),
                repair_after,
                rejected: RejectedToolApiCall {
                    name: "fs.read".to_owned(),
                    arguments_json: r#"{"file_path":"README.md"}"#.to_owned(),
                },
                repaired: RejectedToolApiCall {
                    name: "tools_call".to_owned(),
                    arguments_json: r#"{"tool":"fs.read","input":{"file_path":"README.md"}}"#
                        .to_owned(),
                },
            }
        }

        fn wrapped_tool_api_function() -> Self {
            Self {
                rejected: RejectedToolApiCall {
                    name: "tools_call".to_owned(),
                    arguments_json: r#"{"tool":"tools_list","input":{"limit":10}}"#.to_owned(),
                },
                repaired: RejectedToolApiCall {
                    name: "tools_list".to_owned(),
                    arguments_json: r#"{"limit":10}"#.to_owned(),
                },
                ..Self::repair_after(1)
            }
        }

        fn response(&self, request: CompletionRequest) -> CompletionResponse {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().expect("requests lock").push(request);
            let returned = if attempt < self.repair_after {
                &self.rejected
            } else {
                &self.repaired
            };
            CompletionResponse {
                provider_id: ProviderId::new("repair-test"),
                model: self.model.clone(),
                text: String::new(),
                reasoning_text: Some(format!("attempt {attempt}")),
                finish_reason: Some(CompletionFinishReason::ToolCalls),
                tool_calls: vec![CompletionToolCall::Function {
                    id: format!("call_{attempt}"),
                    name: returned.name.clone(),
                    arguments_json: returned.arguments_json.clone(),
                }],
                usage: Some(CompletionUsage {
                    requests: 1,
                    input_tokens: 1,
                    output_tokens: 1,
                    reasoning_tokens: 1,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    total_cost: 0.0,
                    ..CompletionUsage::default()
                }),
                provider_metadata: None,
            }
        }
    }

    #[async_trait]
    impl ModelRuntime for ProtocolRepairProvider {
        fn id(&self) -> &str {
            "repair-test"
        }

        fn default_model(&self) -> &ModelId {
            &self.model
        }

        fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn list_models(&self) -> Result<Vec<Model>, crate::ProviderError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::ProviderError> {
            Ok(self.response(request))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_core::Stream<
                            Item = Result<CompletionStreamEvent, crate::ProviderError>,
                        > + Send,
                >,
            >,
            crate::ProviderError,
        > {
            let response = self.response(request);
            let post_call_text = format!(
                "post-call {}",
                response.reasoning_text.as_deref().unwrap_or_default()
            );
            let CompletionToolCall::Function {
                id,
                name,
                arguments_json,
            } = response.tool_calls[0].clone();
            let events = vec![
                Ok(CompletionStreamEvent::ThinkingDelta {
                    provider_id: response.provider_id.clone(),
                    model: response.model.clone(),
                    delta: response.reasoning_text.unwrap_or_default(),
                }),
                Ok(CompletionStreamEvent::ToolCallSnapshot {
                    provider_id: response.provider_id.clone(),
                    model: response.model.clone(),
                    stream_key: format!("id:{id}"),
                    id: Some(id),
                    name: Some(name),
                    arguments_json,
                }),
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: response.provider_id.clone(),
                    model: response.model.clone(),
                    delta: post_call_text,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: response.provider_id,
                    model: response.model,
                    finish_reason: response.finish_reason,
                    usage: response.usage,
                    provider_metadata: None,
                    end_turn: None,
                }),
            ];
            Ok(Box::pin(stream::iter(events)))
        }
    }

    fn all_tool_api_definitions() -> Vec<agena_provider::ToolApiDefinition> {
        ["list", "search", "help", "tags", "call"]
            .into_iter()
            .map(|name| {
                let mut definition: ToolDefinition =
                    serde_json::from_value(serde_json::json!({ "name": name }))
                        .expect("tool definition");
                definition.contract.input_schema = serde_json::json!({
                    "type": "object",
                    "properties": {},
                });
                let handler = RegisteredTool::new(
                    PluginKey::new("agena", "tools").expect("plugin key"),
                    definition,
                )
                .expect("registered Tool API handler");
                ToolApiBinding::from_registered_tool(handler)
                    .expect("Tool API binding")
                    .definition()
            })
            .collect()
    }

    fn completed_help_message() -> Message {
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Help.function_name(),
            StructuredObject::try_from(serde_json::json!({ "tool": "fs.read" }))
                .expect("structured help input"),
        );
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: ToolApiFunction::Help,
            arguments: invocation.input.clone(),
        });
        let mut message = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::operation(OperationPart::completed(
                0,
                invocation,
                agena_runtime_contracts::message::OperationCompletion::new(
                    "Tool help",
                    "Help returned",
                    "help output".to_owned(),
                    Vec::new(),
                    Vec::new(),
                    ToolOutput::default(),
                ),
                TimeRange::default(),
            ))],
        );
        message.parts[0].operation_id = Some("call_help".to_owned());
        message
    }

    fn repair_request() -> CompletionRequest {
        CompletionRequest {
            model: ModelId::new("test-model"),
            system: Some("base system".to_owned()),
            messages: vec![
                crate::provider::project_completion_input(&Message::prompt_text(
                    Role::User,
                    "read README.md",
                )),
                crate::provider::project_completion_input(&completed_help_message()),
            ],
            tool_api_functions: all_tool_api_definitions(),
            provider_native_tools: Default::default(),
            disable_tools: false,
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: Some("previous".to_owned()),
            prompt_window_generation: None,
            provider_compaction: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: None,
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: Default::default(),
        }
    }

    fn request_with_tool_api_schema(schema: serde_json::Value) -> CompletionRequest {
        let mut definition: ToolDefinition = serde_json::from_value(serde_json::json!({
            "name": "help"
        }))
        .expect("tool definition");
        definition.contract.input_schema = schema;
        let handler = RegisteredTool::new(
            PluginKey::new("agena", "tools").expect("plugin key"),
            definition,
        )
        .expect("registered Tool API handler");
        let binding = ToolApiBinding::from_registered_tool(handler).expect("Tool API binding");
        let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "test-model",
            "messages": []
        }))
        .expect("minimal request");
        request.tool_api_functions.push(binding.definition());
        request
    }

    #[test]
    fn returned_tool_api_function_must_exactly_match_a_declaration() {
        let declared = BTreeSet::from(["tools_help".to_owned()]);
        validate_returned_tool_api_function("test", "tools_help", &declared)
            .expect("exact declaration");

        for invalid in [
            "agena.tools.help",
            "tools.help",
            " tools_help",
            "tools_help ",
        ] {
            let error = validate_returned_tool_api_function("test", invalid, &declared)
                .expect_err("undeclared provider name must fail");
            assert!(error.to_string().contains("unknown Tool API function"));
        }
    }

    #[test]
    fn body_patches_cannot_inject_provider_function_definitions() {
        for field in ["tools", "functions"] {
            let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
                "model": "test-model",
                "messages": []
            }))
            .expect("minimal request");
            request
                .request_override
                .body_patch
                .insert(field.to_owned(), serde_json::json!([]));

            let error = validate_provider_native_tool_definition_boundary(&request)
                .expect_err("reserved tool field must fail");
            assert!(error.to_string().contains(field));
        }
    }

    #[test]
    fn only_tool_api_definitions_are_subject_to_provider_schema_rules() {
        let valid = request_with_tool_api_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }));
        validate_provider_native_tool_definition_boundary(&valid).expect("object Tool API schema");

        let invalid = request_with_tool_api_schema(serde_json::json!({ "type": "array" }));
        let error = validate_provider_native_tool_definition_boundary(&invalid)
            .expect_err("provider-bound schema must be object-shaped");
        assert!(
            error
                .to_string()
                .contains("provider-bound Tool API function")
        );

        let mut duplicate = valid;
        duplicate
            .tool_api_functions
            .push(duplicate.tool_api_functions[0].clone());
        let error = validate_provider_native_tool_definition_boundary(&duplicate)
            .expect_err("duplicate provider declarations must fail");
        assert!(error.to_string().contains("declared more than once"));
    }

    #[test]
    fn provider_declarations_require_gateway_functions() {
        for invalid_name in ["agena.tools.help", "tools.help", "fs.read"] {
            let mut request = request_with_tool_api_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }));
            request.tool_api_functions[0].name = invalid_name.to_owned();
            let error = validate_provider_native_tool_definition_boundary(&request)
                .expect_err("non-gateway tools must fail");
            assert!(error.to_string().contains("not one of the five Tool API"));
        }
    }

    #[test]
    fn returned_tool_api_arguments_must_be_one_complete_json_object() {
        validate_tool_api_arguments("test", "tools_help", r#"{"tool":"session.get"}"#)
            .expect("valid object arguments");
        for invalid in ["", "null", "[]", r#"{} trailing"#] {
            let error = validate_tool_api_arguments("test", "tools_help", invalid)
                .expect_err("invalid arguments must fail");
            assert!(error.to_string().contains("arguments"));
        }
    }

    #[test]
    fn tool_api_functions_cannot_be_wrapped_as_execution_tools() {
        let error = validate_tool_api_arguments(
            "test",
            "tools_call",
            r#"{"tool":"tools_list","input":{}}"#,
        )
        .expect_err("Tool API functions must be called directly");
        assert!(error.to_string().contains("call `tools_list` directly"));

        let calls = vec![super::RejectedToolApiCall {
            name: "tools_call".to_owned(),
            arguments_json: r#"{"tool":"tools_list","input":{"limit":10}}"#.to_owned(),
        }];
        let declared = BTreeSet::from(["tools_call".to_owned(), "tools_list".to_owned()]);
        let guidance = super::tool_name_repair_guidance(&calls, &declared);
        assert_eq!(guidance.len(), 1);
        assert!(guidance[0].contains("Call function `tools_list` directly"));
        assert!(guidance[0].contains(r#"{"limit":10}"#));
    }

    #[test]
    fn tools_call_requires_an_exact_tool_name_and_complete_input() {
        for invalid in [
            r#"{"tool":"fs_read","input":{}}"#,
            r#"{"tool":"fs.read"}"#,
            r#"{"tool":"fs.read","input":null}"#,
        ] {
            validate_tool_api_arguments("test", "tools_call", invalid)
                .expect_err("invalid tools_call semantics must fail before execution");
        }
        validate_tool_api_arguments(
            "test",
            "tools_call",
            r#"{"tool":"fs.read","input":{"file_path":"README.md"}}"#,
        )
        .expect("complete execution-tool call");
    }

    #[test]
    fn rejected_stream_call_keeps_fragmented_arguments_for_model_repair() {
        let declared = BTreeSet::from(["tools_call".to_owned(), "tools_help".to_owned()]);
        let mut calls = std::collections::BTreeMap::<String, StreamToolApiCallState>::new();
        let provider_id = ProviderId::new("repair-test");
        let model = ModelId::new("test-model");
        let first = CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key: "id:bad".to_owned(),
            id: Some("bad".to_owned()),
            name: Some("fs.read".to_owned()),
            arguments_delta: r#"{"file_"#.to_owned(),
        };
        validate_stream_tool_api_event("repair-test", &first, &declared, &mut calls)
            .expect("validation waits for the complete streamed arguments");
        let second = CompletionStreamEvent::ToolCallDelta {
            provider_id,
            model,
            stream_key: "id:bad".to_owned(),
            id: None,
            name: None,
            arguments_delta: r#"path":"README.md"}"#.to_owned(),
        };
        validate_stream_tool_api_event("repair-test", &second, &declared, &mut calls)
            .expect("argument-only continuation is retained");
        let completed = CompletionStreamEvent::Completed {
            provider_id: ProviderId::new("repair-test"),
            model: ModelId::new("test-model"),
            finish_reason: Some(CompletionFinishReason::ToolCalls),
            usage: None,
            provider_metadata: None,
            end_turn: None,
        };
        validate_stream_tool_api_event("repair-test", &completed, &declared, &mut calls)
            .expect_err("execution-tool name is rejected only after its full input is retained");

        assert_eq!(
            stream_tool_api_calls(&calls),
            vec![super::RejectedToolApiCall {
                name: "fs.read".to_owned(),
                arguments_json: r#"{"file_path":"README.md"}"#.to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn completion_returns_misrouted_execution_tool_call_to_model_for_repair() {
        let provider = std::sync::Arc::new(ProtocolRepairProvider::new());
        let mut registry = crate::provider::ProviderRegistry::new();
        registry.register_arc(provider.clone());

        let response = registry
            .complete(
                &ModelRef::new("repair-test", "test-model"),
                repair_request(),
            )
            .await
            .expect("protocol violation should be repaired internally");

        let CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } = &response.tool_calls[0];
        assert_eq!(name, "tools_call");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(arguments_json).expect("valid arguments"),
            serde_json::json!({
                "tool": "fs.read",
                "input": { "file_path": "README.md" },
            })
        );
        assert_eq!(response.usage.expect("aggregated usage").input_tokens, 2);
        let requests = provider.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].system.as_deref(), Some("base system"));
        let repair = requests[1]
            .messages
            .last()
            .expect("repair message")
            .as_text_lossy();
        assert!(repair.contains("The only allowed Tool API function names are"));
        assert!(!repair.contains("Retry with function `tools_call`"));
    }

    #[tokio::test]
    async fn completion_unwraps_a_tool_api_function_misrouted_through_tools_call() {
        let provider = std::sync::Arc::new(ProtocolRepairProvider::wrapped_tool_api_function());
        let mut registry = crate::provider::ProviderRegistry::new();
        registry.register_arc(provider.clone());

        let response = registry
            .complete(
                &ModelRef::new("repair-test", "test-model"),
                repair_request(),
            )
            .await
            .expect("wrapped Tool API function should be repaired internally");

        let CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } = &response.tool_calls[0];
        assert_eq!(name, "tools_list");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(arguments_json).expect("valid arguments"),
            serde_json::json!({ "limit": 10 })
        );
        let requests = provider.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        let repair_text = requests[1]
            .messages
            .last()
            .expect("repair message")
            .as_text_lossy();
        assert!(repair_text.contains("Call function `tools_list` directly"));
        assert!(repair_text.contains(r#"{"limit":10}"#));
    }

    #[tokio::test]
    async fn exhausted_repairs_return_a_sanitized_terminal_error() {
        let provider = std::sync::Arc::new(ProtocolRepairProvider::repair_after(usize::MAX));
        let mut registry = crate::provider::ProviderRegistry::new();
        registry.register_arc(provider.clone());

        let error = registry
            .complete(
                &ModelRef::new("repair-test", "test-model"),
                repair_request(),
            )
            .await
            .expect_err("a persistently invalid provider must fail closed");
        let message = error.to_string();
        assert!(message.contains("after 2 internal repair attempts"));
        assert!(!message.contains("unknown Tool API function"));
        assert!(!message.contains("fs.read"));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn stream_suppresses_unknown_call_and_emits_repaired_tool_api_call() {
        let provider = std::sync::Arc::new(ProtocolRepairProvider::new());
        let mut registry = crate::provider::ProviderRegistry::new();
        registry.register_arc(provider.clone());

        let events = registry
            .complete_stream(
                &ModelRef::new("repair-test", "test-model"),
                repair_request(),
            )
            .await
            .expect("stream startup")
            .collect::<Vec<_>>()
            .await;
        let events = events
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("protocol repair must not leak a stream error");

        let names = events
            .iter()
            .filter_map(|event| match event {
                CompletionStreamEvent::ToolCallSnapshot { name, .. }
                | CompletionStreamEvent::ToolCallDelta { name, .. } => name.as_deref(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["tools_call"]);
        assert!(!names.contains(&"fs.read"));
        let text_deltas = events
            .iter()
            .filter_map(|event| match event {
                CompletionStreamEvent::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text_deltas, vec!["post-call attempt 1"]);
        let tool_position = events
            .iter()
            .position(|event| matches!(event, CompletionStreamEvent::ToolCallSnapshot { .. }))
            .expect("repaired tool event");
        let post_call_position = events
            .iter()
            .position(|event| matches!(event, CompletionStreamEvent::TextDelta { .. }))
            .expect("repaired post-call text");
        assert!(tool_position < post_call_position);
        let usage = events.iter().find_map(|event| match event {
            CompletionStreamEvent::Completed { usage, .. } => usage.as_ref(),
            _ => None,
        });
        assert_eq!(usage.expect("completed usage").input_tokens, 2);

        let requests = provider.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        let repair = requests[1].messages.last().expect("repair message");
        let repair_text = repair.as_text_lossy();
        assert!(repair_text.contains("rejected before execution"));
        assert!(repair_text.contains("The only allowed Tool API function names are"));
        assert!(!repair_text.contains("optional reusable schema discovery"));
        assert!(!repair_text.contains("Retry with function `tools_call`"));
        assert!(repair_text.contains(r#""name":"fs.read""#));
        assert!(requests[1].previous_response_id.is_none());
        assert_eq!(requests[1].temperature, Some(0.0));
    }
    #[test]
    fn tolerant_parse_accepts_valid_arguments() {
        let json = r#"{"tool":"fs.read"}"#;
        let parsed = parse_tool_api_arguments_tolerant(json).expect("valid arguments parse");
        assert_eq!(parsed["tool"], "fs.read");
    }

    #[test]
    fn tolerant_parse_repairs_invalid_escape_arguments() {
        let json = r#"{"tool":"fs.read","input":{"path":"C:\temp\new"}}"#;
        let parsed = parse_tool_api_arguments_tolerant(json).expect("invalid escape repaired");
        let input = parsed["input"].as_object().expect("input object");
        assert_eq!(input["path"].as_str(), Some("C:\temp\new"));
    }

    #[test]
    fn tolerant_parse_rejects_truly_malformed_arguments() {
        let json = "{\"tool\":";
        assert!(parse_tool_api_arguments_tolerant(json).is_none());
    }
}
