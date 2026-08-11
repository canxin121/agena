use super::{
    AppError, AssistantReasoningField, BTreeMap, CompletionFinishReason, FinishReason,
    PartProviderState, ToolInvocation,
};
use agena_provider::CompletionInputProviderState;
use agena_storage::store::{Part, PartState};

pub(crate) fn complete_part_status(parts: &mut [Part], part_id: i64) -> Result<(), AppError> {
    let part = parts
        .iter_mut()
        .find(|part| part.part_id == part_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "completing missing part on turn accumulator: {part_id}"
            ))
        })?;
    if part.state == PartState::InProgress {
        part.state = PartState::Completed;
    }
    Ok(())
}

pub(crate) fn terminalize_nonterminal_parts(
    parts: &mut [Part],
    terminal_state: PartState,
) -> Result<(), AppError> {
    for part in parts.iter_mut() {
        if matches!(part.state, PartState::Pending | PartState::InProgress) {
            part.state = terminal_state;
        }
    }
    Ok(())
}

pub(crate) fn map_finish_reason(reason: &CompletionFinishReason) -> FinishReason {
    match reason {
        CompletionFinishReason::Stop => FinishReason::Stop,
        CompletionFinishReason::ToolCalls => FinishReason::ToolCalls,
        CompletionFinishReason::Length => FinishReason::MaxTokens,
        CompletionFinishReason::ContentFilter => FinishReason::ContentFilter,
        CompletionFinishReason::Other(_) => FinishReason::Other,
    }
}

pub(crate) fn message_provider_state_from_provider_metadata(
    provider_metadata: &serde_json::Value,
) -> Option<PartProviderState> {
    let mut assistant_reasoning_field =
        provider_metadata_string_field(provider_metadata, "assistant_reasoning_field")
            .and_then(|value| value.as_str())
            .and_then(|value| match value {
                "reasoning_content" => Some(AssistantReasoningField::ReasoningContent),
                "reasoning_details" => Some(AssistantReasoningField::ReasoningDetails),
                _ => None,
            });
    let response_id = provider_metadata_string_field(provider_metadata, "response_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let gemini_thought_signatures =
        provider_metadata_string_field(provider_metadata, "gemini_thought_signatures")
            .and_then(serde_json::Value::as_object)
            .map(|signatures| {
                signatures
                    .iter()
                    .filter_map(|(call_id, signature)| {
                        signature
                            .as_str()
                            .filter(|signature| !signature.is_empty())
                            .map(|signature| (call_id.clone(), signature.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
    let openai_reasoning_items: Vec<serde_json::Value> =
        provider_metadata_string_field(provider_metadata, "openai_reasoning_items")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.is_object())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
    if assistant_reasoning_field.is_none()
        && openai_reasoning_items.iter().any(|item| {
            let has_plaintext_content = item
                .get("content")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|content| !content.is_empty());
            let has_encrypted_content = item
                .get("encrypted_content")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|content| !content.is_empty());
            has_plaintext_content && !has_encrypted_content
        })
    {
        // OpenAI-compatible Responses gateways do not all echo the model's
        // `assistant_reasoning_field` metadata. A reasoning item that carries
        // plaintext `content` but no OpenAI `encrypted_content` is itself the
        // unambiguous replay carrier used by `reasoning_content` models (for
        // example DeepSeek). Persist that observed shape so the next tool
        // follow-up does not silently drop the reasoning item and get rejected
        // by the provider.
        assistant_reasoning_field = Some(AssistantReasoningField::ReasoningContent);
    }
    let anthropic_thinking_blocks =
        provider_metadata_string_field(provider_metadata, "anthropic_thinking_blocks")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.is_object())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
    let openai_chat_reasoning_details =
        provider_metadata_string_field(provider_metadata, "openai_chat_reasoning_details")
            .filter(|value| !value.is_null())
            .cloned();
    let copilot_reasoning_opaque =
        provider_metadata_string_field(provider_metadata, "copilot_reasoning_opaque")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
    let state = CompletionInputProviderState {
        assistant_reasoning_field,
        response_id,
        gemini_thought_signatures,
        anthropic_thinking_blocks,
        openai_reasoning_items,
        openai_chat_reasoning_details,
        copilot_reasoning_opaque,
    };
    (!state.is_empty()).then_some(state.into())
}

pub(crate) fn provider_metadata_string_field<'a>(
    provider_metadata: &'a serde_json::Value,
    field: &str,
) -> Option<&'a serde_json::Value> {
    provider_metadata
        .as_object()
        .and_then(|metadata| metadata.get(field))
        .or_else(|| {
            provider_metadata
                .as_object()
                .and_then(|metadata| metadata.get("provider_metadata"))
                .and_then(serde_json::Value::as_object)
                .and_then(|metadata| metadata.get(field))
        })
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PendingToolCall {
    pub(crate) part_id: Option<i64>,
    pub(crate) call_id: Option<i64>,
    pub(crate) started_at_ms: Option<i64>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) arguments_json: String,
}

/// Pick a stable pending-call key for one provider stream event.
///
/// Provider adapters normally keep `stream_key` stable, but a compatible
/// gateway can change its positional stream key while retaining the same
/// provider call id. The call id is the protocol identity, so it wins over a
/// transient stream key. Conversely, different non-empty call ids must stay
/// independent even when an adapter accidentally reuses a stream key: models
/// are allowed to intentionally invoke the same tool with the same input more
/// than once as long as they issue distinct call ids.
pub(crate) fn pending_tool_call_stream_key(
    pending_calls: &mut BTreeMap<String, PendingToolCall>,
    stream_key: String,
    provider_call_id: Option<&str>,
) -> String {
    let Some(provider_call_id) = provider_call_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return stream_key;
    };

    if let Some(existing_key) = pending_calls.iter().find_map(|(key, pending)| {
        (pending.id.as_deref() == Some(provider_call_id)).then(|| key.clone())
    }) {
        return existing_key;
    }

    let canonical_key = format!("id:{provider_call_id}");
    if pending_calls.contains_key(canonical_key.as_str()) {
        return canonical_key;
    }

    // If an earlier fragment did not include the provider id, retain its
    // materialized operation and history state by moving it under the newly
    // available canonical id instead of creating a second pending operation.
    let can_rekey_existing_stream = pending_calls
        .get(stream_key.as_str())
        .is_some_and(|pending| {
            pending.id.as_deref().is_none() || pending.id.as_deref() == Some(provider_call_id)
        });
    if can_rekey_existing_stream && stream_key != canonical_key {
        let pending = pending_calls
            .remove(stream_key.as_str())
            .expect("checked pending stream key exists");
        pending_calls.insert(canonical_key.clone(), pending);
    }

    canonical_key
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PendingProviderNativeToolCall {
    pub(crate) part_id: Option<i64>,
    pub(crate) call_id: Option<i64>,
    pub(crate) started_at_ms: Option<i64>,
    pub(crate) id: Option<String>,
    pub(crate) invocation: Option<ToolInvocation>,
    pub(crate) title: String,
    pub(crate) raw: Option<serde_json::Value>,
}
