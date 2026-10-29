use futures_util::StreamExt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agena_provider::{CompletionToolCall, CompletionUsage, ProviderCompactionOutput};
use agena_tool_api::normalize_tool_call_input;

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, Instant, ModelRef, ModelRuntime,
    ProviderError, ProviderRegistry, Stream, breaker_open, breaker_record_failure,
    breaker_record_success, elapsed_ms, hydrate_usage_cost_from_provider_metadata, retry_reason,
    stream_resume_policy_label, validate_request_capabilities,
};

/// Safety ceiling for protocol repair turns. This is deliberately large and is
/// not a small "give up" budget: a provider that emits malformed Tool API
/// calls must never interrupt the session. Reaching the ceiling degrades
/// gracefully - the rejected calls are dropped and the run finishes with the
/// remaining response content - instead of aborting the run.
const MAX_TOOL_API_REPAIRS: usize = 64;
const MAX_REJECTED_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_REJECTED_CALLS_IN_REPAIR: usize = 16;

/// Outcome of aligning one text-like delta against the already-emitted
/// prefix during a replay-safe stream restart.
enum ReplayTextAlignment {
    /// The delta is entirely inside the already-emitted prefix; drop it.
    Consumed,
    /// The delta exactly completes the already-emitted prefix; drop it and
    /// mark this content stream as live.
    Aligned,
    /// The delta extends past the already-emitted prefix; emit only `excess`
    /// and mark this content stream as live.
    Live { excess: String },
    /// The concatenated text differs from the already-emitted prefix; the
    /// retried stream cannot be spliced into already-emitted output.
    Diverged { replayed_chars: usize },
}

/// Align one text or reasoning delta against the concatenated emitted prefix.
///
/// Providers (including "deterministic" official endpoints) may re-chunk text
/// deltas across independent retry requests, so event-by-event `PartialEq`
/// comparison is too strict and produced spurious "replay prefix diverged"
/// aborts at chunk boundaries. Comparing the concatenated text keeps the
/// replay safe (no duplicate output) while tolerating different chunk
/// boundaries.
fn align_replay_text(
    accumulator: &mut String,
    expected: &str,
    live: &mut bool,
    delta: &str,
) -> ReplayTextAlignment {
    if *live {
        return ReplayTextAlignment::Live {
            excess: delta.to_owned(),
        };
    }
    accumulator.push_str(delta);
    if accumulator.len() > expected.len() {
        if !accumulator.starts_with(expected) {
            return ReplayTextAlignment::Diverged {
                replayed_chars: expected.len(),
            };
        }
        let excess = accumulator[expected.len()..].to_owned();
        accumulator.clear();
        *live = true;
        if excess.is_empty() {
            return ReplayTextAlignment::Aligned;
        }
        return ReplayTextAlignment::Live { excess };
    }
    if expected.starts_with(accumulator.as_str()) {
        if accumulator.len() == expected.len() {
            *live = true;
            return ReplayTextAlignment::Aligned;
        }
        return ReplayTextAlignment::Consumed;
    }
    ReplayTextAlignment::Diverged {
        replayed_chars: accumulator.len(),
    }
}

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

/// Whether a stream event carries user-visible or actionable content.
/// `Completed` and provider retry notifications are bookkeeping; everything else is
/// content (text, reasoning, or tool calls). An attempt whose only yielded
/// events are bookkeeping is an empty response.
fn completion_stream_event_is_content(event: &CompletionStreamEvent) -> bool {
    !matches!(
        event,
        CompletionStreamEvent::Completed { .. }
            | CompletionStreamEvent::ProviderRetry { .. }
            | CompletionStreamEvent::ProviderRetryStarted { .. }
    )
}

fn stream_tool_api_calls(calls: &StreamToolApiCalls) -> Vec<RejectedToolApiCall> {
    calls
        .calls
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
    let mut rendered = match serde_json::to_string(&calls) {
        Ok(rendered) => rendered,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize rejected Tool API calls for repair guidance",
                    &error,
                ),
                "Tool API repair guidance could not include rejected call details"
            );
            "[rejected call details could not be serialized]".to_owned()
        }
    };
    if omitted {
        rendered.push_str(" [additional rejected calls omitted]");
    }
    rendered
}

fn repair_arguments_json(arguments: &serde_json::Value, context: &'static str) -> String {
    match serde_json::to_string(arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "Tool API repair guidance could not serialize direct arguments"
            );
            "[arguments could not be serialized]".to_owned()
        }
    }
}

fn tool_name_repair_guidance(
    calls: &[RejectedToolApiCall],
    declared: &BTreeSet<String>,
) -> Vec<String> {
    calls
        .iter()
        .filter_map(|call| {
            if call.name == "tools_call" {
                let arguments = match serde_json::from_str::<serde_json::Value>(
                    call.arguments_json.as_str(),
                ) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        tracing::warn!(
                            rejected_tool = %call.name,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "decode rejected tools_call arguments for repair guidance",
                                &error,
                            ),
                            "Tool API repair guidance omitted malformed call details"
                        );
                        return None;
                    }
                };
                let tool_name = arguments.get("tool")?.as_str()?;
                let api_function =
                    agena_domain::ToolApiFunction::from_function_name(tool_name)?;
                let function_name = api_function.function_name();
                if !declared.contains(function_name) {
                    return None;
                }
                let direct_arguments = arguments
                    .get("input")
                    .cloned()
                    .and_then(|input| normalize_tool_call_input(tool_name, input).ok())
                    .map(serde_json::Value::Object)
                    .unwrap_or_else(|| serde_json::json!({}));
                return Some(format!(
                    "- `{tool_name}` identifies Tool API function `{function_name}`, not an execution tool. Call function `{function_name}` directly with arguments {}; never put a Tool API function name inside `tools_call.arguments.tool`.",
                    repair_arguments_json(
                        &direct_arguments,
                        "serialize direct Tool API arguments from the tools_call repair path",
                    )
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
                    repair_arguments_json(
                        &direct_arguments,
                        "serialize direct Tool API arguments from an internal handler repair path",
                    )
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
    request.turns.push(agena_provider::CompletionInputRun {
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

/// Append a short nudge for an empty-response retry (mirrors gemini's
/// on-retry nudging: "you only produced thinking, please answer now"). The
/// previous attempt completed with no text, reasoning, or tool call, so the
/// next attempt must be told the task is still open instead of silently
/// resampling the same empty completion.
fn append_empty_response_nudge(request: &mut CompletionRequest) {
    request.turns.push(agena_provider::CompletionInputRun {
        role: agena_domain::Role::User,
        parts: vec![agena_provider::CompletionInputPart::Text {
            text: "Trusted Agena runtime note: the previous provider attempt returned an empty response with no text, reasoning, or tool call. The user's task is still unresolved. Provide your final answer now, or emit a valid Tool API call to make progress."
                .to_owned(),
        }],
        provider_state: Default::default(),
    });
    request.previous_response_id = None;
    request.temperature = Some(0.0);
}

fn merge_completion_usage(
    target: &mut Option<CompletionUsage>,
    additional: Option<CompletionUsage>,
) {
    let Some(mut additional) = additional else {
        return;
    };
    let Some(target) = target.as_mut() else {
        // No usage was reported for the final attempt. Retain the billable
        // discarded attempts, but do not invent a prompt measurement for it.
        additional.request_usage = Some(Default::default());
        *target = Some(additional);
        return;
    };
    let request_usage = target.request_usage_snapshot();
    target.add_assign(&additional);
    target.request_usage = Some(request_usage);
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
    agena_provider::apply_configured_tool_request(mode, request);
}

/// Backfill the assistant reasoning wire field onto the request's assistant
/// turns, resolved from the provider's configured metadata for this route.
///
/// This runs unconditionally at the registry boundary - parallel to
/// `apply_configured_tool_mode` - so it applies to every provider, including
/// bare providers that are not wrapped in a catalog. It must run after
/// `request.model` is resolved to the route's model id, because the backfill
/// reads `model_metadata_for_adapter` for that model.
fn apply_request_backfill(
    model: &ModelRef,
    provider: &dyn ModelRuntime,
    request: &mut CompletionRequest,
) {
    provider.backfill_assistant_reasoning_field(model.adapter_id.as_ref(), request);
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
        if let Err(reason) = validate_provider_bound_tool_name(tool.name.as_str()) {
            return Err(ProviderError::Config(reason));
        }
        if !has_valid_provider_function_name(tool.name.as_str()) {
            return Err(ProviderError::Config(format!(
                "provider-bound tool name {:?} contains unsupported characters",
                tool.name
            )));
        }
        if !declared.insert(tool.name.clone()) {
            return Err(ProviderError::Config(format!(
                "provider-bound function `{}` is declared more than once",
                tool.name
            )));
        }
        let schema = tool.input_schema.as_object().ok_or_else(|| {
            ProviderError::Config(format!(
                "provider-bound function `{}` must use an object input schema",
                tool.name
            ))
        })?;
        if schema.get("type").and_then(serde_json::Value::as_str) != Some("object") {
            return Err(ProviderError::Config(format!(
                "provider-bound function `{}` must use an object input schema",
                tool.name
            )));
        }
        if schema
            .get("properties")
            .is_some_and(|properties| !properties.is_object())
        {
            return Err(ProviderError::Config(format!(
                "provider-bound function `{}` has non-object schema properties",
                tool.name
            )));
        }
        if schema.get("required").is_some_and(|required| {
            required
                .as_array()
                .is_none_or(|items| items.iter().any(|item| !item.is_string()))
        }) {
            return Err(ProviderError::Config(format!(
                "provider-bound function `{}` has a non-string schema required list",
                tool.name
            )));
        }
    }
    Ok(())
}

/// A provider-bound function is either one of the declared Tool API gateway
/// functions or one of the runtime's closed set of control functions.
///
/// The gateway rule is what keeps an execution tool from ever reaching a
/// provider as its own function name; control functions are the deliberate,
/// tiny exception for runtime-owned requests whose "tool call" *is* the
/// answer (the automatic-approval verdict). Their calls are read and dropped
/// by the component that declared them and never enter the executor, so they
/// carry no execution capability. [`agena_domain::is_control_function_name`]
/// is the single source of truth for the set.
fn validate_provider_bound_tool_name(name: &str) -> Result<(), String> {
    if agena_domain::ToolApiFunction::from_function_name(name).is_some()
        || agena_domain::is_control_function_name(name)
    {
        return Ok(());
    }
    Err(format!(
        "provider-bound tool {name:?} is not one of the declared Tool API gateway functions and not a runtime control function; execution tools reach a provider only through `tools_call`"
    ))
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
    // A control function's decision is carried by its *name*, not by its
    // arguments, so a call whose arguments were dropped, wrapped, or mangled
    // by a gateway is still a complete answer and must not be sent back to the
    // model for protocol repair. The declaring component reads whatever fields
    // did survive (see `ClassifierVerdict::from_tool_call`).
    if agena_domain::is_control_function_name(name) {
        return Ok(());
    }
    let Some(arguments) = parse_tool_api_arguments_tolerant(arguments_json) else {
        let detail = serde_json::from_str::<serde_json::Value>(arguments_json)
            .err()
            .map(|error| agena_failure::diagnostic::format_error_chain(&error))
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
        "provider `{provider_id}` returned invalid execution-tool name {tool_name:?} for Tool API function `{function}`; use an exact name such as `fs.read` returned by `tools_list`"
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
            let tool_names = match arguments.get("tool") {
                Some(serde_json::Value::String(tool_name)) => vec![tool_name.as_str()],
                Some(serde_json::Value::Array(tool_names)) if !tool_names.is_empty() => tool_names
                    .iter()
                    .map(|tool_name| {
                        tool_name.as_str().ok_or_else(|| {
                            ProviderError::Provider(format!(
                                "provider `{provider_id}` returned `tools_help` with a non-string item in `arguments.tool`"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => {
                    return Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` returned `tools_help` without a string execution-tool name or non-empty array of names in `arguments.tool`"
                    )));
                }
            };
            for tool_name in tool_names {
                validate_execution_tool_name(provider_id, name, tool_name)?;
            }
            Ok(())
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
            let input = arguments.get("input").ok_or_else(|| {
                ProviderError::Provider(format!(
                    "provider `{provider_id}` returned `tools_call` without an object or a JSON string encoding an object in `arguments.input`"
                ))
            })?;
            // Check both supported wire forms without rewriting the provider's
            // original arguments. Dispatch still performs normalization before
            // the selected execution tool's schema validation.
            normalize_tool_call_input(tool_name, input.clone())
                .map(|_| ())
                .map_err(|error| {
                    ProviderError::Provider(format!(
                        "provider `{provider_id}` returned invalid `tools_call` input: {error}"
                    ))
                })
        }
        ToolApiFunction::List | ToolApiFunction::PluginsList => Ok(()),
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
        validate_completion_tool_call(provider_id, name, arguments_json, declared)?;
    }
    Ok(())
}

fn validate_completion_tool_call(
    provider_id: &str,
    name: &str,
    arguments_json: &str,
    declared: &BTreeSet<String>,
) -> Result<(), ProviderError> {
    validate_returned_tool_api_function(provider_id, name, declared)?;
    validate_tool_api_arguments(provider_id, name, arguments_json)
}

/// Keep only the tool calls in a response that individually pass transport
/// validation. Used when protocol repair is exhausted so the run can finish
/// with the valid calls (if any) instead of aborting the whole response.
fn retain_valid_completion_tool_calls(
    provider_id: &str,
    response: &mut CompletionResponse,
    declared: &BTreeSet<String>,
) {
    response.tool_calls.retain(|call| {
        matches!(call, CompletionToolCall::Function { name, arguments_json, .. }
            if validate_completion_tool_call(
                provider_id,
                name,
                arguments_json,
                declared,
            )
            .is_ok())
    });
}

#[derive(Default)]
struct StreamToolApiCallState {
    id: Option<String>,
    name: Option<String>,
    arguments_json: String,
}

#[derive(Default)]
struct StreamToolApiCalls {
    calls: BTreeMap<String, StreamToolApiCallState>,
    aliases: BTreeMap<String, String>,
}

impl StreamToolApiCalls {
    fn state_for_event(
        &mut self,
        stream_key: &str,
        provider_call_id: Option<&str>,
    ) -> (String, &mut StreamToolApiCallState) {
        let provider_call_id = provider_call_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let resolved_key = self.resolve_stream_key(stream_key, provider_call_id);
        let state = self.calls.entry(resolved_key.clone()).or_default();
        if let Some(provider_call_id) = provider_call_id {
            state.id = Some(provider_call_id.to_owned());
        }
        (resolved_key, state)
    }

    fn resolve_stream_key(&mut self, stream_key: &str, provider_call_id: Option<&str>) -> String {
        let Some(provider_call_id) = provider_call_id else {
            if self.calls.contains_key(stream_key) {
                return stream_key.to_owned();
            }
            if let Some(existing_key) = self
                .aliases
                .get(stream_key)
                .filter(|existing_key| self.calls.contains_key(existing_key.as_str()))
            {
                return existing_key.clone();
            }
            return stream_key.to_owned();
        };

        if let Some(existing_key) = self.calls.iter().find_map(|(key, state)| {
            (state.id.as_deref().map(str::trim) == Some(provider_call_id)).then(|| key.clone())
        }) {
            self.aliases
                .insert(stream_key.to_owned(), existing_key.clone());
            return existing_key;
        }

        let canonical_key = format!("id:{provider_call_id}");
        if self.calls.contains_key(canonical_key.as_str()) {
            self.aliases
                .insert(stream_key.to_owned(), canonical_key.clone());
            return canonical_key;
        }

        let existing_stream_key = if self.calls.contains_key(stream_key) {
            Some(stream_key.to_owned())
        } else {
            self.aliases
                .get(stream_key)
                .filter(|existing_key| self.calls.contains_key(existing_key.as_str()))
                .cloned()
        };
        let can_rekey_existing_stream =
            existing_stream_key.as_deref().is_some_and(|existing_key| {
                self.calls.get(existing_key).is_some_and(|state| {
                    state.id.as_deref().is_none()
                        || state.id.as_deref().map(str::trim) == Some(provider_call_id)
                })
            });
        if can_rekey_existing_stream
            && let Some(existing_stream_key) = existing_stream_key
            && existing_stream_key != canonical_key
        {
            let state = self
                .calls
                .remove(existing_stream_key.as_str())
                .expect("checked tool validation stream exists");
            self.calls.insert(canonical_key.clone(), state);
            for alias_target in self.aliases.values_mut() {
                if alias_target == &existing_stream_key {
                    *alias_target = canonical_key.clone();
                }
            }
        }

        self.aliases
            .insert(stream_key.to_owned(), canonical_key.clone());
        canonical_key
    }
}

fn validate_stream_tool_api_event(
    provider_id: &str,
    event: &CompletionStreamEvent,
    declared: &BTreeSet<String>,
    calls: &mut StreamToolApiCalls,
) -> Result<(), ProviderError> {
    match event {
        CompletionStreamEvent::ToolCallDelta {
            stream_key,
            id,
            name,
            arguments_delta,
            ..
        } => {
            let (resolved_key, state) = calls.state_for_event(stream_key, id.as_deref());
            if let Some(name) = name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` changed Tool API function name for stream `{resolved_key}`"
                    )));
                }
                state.name = Some(name.to_owned());
            }
            state.arguments_json.push_str(arguments_delta);
            Ok(())
        }
        CompletionStreamEvent::ToolCallSnapshot {
            stream_key,
            id,
            name,
            arguments_json,
            ..
        } => {
            let (resolved_key, state) = calls.state_for_event(stream_key, id.as_deref());
            if let Some(name) = name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(ProviderError::Provider(format!(
                        "provider `{provider_id}` changed Tool API function name for stream `{resolved_key}`"
                    )));
                }
                state.name = Some(name.to_owned());
            }
            let snapshot = arguments_json.trim();
            let snapshot_is_degenerate = snapshot.is_empty() || snapshot == "{}";
            if !snapshot_is_degenerate || state.arguments_json.trim().is_empty() {
                state.arguments_json.clone_from(arguments_json);
            }
            Ok(())
        }
        CompletionStreamEvent::Completed { .. } => {
            for (stream_key, state) in &calls.calls {
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
        crate::provider::wire_message::validate_provider_native_tool_input_history(&request.turns)?;
        let declared_tool_api_functions = declared_tool_api_functions(&request);
        validate_request_capabilities(model, provider.as_ref(), &mut request)?;
        request.model = model.model_id.clone();
        apply_request_backfill(model, provider.as_ref(), &mut request);
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
                    tracing::warn!(
                        provider_id = model.provider_id.as_ref(),
                        model_id = model.model_id.as_ref(),
                        repair_count,
                        error = %error,
                        "provider exhausted internal Tool API repairs; dropping the rejected tool calls and finishing the response without them"
                    );
                    retain_valid_completion_tool_calls(
                        model.provider_id.as_ref(),
                        &mut response,
                        &declared_tool_api_functions,
                    );
                    merge_completion_usage(&mut response.usage, discarded_usage.take());
                    return Ok(response);
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
        crate::provider::wire_message::validate_provider_native_tool_input_history(&request.turns)?;
        validate_request_capabilities(model, provider.as_ref(), &mut request)?;
        request.model = model.model_id.clone();
        apply_request_backfill(model, provider.as_ref(), &mut request);
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
        request: CompletionRequest,
    ) -> Result<super::super::core::CompletionEventStream, ProviderError> {
        self.complete_stream_inner(model, request, false).await
    }

    /// Emit Tool API fragments in provider order for consumers that retain
    /// non-executable drafts. Validation still precedes execution, but a
    /// rejected call is emitted as-is: the consumer owns draft identities and
    /// can turn the rejection into an ordinary tool-visible failure. This
    /// variant never reissues the provider request.
    pub async fn complete_stream_with_tool_drafts(
        &self,
        model: &ModelRef,
        request: CompletionRequest,
    ) -> Result<super::super::core::CompletionEventStream, ProviderError> {
        self.complete_stream_inner(model, request, true).await
    }

    async fn complete_stream_inner(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
        tool_drafts: bool,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let provider = self.provider_for_model_ref(model)?;
        apply_configured_tool_mode(model, provider.as_ref(), &mut request);
        validate_provider_native_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_native_tool_input_history(&request.turns)?;
        let declared_tool_api_functions = declared_tool_api_functions(&request);
        validate_request_capabilities(model, provider.as_ref(), &mut request)?;
        request.model = model.model_id.clone();
        apply_request_backfill(model, provider.as_ref(), &mut request);
        let provider_id = model.provider_id.to_string();
        let model_id = model.model_id.to_string();
        let adapter_id = model.adapter_id.clone();
        let model_ref = model.clone();
        let provider_for_usage = provider.clone();
        let retry_policy = self.retry_policy;
        let replay_policy = self.stream_replay_policy;
        let provider_resume_policy = provider.stream_resume_policy();
        let replay_safe_enabled = replay_policy.enabled(provider_resume_policy);
        let breaker = Arc::clone(&self.breaker);

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
            let mut has_nudged_empty_response = false;
            let mut retry_pending = false;

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

                if breaker_open(&breaker, &provider_id) {
                    let err = ProviderError::Provider(format!(
                        "provider `{provider_id}` request skipped: circuit breaker is open after consecutive request failures"
                    ));
                    tracing::warn!(
                        provider_id = provider_id.as_str(),
                        operation = "complete_stream",
                        attempt,
                        retries = retry_index,
                        stage = "circuit_open",
                        "provider circuit breaker is open; skipping request to fail fast"
                    );
                    Err(err)?;
                    continue;
                }

                if std::mem::take(&mut retry_pending) {
                    yield CompletionStreamEvent::ProviderRetryStarted {
                        provider_id: model_ref.provider_id.clone(),
                        model: model_ref.model_id.clone(),
                    };
                }

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
                        let can_retry = err.retryable()
                            && retry_index < retry_policy.max_retries
                            && !breaker_open(&breaker, &provider_id);
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
                                message: agena_failure::diagnostic::format_error_chain(&err),
                                retry_index,
                                attempt,
                                max_retries: retry_policy.max_retries,
                                delay_ms: delay.as_millis() as u64,
                            };
                            tokio::time::sleep(delay).await;
                            retry_pending = true;
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

                        breaker_record_failure(&breaker, &provider_id);
                        Err(err)?;
                        continue;
                    }
                };

                let mut emitted_event_in_attempt = false;
                let mut should_restart_stream = false;
                let mut replay_mode = replay_mode_enabled;
                // Replay-safe restart alignment. Even "deterministic"
                // endpoints (e.g. the official OpenAI Responses API) may
                // re-chunk text deltas across independent requests, so strict
                // per-event equality is too brittle: compare the concatenated
                // text/reasoning prefix and reserve exact equality for
                // structural events (tool calls, Completed).
                let replay_expected_text = emitted_history
                    .iter()
                    .filter_map(|event| match event {
                        CompletionStreamEvent::TextDelta { delta, .. } => Some(delta.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                let replay_expected_reasoning = emitted_history
                    .iter()
                    .filter_map(|event| match event {
                        CompletionStreamEvent::ThinkingDelta { delta, .. } => Some(delta.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                let replay_structural_indices = emitted_history
                    .iter()
                    .enumerate()
                    .filter_map(|(index, event)| {
                        if matches!(
                            event,
                            CompletionStreamEvent::TextDelta { .. }
                                | CompletionStreamEvent::ThinkingDelta { .. }
                        ) {
                            None
                        } else {
                            Some(index)
                        }
                    })
                    .collect::<Vec<_>>();
                let mut replay_text = String::new();
                let mut replay_reasoning = String::new();
                let mut text_replay_live = replay_expected_text.is_empty();
                let mut reasoning_replay_live = replay_expected_reasoning.is_empty();
                let mut replay_structural_cursor = 0_usize;
                let mut emitted_events_in_attempt = 0_u64;
                let mut replayed_events_in_attempt = 0_u64;
                let mut emitted_content_in_attempt = false;
                let mut terminal_event_in_attempt = false;
                let mut should_restart_for_protocol_repair = false;
                let mut tool_api_calls = StreamToolApiCalls::default();
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
                            if replay_mode {
                                // Recompute after the previous event may have
                                // completed alignment (its `continue` skipped
                                // the recompute below).
                                replay_mode = !text_replay_live
                                    || !reasoning_replay_live
                                    || replay_structural_cursor
                                        < replay_structural_indices.len();
                            }
                            if replay_mode {
                                let mut replay_replacement: Option<CompletionStreamEvent> = None;
                                match &event {
                                    CompletionStreamEvent::TextDelta { delta, .. } => {
                                        match align_replay_text(
                                            &mut replay_text,
                                            &replay_expected_text,
                                            &mut text_replay_live,
                                            delta.as_str(),
                                        ) {
                                            ReplayTextAlignment::Consumed
                                            | ReplayTextAlignment::Aligned => {
                                                replayed_events_in_attempt += 1;
                                                continue;
                                            }
                                            ReplayTextAlignment::Live { excess } => {
                                                if let CompletionStreamEvent::TextDelta {
                                                    provider_id,
                                                    model,
                                                    ..
                                                } = &event
                                                {
                                                    replay_replacement =
                                                        Some(CompletionStreamEvent::TextDelta {
                                                            provider_id: provider_id.clone(),
                                                            model: model.clone(),
                                                            delta: excess,
                                                        });
                                                }
                                            }
                                            ReplayTextAlignment::Diverged { replayed_chars } => {
                                                let err = ProviderError::Provider(format!(
                                                    "provider stream replay text diverged after {replayed_chars} replayed character(s)"
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
                                                    "provider stream replay text diverged; aborting to avoid duplicate output"
                                                );
                                                Err(err)?;
                                            }
                                        }
                                    }
                                    CompletionStreamEvent::ThinkingDelta { delta, .. } => {
                                        match align_replay_text(
                                            &mut replay_reasoning,
                                            &replay_expected_reasoning,
                                            &mut reasoning_replay_live,
                                            delta.as_str(),
                                        ) {
                                            ReplayTextAlignment::Consumed
                                            | ReplayTextAlignment::Aligned => {
                                                replayed_events_in_attempt += 1;
                                                continue;
                                            }
                                            ReplayTextAlignment::Live { excess } => {
                                                if let CompletionStreamEvent::ThinkingDelta {
                                                    provider_id,
                                                    model,
                                                    ..
                                                } = &event
                                                {
                                                    replay_replacement =
                                                        Some(CompletionStreamEvent::ThinkingDelta {
                                                            provider_id: provider_id.clone(),
                                                            model: model.clone(),
                                                            delta: excess,
                                                        });
                                                }
                                            }
                                            ReplayTextAlignment::Diverged { replayed_chars } => {
                                                let err = ProviderError::Provider(format!(
                                                    "provider stream replay reasoning diverged after {replayed_chars} replayed character(s)"
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
                                                    "provider stream replay reasoning diverged; aborting to avoid duplicate output"
                                                );
                                                Err(err)?;
                                            }
                                        }
                                    }
                                    _ => {
                                        if replay_structural_cursor
                                            < replay_structural_indices.len()
                                            && event
                                                == emitted_history[replay_structural_indices
                                                    [replay_structural_cursor]]
                                        {
                                            replay_structural_cursor += 1;
                                            replayed_events_in_attempt += 1;
                                            continue;
                                        }
                                        // A structural event that is not the next
                                        // expected one, or one that arrives after
                                        // the tracked prefix, cannot be spliced
                                        // into already-emitted output.
                                        let err = ProviderError::Provider(format!(
                                            "provider stream replay prefix diverged at structural event index {replay_structural_cursor}"
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
                                }

                                if let Some(replacement) = replay_replacement {
                                    event = replacement;
                                }
                                replay_mode = !text_replay_live
                                    || !reasoning_replay_live
                                    || replay_structural_cursor
                                        < replay_structural_indices.len();
                                if !replay_mode {
                                    tracing::debug!(
                                        provider_id = provider_id.as_str(),
                                        operation = "complete_stream",
                                        attempt,
                                        status = "replay_prefix_aligned",
                                        replayed_events = replayed_events_in_attempt,
                                        "provider stream replay prefix aligned"
                                    );
                                }
                            }
                            if terminal_event_in_attempt && !matches!(event, CompletionStreamEvent::Completed { .. }) {
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
                                    if tool_drafts {
                                        tracing::warn!(
                                            provider_id = provider_id.as_str(),
                                            model_id = model_id.as_str(),
                                            error = %error,
                                            "emitting rejected Tool API stream call with retained drafts"
                                        );
                                    } else {
                                        tool_api_error.get_or_insert(error);
                                    }
                                }
                                if !tool_drafts {
                                    buffered_tool_api_turn.push(event);
                                    continue;
                                }
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
                                match tool_api_error.take() {
                                    Some(error) if tool_drafts => {
                                        tracing::warn!(
                                            provider_id = provider_id.as_str(),
                                            model_id = model_id.as_str(),
                                            error = %error,
                                            "finishing Tool API stream with a rejected draft call"
                                        );
                                    }
                                    Some(error) => {
                                    if protocol_repair_count < MAX_TOOL_API_REPAIRS {
                                        if let CompletionStreamEvent::Completed { usage, .. } = &mut event {
                                            merge_completion_usage(&mut discarded_usage, usage.take());
                                        }
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
                                    tracing::warn!(
                                        provider_id = provider_id.as_str(),
                                        model_id = model_id.as_str(),
                                        protocol_repair_count,
                                        error = %error,
                                        "provider exhausted internal Tool API stream repairs; dropping the rejected tool calls and finishing the stream without them"
                                    );
                                    buffered_tool_api_turn.retain(|buffered| {
                                        !matches!(
                                            buffered,
                                            CompletionStreamEvent::ToolCallDelta { .. }
                                                | CompletionStreamEvent::ToolCallSnapshot { .. }
                                        )
                                    });
                                    }
                                    None => {}
                                }
                                if let CompletionStreamEvent::Completed { usage, .. } = &mut event {
                                    merge_completion_usage(usage, discarded_usage.take());
                                }

                                for buffered_event in buffered_tool_api_turn.drain(..) {
                                    emitted_events_in_attempt += 1;
                                    emitted_content_in_attempt = true;
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
                            emitted_content_in_attempt |=
                                completion_stream_event_is_content(&event);

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
                            let can_retry_now = err.retryable()
                                && retry_index < retry_policy.max_retries
                                && !breaker_open(&breaker, &provider_id);
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
                                    message: agena_failure::diagnostic::format_error_chain(&err),
                                    retry_index,
                                    attempt,
                                    max_retries: retry_policy.max_retries,
                                    delay_ms: delay.as_millis() as u64,
                                };
                                tokio::time::sleep(delay).await;
                                retry_pending = true;
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
                                    message: agena_failure::diagnostic::format_error_chain(&err),
                                    retry_index,
                                    attempt: replay_retry_index + 1,
                                    max_retries: replay_policy.max_retries_after_output,
                                    delay_ms: delay.as_millis() as u64,
                                };
                                tokio::time::sleep(delay).await;
                                retry_pending = true;
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

                            breaker_record_failure(&breaker, &provider_id);
                            Err(err)?;
                        }
                    }
                }

                if should_restart_for_protocol_repair {
                    continue;
                }

                if replay_mode {
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

                // A completed attempt that produced no content at all (no
                // text, no reasoning, no tool call) is an empty response.
                // Providers (including official endpoints) transiently
                // return empty completions; resample within the request
                // retry budget before surfacing the empty result (mirrors
                // grok's retryable EmptyResponse classification). Nothing
                // was emitted, so the restart is safe for every provider
                // and the replay history is reset to avoid treating the
                // previous attempt's Completed event as a replay prefix.
                if !emitted_content_in_attempt && retry_index < retry_policy.max_retries {
                    let delay = retry_policy.delay_for_retry(retry_index);
                    tracing::warn!(
                        provider_id = provider_id.as_str(),
                        operation = "complete_stream",
                        attempt,
                        retries = retry_index,
                        stage = "empty_response",
                        max_retries = retry_policy.max_retries,
                        latency_ms = elapsed_ms(attempt_started_at),
                        status = "retry_scheduled",
                        retry_reason = "empty_response",
                        delay_ms = delay.as_millis() as u64,
                        "provider stream completed with no content; retrying empty response"
                    );
                    yield CompletionStreamEvent::ProviderRetry {
                        provider_id: model_ref.provider_id.clone(),
                        model: model_ref.model_id.clone(),
                        message: "provider stream returned an empty response; retrying".to_owned(),
                        retry_index,
                        attempt,
                        max_retries: retry_policy.max_retries,
                        delay_ms: delay.as_millis() as u64,
                    };
                    tokio::time::sleep(delay).await;
                    retry_pending = true;
                    retry_index += 1;
                    emitted_history.clear();
                    replay_buffer_exhausted = false;
                    if !has_nudged_empty_response {
                        append_empty_response_nudge(&mut request);
                        has_nudged_empty_response = true;
                    }
                    continue;
                }

                if !emitted_content_in_attempt {
                    // The full empty-response budget was consumed: the request
                    // completed but produced nothing usable, so record it as a
                    // request failure for the circuit breaker too.
                    breaker_record_failure(&breaker, &provider_id);
                } else {
                    breaker_record_success(&breaker, &provider_id);
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
mod tool_input_boundary_tests {
    use super::*;
    use serde_json::{Value, json};

    fn declared_functions() -> BTreeSet<String> {
        ["tools_call", "tools_help", "approve_action", "block_action"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn execution_arguments(input: Value) -> String {
        json!({"tool": "shell.exec", "input": input}).to_string()
    }

    fn completion_response(arguments_json: String) -> CompletionResponse {
        CompletionResponse {
            provider_id: agena_domain::ProviderId::new("test".to_owned()),
            model: agena_domain::ModelId::new("test-model".to_owned()),
            text: String::new(),
            reasoning_text: None,
            finish_reason: None,
            tool_calls: vec![CompletionToolCall::Function {
                id: "call-1".to_owned(),
                name: "tools_call".to_owned(),
                arguments_json,
            }],
            usage: None,
            provider_metadata: None,
        }
    }

    fn completed_event() -> CompletionStreamEvent {
        CompletionStreamEvent::Completed {
            provider_id: agena_domain::ProviderId::new("test".to_owned()),
            model: agena_domain::ModelId::new("test-model".to_owned()),
            finish_reason: None,
            usage: None,
            provider_metadata: None,
            end_turn: None,
        }
    }

    fn stream_argument_chars(
        arguments_json: &str,
        declared: &BTreeSet<String>,
    ) -> StreamToolApiCalls {
        let mut calls = StreamToolApiCalls::default();
        for (index, ch) in arguments_json.chars().enumerate() {
            let event = CompletionStreamEvent::ToolCallDelta {
                provider_id: agena_domain::ProviderId::new("test".to_owned()),
                model: agena_domain::ModelId::new("test-model".to_owned()),
                stream_key: "call:0".to_owned(),
                id: Some("call-1".to_owned()),
                name: (index == 0).then(|| "tools_call".to_owned()),
                arguments_delta: ch.to_string(),
            };
            validate_stream_tool_api_event("test", &event, declared, &mut calls).unwrap();
        }
        calls
    }

    #[test]
    fn complete_calls_accept_object_and_encoded_inputs_without_rewriting_arguments() {
        let input = json!({
            "command": "printf '%s\\n' '\"quoted\"'\nprintf '%s' '你好'",
            "nested": {"items": [null, true, 7, "\\|"]},
        });
        let declared = declared_functions();
        for wire_input in [input.clone(), json!(input.to_string())] {
            let arguments_json = execution_arguments(wire_input.clone());
            let mut response = completion_response(arguments_json.clone());

            validate_completion_tool_calls("test", &response, &declared).unwrap();
            retain_valid_completion_tool_calls("test", &mut response, &declared);

            assert_eq!(
                completion_tool_api_calls(&response),
                vec![RejectedToolApiCall {
                    name: "tools_call".to_owned(),
                    arguments_json: arguments_json.clone(),
                }],
            );
            let CompletionToolCall::Function { arguments_json, .. } = &response.tool_calls[0];
            assert_eq!(
                serde_json::from_str::<Value>(arguments_json).unwrap()["input"],
                wire_input,
            );
        }
    }

    #[test]
    fn streamed_calls_accept_both_input_forms_split_inside_strings_and_escapes() {
        let input = json!({"command": "printf '%s\\n' '\"quoted\"'\nprintf '%s' '你好'"});
        let declared = declared_functions();
        for wire_input in [input.clone(), json!(input.to_string())] {
            let arguments_json = execution_arguments(wire_input);
            let mut calls = stream_argument_chars(&arguments_json, &declared);

            validate_stream_tool_api_event("test", &completed_event(), &declared, &mut calls)
                .unwrap();

            assert_eq!(
                stream_tool_api_calls(&calls),
                vec![RejectedToolApiCall {
                    name: "tools_call".to_owned(),
                    arguments_json,
                }],
            );
        }
    }

    #[test]
    fn stream_snapshots_keep_encoded_input_when_final_snapshot_is_degenerate() {
        let declared = declared_functions();
        let arguments_json =
            execution_arguments(json!(json!({"command": "echo hello"}).to_string()));
        let mut calls = StreamToolApiCalls::default();
        for snapshot in [arguments_json.as_str(), "{}"] {
            let event = CompletionStreamEvent::ToolCallSnapshot {
                provider_id: agena_domain::ProviderId::new("test".to_owned()),
                model: agena_domain::ModelId::new("test-model".to_owned()),
                stream_key: "call:0".to_owned(),
                id: Some("call-1".to_owned()),
                name: Some("tools_call".to_owned()),
                arguments_json: snapshot.to_owned(),
            };
            validate_stream_tool_api_event("test", &event, &declared, &mut calls).unwrap();
        }

        validate_stream_tool_api_event("test", &completed_event(), &declared, &mut calls).unwrap();

        assert_eq!(
            stream_tool_api_calls(&calls),
            vec![RejectedToolApiCall {
                name: "tools_call".to_owned(),
                arguments_json,
            }],
        );
    }

    #[test]
    fn streamed_calls_reject_invalid_inner_json_at_completion_without_erasing_it() {
        let declared = declared_functions();
        for input in ["", "{", "[]", "null", "true", "7", r#""{}""#] {
            let arguments_json = execution_arguments(json!(input));
            let mut calls = stream_argument_chars(&arguments_json, &declared);

            let error =
                validate_stream_tool_api_event("test", &completed_event(), &declared, &mut calls)
                    .unwrap_err();

            assert!(error.to_string().contains("shell.exec"), "{error}");
            assert_eq!(
                stream_tool_api_calls(&calls)[0].arguments_json,
                arguments_json
            );
        }
    }

    #[test]
    fn complete_calls_reject_invalid_encoded_inputs_and_non_object_envelopes() {
        let declared = declared_functions();
        for input in [
            "",
            "{",
            "{} trailing",
            r#"{"command":"\|"}"#,
            "[]",
            "null",
            "true",
            "7",
            r#""{}""#,
        ] {
            let arguments_json = execution_arguments(json!(input));
            let error =
                validate_completion_tool_call("test", "tools_call", &arguments_json, &declared)
                    .unwrap_err();
            assert!(error.to_string().contains("shell.exec"), "{error}");
        }
        for arguments_json in [
            json!(execution_arguments(json!({}))).to_string(),
            "[]".to_owned(),
            r#"{"tool":"shell.exec","input":{"command":invalid}}"#.to_owned(),
        ] {
            assert!(
                validate_completion_tool_call("test", "tools_call", &arguments_json, &declared,)
                    .is_err(),
                "{arguments_json}",
            );
        }
    }

    #[test]
    fn encoded_inputs_keep_protocol_and_control_target_restrictions() {
        let declared = declared_functions();
        for tool in [
            "tools_call",
            "tools_help",
            "tools_list",
            "plugins_list",
            "approve_action",
            "block_action",
        ] {
            let arguments_json = json!({"tool": tool, "input": "{}"}).to_string();
            assert!(
                validate_completion_tool_call("test", "tools_call", &arguments_json, &declared,)
                    .is_err(),
                "{tool}",
            );
        }
        assert!(validate_completion_tool_call("test", "shell.exec", "{}", &declared).is_err());
    }

    #[test]
    fn existing_outer_escape_tolerance_and_direct_control_leniency_are_unchanged() {
        let declared = declared_functions();
        let arguments_json = r#"{"tool":"shell.exec","input":{"command":"printf '\|'"}}"#;
        validate_completion_tool_call("test", "tools_call", arguments_json, &declared).unwrap();
        for name in ["approve_action", "block_action"] {
            for arguments_json in ["", "{invalid", "[]"] {
                validate_completion_tool_call("test", name, arguments_json, &declared).unwrap();
            }
        }
    }

    #[test]
    fn protocol_name_repair_preserves_object_and_encoded_inner_arguments() {
        let declared = declared_functions();
        let input = json!({"tool": ["shell.exec", "fs.read"]});
        for wire_input in [input.clone(), json!(input.to_string())] {
            let calls = vec![RejectedToolApiCall {
                name: "tools_call".to_owned(),
                arguments_json: json!({"tool": "tools_help", "input": wire_input}).to_string(),
            }];
            let original_calls = calls.clone();

            let guidance = tool_name_repair_guidance(&calls, &declared);

            assert_eq!(guidance.len(), 1);
            assert!(guidance[0].contains("Call function `tools_help` directly"));
            assert!(guidance[0].contains(&format!("arguments {}", input)));
            assert_eq!(calls, original_calls);
        }
    }
}
