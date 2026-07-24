use super::{ModelInputModality, OpenAiResponsesInputItem, chat_wire, prompt_cache, wire_message};

pub(super) use agena_provider::{
    OpenAiOutputItem, OpenAiResponsesResponse, OpenAiUsage,
    openai_responses_reasoning_delta as responses_reasoning_delta,
};

pub(super) fn apply_chat_prompt_cache_hints(messages: &mut [chat_wire::ChatMessage]) {
    let flags = messages
        .iter()
        .map(|message| message.role == "system")
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(message) = messages.get_mut(index) {
            message.copilot_cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

pub(super) fn clear_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    for item in input {
        match item {
            OpenAiResponsesInputItem::Message(message) => message.copilot_cache_control = None,
            OpenAiResponsesInputItem::Reasoning(_) => {}
            OpenAiResponsesInputItem::FunctionCall(item) => item.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCallOutput(item) => item.copilot_cache_control = None,
            OpenAiResponsesInputItem::Raw(_) => {}
        }
    }
}

pub(super) fn session_text_lossy(
    message: &agena_provider::CompletionInputMessage,
    projected_parts: &[wire_message::WirePart],
) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
    }
}

pub(super) fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

pub(super) fn openai_client_version() -> String {
    crate::codex_package_version()
}

pub(super) fn append_query_param(endpoint: &str, key: &str, value: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}{key}={value}")
}

pub(super) fn model_supports_input_modality(
    input_modality: &str,
    modality: ModelInputModality,
) -> bool {
    let normalized = input_modality.trim().to_ascii_lowercase();
    match modality {
        ModelInputModality::Text => normalized == "text",
        ModelInputModality::Image => normalized == "image",
        ModelInputModality::Document => normalized == "document",
        ModelInputModality::Audio => normalized == "audio",
        ModelInputModality::Video => normalized == "video",
        ModelInputModality::File => normalized == "file",
    }
}

pub(super) fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}
