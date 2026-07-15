use futures_util::StreamExt;
use std::collections::{BTreeMap, BTreeSet};

use crate::provider::CompletionToolCall;

use super::{
    AppError, CompletionRequest, CompletionResponse, CompletionStreamEvent, Instant, ModelRef,
    ProviderRegistry, Stream, elapsed_ms, hydrate_usage_cost_from_provider_metadata, retry_reason,
    stream_resume_policy_label, validate_request_capabilities,
};

fn declared_gateway_functions(request: &CompletionRequest) -> BTreeSet<String> {
    request
        .tools
        .iter()
        .map(|tool| tool.protocol_name().to_owned())
        .collect()
}

fn validate_provider_tool_definition_boundary(request: &CompletionRequest) -> Result<(), AppError> {
    const RESERVED_FIELDS: [&str; 2] = ["tools", "functions"];
    let overridden = RESERVED_FIELDS
        .into_iter()
        .filter(|field| request.request_override.body_patch.contains_key(*field))
        .collect::<Vec<_>>();
    if !overridden.is_empty() {
        return Err(AppError::Config(format!(
            "request_override.body_patch cannot override provider tool-definition field(s) {}; declare Agena gateway functions through CompletionRequest.tools and provider-hosted tools through provider_tools",
            overridden
                .into_iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let mut declared = BTreeSet::new();
    for tool in crate::tool::gateway_function_specs(request.tools.as_slice()) {
        if !declared.insert(tool.protocol_name.clone()) {
            return Err(AppError::Config(format!(
                "gateway function `{}` is declared more than once",
                tool.protocol_name
            )));
        }
        let schema = tool.input_schema.as_object().ok_or_else(|| {
            AppError::Config(format!(
                "provider-bound gateway function `{}` must use an object input schema",
                tool.protocol_name
            ))
        })?;
        if schema.get("type").and_then(serde_json::Value::as_str) != Some("object") {
            return Err(AppError::Config(format!(
                "provider-bound gateway function `{}` must use an object input schema",
                tool.protocol_name
            )));
        }
        if schema
            .get("properties")
            .is_some_and(|properties| !properties.is_object())
        {
            return Err(AppError::Config(format!(
                "provider-bound gateway function `{}` has non-object schema properties",
                tool.protocol_name
            )));
        }
        if schema.get("required").is_some_and(|required| {
            required
                .as_array()
                .is_none_or(|items| items.iter().any(|item| !item.is_string()))
        }) {
            return Err(AppError::Config(format!(
                "provider-bound gateway function `{}` has a non-string schema required list",
                tool.protocol_name
            )));
        }
    }
    Ok(())
}

fn validate_returned_gateway_function(
    provider_id: &str,
    name: &str,
    declared: &BTreeSet<String>,
) -> Result<(), AppError> {
    if declared.contains(name) {
        return Ok(());
    }
    let declared = declared.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(AppError::Provider(format!(
        "provider `{provider_id}` returned undeclared gateway function {name:?}; declared functions: [{declared}]"
    )))
}

fn validate_gateway_arguments(
    provider_id: &str,
    name: &str,
    arguments_json: &str,
) -> Result<(), AppError> {
    let arguments: serde_json::Value = serde_json::from_str(arguments_json).map_err(|error| {
        AppError::Provider(format!(
            "provider `{provider_id}` returned invalid JSON arguments for gateway function `{name}`: {error}"
        ))
    })?;
    if arguments.is_object() {
        return Ok(());
    }
    Err(AppError::Provider(format!(
        "provider `{provider_id}` returned non-object arguments for gateway function `{name}`"
    )))
}

fn validate_completion_tool_calls(
    provider_id: &str,
    response: &CompletionResponse,
    declared: &BTreeSet<String>,
) -> Result<(), AppError> {
    for call in &response.tool_calls {
        let CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } = call;
        validate_returned_gateway_function(provider_id, name, declared)?;
        validate_gateway_arguments(provider_id, name, arguments_json)?;
    }
    Ok(())
}

#[derive(Default)]
struct StreamGatewayCallState {
    name: Option<String>,
    arguments_json: String,
}

fn validate_stream_gateway_event(
    provider_id: &str,
    event: &CompletionStreamEvent,
    declared: &BTreeSet<String>,
    calls: &mut BTreeMap<String, StreamGatewayCallState>,
) -> Result<(), AppError> {
    match event {
        CompletionStreamEvent::ToolCallDelta {
            stream_key,
            name,
            arguments_delta,
            ..
        } => {
            if let Some(name) = name {
                validate_returned_gateway_function(provider_id, name, declared)?;
            }
            let state = calls.entry(stream_key.clone()).or_default();
            if let Some(name) = name {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(AppError::Provider(format!(
                        "provider `{provider_id}` changed gateway function name for stream `{stream_key}`"
                    )));
                }
                state.name = Some(name.clone());
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
            if let Some(name) = name {
                validate_returned_gateway_function(provider_id, name, declared)?;
            }
            let state = calls.entry(stream_key.clone()).or_default();
            if let Some(name) = name {
                if state
                    .name
                    .as_deref()
                    .is_some_and(|existing| existing != name)
                {
                    return Err(AppError::Provider(format!(
                        "provider `{provider_id}` changed gateway function name for stream `{stream_key}`"
                    )));
                }
                state.name = Some(name.clone());
            }
            state.arguments_json.clone_from(arguments_json);
            Ok(())
        }
        CompletionStreamEvent::Completed { .. } => {
            for (stream_key, state) in calls.iter() {
                let name = state.name.as_deref().ok_or_else(|| {
                    AppError::Provider(format!(
                        "provider `{provider_id}` completed gateway tool call `{stream_key}` without a function name"
                    ))
                })?;
                validate_gateway_arguments(provider_id, name, state.arguments_json.as_str())?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl ProviderRegistry {
    pub async fn complete(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, AppError> {
        validate_provider_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_tool_history(&request.messages)?;
        let declared_gateway_functions = declared_gateway_functions(&request);
        let provider = self.provider_for_model_ref(model)?;
        validate_request_capabilities(model, provider.as_ref(), &request)?;
        request.model = model.model_id.clone();
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
        validate_completion_tool_calls(
            model.provider_id.as_ref(),
            &response,
            &declared_gateway_functions,
        )?;
        hydrate_usage_cost_from_provider_metadata(provider.as_ref(), model, &mut response.usage);
        Ok(response)
    }

    pub async fn compact_conversation(
        &self,
        model: &ModelRef,
        mut request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        validate_provider_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_tool_history(&request.messages)?;
        let provider = self.provider_for_model_ref(model)?;
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
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        validate_provider_tool_definition_boundary(&request)?;
        crate::provider::wire_message::validate_provider_tool_history(&request.messages)?;
        let declared_gateway_functions = declared_gateway_functions(&request);
        let provider = self.provider_for_model_ref(model)?;
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
            let mut gateway_calls = BTreeMap::<String, StreamGatewayCallState>::new();

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

                                let err = AppError::Provider(format!(
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
                                Err(AppError::Provider(format!(
                                    "provider `{provider_id}` emitted a stream event after Completed"
                                )))?;
                            }
                            if matches!(event, CompletionStreamEvent::Completed { .. }) {
                                terminal_event_in_attempt = true;
                            }
                            validate_stream_gateway_event(
                                provider_id.as_str(),
                                &event,
                                &declared_gateway_functions,
                                &mut gateway_calls,
                            )?;
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

                if replay_mode && replay_cursor < emitted_history.len() {
                    let err = AppError::Provider(
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
                    Err(AppError::Provider(format!(
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
mod gateway_function_validation_tests {
    use std::collections::BTreeSet;

    use super::{
        validate_gateway_arguments, validate_provider_tool_definition_boundary,
        validate_returned_gateway_function,
    };
    use crate::plugin::registry::RegisteredTool;
    use crate::plugin::sdk::{PluginKey, ToolDefinition};
    use crate::provider::CompletionRequest;
    use crate::tool::GatewayToolBinding;

    fn request_with_gateway_schema(schema: serde_json::Value) -> CompletionRequest {
        let mut definition: ToolDefinition = serde_json::from_value(serde_json::json!({
            "name": "help"
        }))
        .expect("tool definition");
        definition.contract.input_schema = schema;
        let handler = RegisteredTool::new(
            PluginKey::new("agena", "tools").expect("plugin key"),
            definition,
        )
        .expect("registered gateway handler");
        let binding = GatewayToolBinding::from_registered_tool(handler).expect("gateway binding");
        let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "test-model",
            "messages": []
        }))
        .expect("minimal request");
        request.tools.push(binding);
        request
    }

    #[test]
    fn returned_gateway_function_must_exactly_match_a_declaration() {
        let declared = BTreeSet::from(["tools_help".to_owned()]);
        validate_returned_gateway_function("test", "tools_help", &declared)
            .expect("exact declaration");

        for invalid in [
            "agena.tools.help",
            "tools.help",
            " tools_help",
            "tools_help ",
        ] {
            let error = validate_returned_gateway_function("test", invalid, &declared)
                .expect_err("undeclared provider name must fail");
            assert!(error.to_string().contains("undeclared gateway function"));
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

            let error = validate_provider_tool_definition_boundary(&request)
                .expect_err("reserved tool field must fail");
            assert!(error.to_string().contains(field));
        }
    }

    #[test]
    fn only_gateway_definitions_are_subject_to_provider_schema_rules() {
        let valid = request_with_gateway_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }));
        validate_provider_tool_definition_boundary(&valid).expect("object gateway schema");

        let invalid = request_with_gateway_schema(serde_json::json!({ "type": "array" }));
        let error = validate_provider_tool_definition_boundary(&invalid)
            .expect_err("provider-bound schema must be object-shaped");
        assert!(
            error
                .to_string()
                .contains("provider-bound gateway function")
        );

        let mut duplicate = valid;
        duplicate.tools.push(duplicate.tools[0].clone());
        let error = validate_provider_tool_definition_boundary(&duplicate)
            .expect_err("duplicate provider declarations must fail");
        assert!(error.to_string().contains("declared more than once"));
    }

    #[test]
    fn returned_gateway_arguments_must_be_one_complete_json_object() {
        validate_gateway_arguments("test", "tools_help", r#"{"tool":"session.get"}"#)
            .expect("valid object arguments");
        for invalid in ["", "null", "[]", r#"{} trailing"#] {
            let error = validate_gateway_arguments("test", "tools_help", invalid)
                .expect_err("invalid arguments must fail");
            assert!(error.to_string().contains("arguments"));
        }
    }
}
