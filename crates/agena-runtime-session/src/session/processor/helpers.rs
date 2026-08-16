use super::{
    AppError, AssistantReasoningField, BTreeMap, CompletionFinishReason, FinishReason,
    PartProviderState, ToolInvocation,
};
use agena_provider::{CompletionInputProviderState, provider_metadata_value_is_meaningful};
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
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let gemini_thought_signatures =
        provider_metadata_string_field(provider_metadata, "gemini_thought_signatures")
            .and_then(serde_json::Value::as_object)
            .map(|signatures| {
                signatures
                    .iter()
                    .filter_map(|(call_id, signature)| {
                        let call_id = call_id.trim();
                        if call_id.is_empty() {
                            return None;
                        }
                        signature
                            .as_str()
                            .filter(|signature| !signature.trim().is_empty())
                            .map(|signature| (call_id.to_owned(), signature.to_owned()))
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
                .is_some_and(|content| !content.trim().is_empty());
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
            .filter(|value| provider_metadata_value_is_meaningful(value))
            .cloned();
    let copilot_reasoning_opaque =
        provider_metadata_string_field(provider_metadata, "copilot_reasoning_opaque")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
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
        .filter(|value| provider_metadata_value_is_meaningful(value))
        .or_else(|| {
            provider_metadata
                .as_object()
                .and_then(|metadata| metadata.get("provider_metadata"))
                .and_then(serde_json::Value::as_object)
                .and_then(|metadata| metadata.get(field))
                .filter(|value| provider_metadata_value_is_meaningful(value))
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

/// Merge a provider-hosted tool invocation snapshot without letting a late,
/// structurally empty done/progress item erase the richer started snapshot.
/// A genuinely different non-empty tool name is treated as a replacement;
/// otherwise missing fields inherit from the same logical invocation.
pub(crate) fn merge_provider_native_tool_invocation(
    current: Option<&ToolInvocation>,
    mut update: ToolInvocation,
) -> ToolInvocation {
    let Some(current) = current else {
        return update;
    };
    let update_name = update.name.trim();
    let same_tool = update_name.is_empty() || update_name == current.name.trim();
    if !same_tool {
        return update;
    }
    if update_name.is_empty() && !current.name.trim().is_empty() {
        update.name.clone_from(&current.name);
    }
    if update.input.is_empty() && !current.input.is_empty() {
        update.input.clone_from(&current.input);
    }
    if update.plugin_name.is_none() {
        update.plugin_name.clone_from(&current.plugin_name);
    }
    if update.tool_api_call.is_none() {
        update.tool_api_call.clone_from(&current.tool_api_call);
    }
    update
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
    stream_aliases: &mut BTreeMap<String, String>,
    stream_key: String,
    provider_call_id: Option<&str>,
) -> String {
    let Some(provider_call_id) = provider_call_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        if pending_calls.contains_key(stream_key.as_str()) {
            return stream_key;
        }
        if let Some(existing_key) = stream_aliases
            .get(stream_key.as_str())
            .filter(|existing_key| pending_calls.contains_key(existing_key.as_str()))
        {
            return existing_key.clone();
        }
        return stream_key;
    };

    if let Some(existing_key) = pending_calls.iter().find_map(|(key, pending)| {
        (pending.id.as_deref().map(str::trim) == Some(provider_call_id)).then(|| key.clone())
    }) {
        stream_aliases.insert(stream_key, existing_key.clone());
        return existing_key;
    }

    let canonical_key = format!("id:{provider_call_id}");
    if pending_calls.contains_key(canonical_key.as_str()) {
        stream_aliases.insert(stream_key, canonical_key.clone());
        return canonical_key;
    }

    // If an earlier fragment did not include the provider id, retain its
    // materialized operation and history state by moving it under the newly
    // available canonical id instead of creating a second pending operation.
    // Keep every former adapter key as an alias: compatible gateways may omit
    // `call_id` again on a later trailer, and that fragment must still land on
    // the same operation after the map rekey.
    let existing_stream_key = if pending_calls.contains_key(stream_key.as_str()) {
        Some(stream_key.clone())
    } else {
        stream_aliases
            .get(stream_key.as_str())
            .filter(|existing_key| pending_calls.contains_key(existing_key.as_str()))
            .cloned()
    };
    let can_rekey_existing_stream = existing_stream_key.as_deref().is_some_and(|existing_key| {
        pending_calls.get(existing_key).is_some_and(|pending| {
            pending.id.as_deref().is_none()
                || pending.id.as_deref().map(str::trim) == Some(provider_call_id)
        })
    });
    if can_rekey_existing_stream
        && let Some(existing_stream_key) = existing_stream_key
        && existing_stream_key != canonical_key
    {
        let pending = pending_calls
            .remove(existing_stream_key.as_str())
            .expect("checked pending stream key exists");
        pending_calls.insert(canonical_key.clone(), pending);
        for alias_target in stream_aliases.values_mut() {
            if alias_target == &existing_stream_key {
                *alias_target = canonical_key.clone();
            }
        }
    }

    stream_aliases.insert(stream_key, canonical_key.clone());
    canonical_key
}

/// Resolve a provider-hosted tool's stable pending key across started/done
/// events. Hosted tools use an item id rather than a model function-call id,
/// but have the same transport hazard: a compatible gateway may add the id or
/// change its positional stream key only on the terminal event.
pub(crate) fn pending_provider_native_tool_call_stream_key(
    pending_calls: &mut BTreeMap<String, PendingProviderNativeToolCall>,
    stream_aliases: &mut BTreeMap<String, String>,
    stream_key: String,
    provider_item_id: Option<&str>,
) -> String {
    let Some(provider_item_id) = provider_item_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        if pending_calls.contains_key(stream_key.as_str()) {
            return stream_key;
        }
        if let Some(existing_key) = stream_aliases
            .get(stream_key.as_str())
            .filter(|existing_key| pending_calls.contains_key(existing_key.as_str()))
        {
            return existing_key.clone();
        }
        return stream_key;
    };

    if let Some(existing_key) = pending_calls.iter().find_map(|(key, pending)| {
        (pending.id.as_deref().map(str::trim) == Some(provider_item_id)).then(|| key.clone())
    }) {
        stream_aliases.insert(stream_key, existing_key.clone());
        return existing_key;
    }

    let canonical_key = format!("id:{provider_item_id}");
    if pending_calls.contains_key(canonical_key.as_str()) {
        stream_aliases.insert(stream_key, canonical_key.clone());
        return canonical_key;
    }

    let existing_stream_key = if pending_calls.contains_key(stream_key.as_str()) {
        Some(stream_key.clone())
    } else {
        stream_aliases
            .get(stream_key.as_str())
            .filter(|existing_key| pending_calls.contains_key(existing_key.as_str()))
            .cloned()
    };
    let can_rekey_existing_stream = existing_stream_key.as_deref().is_some_and(|existing_key| {
        pending_calls.get(existing_key).is_some_and(|pending| {
            pending.id.as_deref().is_none()
                || pending.id.as_deref().map(str::trim) == Some(provider_item_id)
        })
    });
    if can_rekey_existing_stream
        && let Some(existing_stream_key) = existing_stream_key
        && existing_stream_key != canonical_key
    {
        let pending = pending_calls
            .remove(existing_stream_key.as_str())
            .expect("checked provider-native stream key exists");
        pending_calls.insert(canonical_key.clone(), pending);
        for alias_target in stream_aliases.values_mut() {
            if alias_target == &existing_stream_key {
                *alias_target = canonical_key.clone();
            }
        }
    }

    stream_aliases.insert(stream_key, canonical_key.clone());
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

#[cfg(test)]
mod provider_state_tests {
    use super::message_provider_state_from_provider_metadata;

    #[test]
    fn empty_direct_fields_do_not_mask_nested_replay_state() {
        let metadata = serde_json::json!({
            "response_id": "   ",
            "gemini_thought_signatures": {},
            "copilot_reasoning_opaque": "   ",
            "openai_chat_reasoning_details": [],
            "provider_metadata": {
                "response_id": "resp_nested",
                "gemini_thought_signatures": {
                    "   ": "ignored",
                    "call_1": "signed",
                    "call_2": "   "
                },
                "copilot_reasoning_opaque": "opaque_nested",
                "openai_chat_reasoning_details": [{
                    "type": "reasoning.text",
                    "text": "nested reasoning"
                }]
            }
        });

        let state = message_provider_state_from_provider_metadata(&metadata)
            .expect("nested replay state remains meaningful");
        assert_eq!(state.response_id.as_deref(), Some("resp_nested"));
        assert_eq!(
            state.gemini_thought_signatures,
            std::collections::BTreeMap::from([("call_1".to_owned(), "signed".to_owned())])
        );
        assert_eq!(
            state.copilot_reasoning_opaque.as_deref(),
            Some("opaque_nested")
        );
        assert_eq!(
            state
                .openai_chat_reasoning_details
                .as_ref()
                .and_then(|value| value.pointer("/0/text"))
                .and_then(serde_json::Value::as_str),
            Some("nested reasoning")
        );
    }

    #[test]
    fn blank_encrypted_reasoning_does_not_hide_plaintext_replay_shape() {
        let metadata = serde_json::json!({
            "openai_reasoning_items": [{
                "type": "reasoning",
                "encrypted_content": "   ",
                "content": [{"type": "reasoning_text", "text": "reasoning"}]
            }]
        });
        let state = message_provider_state_from_provider_metadata(&metadata)
            .expect("plaintext reasoning item remains replay state");
        assert_eq!(
            state.assistant_reasoning_field,
            Some(agena_domain::AssistantReasoningField::ReasoningContent)
        );
    }
}
