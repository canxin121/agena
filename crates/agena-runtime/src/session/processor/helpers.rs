use super::{
    AppError, AssistantReasoningField, BTreeMap, CompletionFinishReason, EventKind,
    ExecutionStatus, FinishReason, HistoryMessageId, Message, MessageProviderState, ToolCallId,
    ToolInvocation,
};
use agena_provider::CompletionInputProviderState;
use agena_storage::MessageIdAllocator;

/// Adapter that returns a single, pre-allocated `MessageId` to satisfy the
/// `RunBuffer` API. The processor reserves message ids via the global session
/// allocator before opening the buffer, so the buffer must adopt that id
/// rather than mint its own.
pub(crate) struct FixedAssistantId {
    next: Option<HistoryMessageId>,
}

impl FixedAssistantId {
    pub(crate) fn new(message_id: i64) -> Self {
        Self {
            next: Some(HistoryMessageId(message_id)),
        }
    }
}

impl MessageIdAllocator for FixedAssistantId {
    fn next_message_id(&mut self) -> HistoryMessageId {
        self.next
            .take()
            .expect("FixedAssistantId only yields one id per run")
    }
}

pub(crate) fn complete_part_status(assistant: &mut Message, part_id: i64) -> Result<(), AppError> {
    let part = assistant
        .parts
        .iter_mut()
        .find(|part| part.id == part_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "completing missing part on assistant snapshot: {part_id}"
            ))
        })?;
    if part.status == ExecutionStatus::InProgress {
        part.transition_status(ExecutionStatus::Completed)
            .map_err(|err| AppError::Internal(err.to_string()))?;
    }
    Ok(())
}

pub(crate) fn cancel_nonterminal_parts(assistant: &mut Message) -> Result<(), AppError> {
    terminalize_nonterminal_parts(assistant, ExecutionStatus::Cancelled)
}

pub(crate) fn fail_nonterminal_parts(assistant: &mut Message) -> Result<(), AppError> {
    terminalize_nonterminal_parts(assistant, ExecutionStatus::Failed)
}

fn terminalize_nonterminal_parts(
    assistant: &mut Message,
    terminal_status: ExecutionStatus,
) -> Result<(), AppError> {
    for part in &mut assistant.parts {
        if matches!(
            part.status,
            ExecutionStatus::Pending | ExecutionStatus::InProgress
        ) {
            part.transition_status(terminal_status)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
    }
    Ok(())
}

pub(crate) fn sync_assistant_completion_event(
    history_items: &mut [EventKind],
    assistant: &Message,
) {
    for event in history_items {
        let EventKind::AssistantMessageFinished(payload) = event else {
            continue;
        };
        if payload.message_id.raw() != assistant.id {
            continue;
        }
        payload.parts = assistant.parts.clone();
        payload.usage = assistant.usage.clone();
        payload.metadata = assistant.metadata.clone();
        payload.provider_state = assistant.provider_state.clone();
    }
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
) -> Option<MessageProviderState> {
    let assistant_reasoning_field =
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
    let openai_reasoning_items =
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
    /// History-side call identifier propagated to `RunBuffer`. Set the first
    /// time the part is materialized and reused for every subsequent argument
    /// fragment so chunks land on the right tool.
    pub(crate) history_call_id: Option<ToolCallId>,
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
