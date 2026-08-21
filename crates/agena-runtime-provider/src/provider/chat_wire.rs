use agena_domain::*;
use agena_domain::{AssistantReasoningField, Role};
use agena_provider::{
    CompletionFinishReason, CompletionInputPart, CompletionInputRun, CompletionToolCall,
};
/// Shared wire types and message-conversion helpers for OpenAI-compatible
/// Chat Completions API endpoints.
///
/// The explicit OpenAI Chat Completions adapter and compatible Chat
/// Completions backends share these structs. Responses and Realtime use their
/// own wire types and never serialize this schema.
use serde_json::Value;

use crate::{
    ProviderError,
    provider::{CompletionResponse, utils, wire_message},
};
use agena_provider::CompletionRequest;

// ─── Request body ────────────────────────────────────────────────────────────

pub use agena_provider::ChatCompletionRequest;

pub use agena_provider::ChatStreamOptions;

// ─── Message types ────────────────────────────────────────────────────────────

pub use agena_provider::ChatMessage;

pub use agena_provider::{ChatFunctionCallRequest, ChatToolCallRequest};

// ─── Tool definition ──────────────────────────────────────────────────────────

pub use agena_provider::{ChatFunctionDefinition, ChatToolDefinition};

// ─── Response format ─────────────────────────────────────────────────────────

pub use agena_provider::openai_chat_response_format as map_response_format;

// ─── Reasoning effort mapping ─────────────────────────────────────────────────

/// Convert a `ThinkingRequest` to an OpenAI `reasoning_effort` string for
/// OpenAI reasoning models. Returns `None` for non-reasoning models or when thinking
/// is disabled / absent.
pub use agena_provider::openai_chat_reasoning_effort as reasoning_effort;

// ─── Response / decode types ──────────────────────────────────────────────────

pub use agena_provider::ChatFunctionCallWire;
pub use agena_provider::{ChatCompletionResponse, ChatDeltaOrMessage, ChatToolCallWire};

pub use agena_provider::{ChatUsage, chat_usage_to_completion};

// ─── Utility helpers ──────────────────────────────────────────────────────────

pub use agena_provider::openai_chat_extract_text as extract_text_from_content;

pub use agena_provider::openai_chat_extract_reasoning_text as extract_reasoning_text_from_fields;

#[cfg(test)]
pub use agena_provider::merge_openai_chat_reasoning_details as merge_reasoning_details;

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatUsage,
        apply_raw_assistant_reasoning_state, chat_usage_to_completion, merge_reasoning_details,
        parse_completion_response, reasoning_effort,
    };
    use agena_domain::ThinkingRequest;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
    use serde_json::{Value, json};

    /// A minimal persisted assistant content part (R6-T5: projections consume
    /// storage `Part` slices, so fixtures are built as parts, not v1 messages).
    fn part(kind: &str, content: Value) -> Part {
        Part {
            part_id: 1,
            kind: kind.to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: Some(1),
            origin_session_id: 1,
            revision: 1,
            started_at_ms: 0,
            finished_at_ms: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            provider_state: None,
        }
    }

    /// A run marker carrying the assistant's provider replay state.
    fn run_marker(provider_state: Value) -> Part {
        let mut marker = part("run", json!({ "run_kind": "assistant" }));
        marker.run_id = None;
        marker.provider_state = Some(provider_state);
        marker
    }

    fn request(parallel_tool_calls: Option<bool>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            tools: None,
            temperature: None,
            max_tokens: None,
            max_completion_tokens: None,
            cache_control: None,
            prompt_cache_key: None,
            parallel_tool_calls,
            stream: true,
            stream_options: None,
            stop: Vec::new(),
            top_p: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
            verbosity: None,
        }
    }

    #[test]
    fn malformed_tool_calls_are_rejected_instead_of_silently_dropped() {
        let malformed_calls = [
            serde_json::json!({ "id": "call-1" }),
            serde_json::json!({ "function": { "name": "tools_help", "arguments": "{}" } }),
            serde_json::json!({ "id": "call-1", "function": { "arguments": "{}" } }),
        ];

        for tool_call in malformed_calls {
            let payload: ChatCompletionResponse = serde_json::from_value(serde_json::json!({
                "model": "test-model",
                "choices": [{
                    "message": { "tool_calls": [tool_call] },
                    "finish_reason": "tool_calls"
                }]
            }))
            .expect("deserialize response");

            let error = parse_completion_response("test", "test-model", payload)
                .expect_err("malformed tool call must fail");
            assert!(error.to_string().contains("returned tool_call without"));
        }
    }

    #[test]
    fn serializes_explicit_parallel_tool_call_policy_without_forcing_a_default() {
        let disabled = serde_json::to_value(request(Some(false))).expect("serialize request");
        assert_eq!(
            disabled.get("parallel_tool_calls"),
            Some(&serde_json::Value::Bool(false))
        );

        let unspecified = serde_json::to_value(request(None)).expect("serialize request");
        assert!(unspecified.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn serializes_the_official_completion_token_field_independently() {
        let mut request = request(None);
        request.max_completion_tokens = Some(4096);

        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(value.get("max_completion_tokens"), Some(&4096.into()));
        assert!(value.get("max_tokens").is_none());
    }

    #[test]
    fn chat_completions_wire_shape_never_uses_responses_fields() {
        let value = serde_json::to_value(request(None)).expect("serialize Chat Completions");

        assert!(value.get("messages").is_some());
        assert!(value.get("input").is_none());
        assert!(value.get("instructions").is_none());
        assert!(value.get("text").is_none());
        assert!(value.get("previous_response_id").is_none());
    }

    #[test]
    fn prompt_cache_key_uses_the_openai_wire_name_only() {
        let mut request = request(None);
        request.prompt_cache_key = Some("session-affinity".to_owned());

        let value = serde_json::to_value(request).expect("serialize Chat Completions");
        assert_eq!(
            value.get("prompt_cache_key"),
            Some(&"session-affinity".into())
        );
        assert!(value.get("promptCacheKey").is_none());
    }

    #[test]
    fn disabled_reasoning_uses_none_only_for_supported_gpt5_versions() {
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "gpt-5.2"),
            Some("none".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "openai/gpt-5.4-codex"),
            Some("none".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "gpt-5"),
            None
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "o4-mini"),
            None
        );
    }

    #[test]
    fn max_reasoning_uses_each_protocols_strongest_wire_value() {
        let thinking = ThinkingRequest::Effort {
            effort: agena_domain::ReasoningEffort::Max,
        };
        assert_eq!(
            reasoning_effort(Some(&thinking), "gpt-5.4-codex"),
            Some("xhigh".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&thinking), "deepseek-v4"),
            Some("max".to_owned())
        );
    }

    #[test]
    fn official_chat_usage_detail_fields_are_normalized() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "prompt_tokens_details": { "cached_tokens": 25 },
            "completion_tokens_details": { "reasoning_tokens": 30 }
        }))
        .expect("deserialize Chat Completions usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn xai_chat_usage_keeps_reasoning_separate_from_visible_completion() {
        // xAI Chat Completions reports completion_tokens as visible output,
        // with reasoning added separately into total_tokens.
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 32,
            "completion_tokens": 9,
            "total_tokens": 135,
            "cost_in_usd_ticks": 37_756_000,
            "prompt_tokens_details": { "cached_tokens": 6 },
            "completion_tokens_details": { "reasoning_tokens": 94 }
        }))
        .expect("deserialize xAI Chat usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 26);
        assert_eq!(usage.cache_read_tokens, 6);
        assert_eq!(usage.output_tokens, 9);
        assert_eq!(usage.reasoning_tokens, 94);
        assert!((usage.total_cost - 0.0037756).abs() < 1e-12);
    }

    #[test]
    fn responses_style_chat_usage_detail_fields_are_normalized() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "input_tokens_details": { "cached_tokens": 25 },
            "output_tokens_details": { "reasoning_tokens": 30 }
        }))
        .expect("deserialize Responses-style usage fields");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn chat_usage_accepts_both_detail_field_names_in_one_payload() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "prompt_tokens_details": { "cached_tokens": 25 },
            "input_tokens_details": { "cached_tokens": 20 },
            "completion_tokens_details": { "reasoning_tokens": 30 },
            "output_tokens_details": { "reasoning_tokens": 10 }
        }))
        .expect("deserialize usage containing both naming conventions");
        let usage = chat_usage_to_completion(usage);

        // Chat Completions names are authoritative when both are populated.
        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn copilot_chat_usage_keeps_separately_reported_reasoning_tokens() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 19_581,
            "completion_tokens": 53,
            "reasoning_tokens": 134,
            "total_tokens": 19_768,
            "prompt_tokens_details": { "cached_tokens": 17_068 }
        }))
        .expect("deserialize Copilot usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 2_513);
        assert_eq!(usage.cache_read_tokens, 17_068);
        assert_eq!(usage.output_tokens, 53);
        assert_eq!(usage.reasoning_tokens, 134);
    }

    #[test]
    fn total_tokens_can_identify_separate_reasoning_without_a_named_field() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 20,
            "total_tokens": 135
        }))
        .expect("deserialize compatible usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.reasoning_tokens, 15);
    }

    #[test]
    fn streamed_reasoning_details_merge_text_and_preserve_provider_state() {
        let mut details = None;
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([{
                "type": "reasoning.text",
                "index": 0,
                "text": "think",
                "format": "unknown"
            }]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([{
                "type": "reasoning.text",
                "index": 0,
                "text": "ing",
                "signature": "opaque-signature"
            }]),
        );

        let details = details.expect("merged reasoning details");
        assert_eq!(details[0]["text"], "thinking");
        assert_eq!(details[0]["format"], "unknown");
        assert_eq!(details[0]["signature"], "opaque-signature");
    }

    #[test]
    fn reasoning_details_preserve_summary_encrypted_and_non_adjacent_order() {
        let mut details = None;
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.text", "index": 0, "text": "first" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary one" }
            ]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher one" },
                { "type": "reasoning.text", "index": 0, "text": "second" }
            ]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.summary", "index": 0, "summary": "summary two" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher two" }
            ]),
        );

        assert_eq!(
            details.expect("ordered reasoning details"),
            serde_json::json!([
                { "type": "reasoning.text", "index": 0, "text": "first" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary one" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher one" },
                { "type": "reasoning.text", "index": 0, "text": "second" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary two" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher two" }
            ])
        );
    }

    #[test]
    fn reasoning_summary_is_emitted_as_thinking_text() {
        let details = serde_json::json!([
            { "type": "reasoning.summary", "index": 0, "summary": "short summary" },
            { "type": "reasoning.encrypted", "index": 0, "data": "opaque" }
        ]);

        assert_eq!(
            super::extract_reasoning_text_from_fields(None, Some(&details), None).as_deref(),
            Some("short summary")
        );
    }

    #[test]
    fn assistant_replay_uses_exact_reasoning_details_and_copilot_opaque_state() {
        let raw_details = serde_json::json!([{
            "type": "reasoning.text",
            "index": 0,
            "text": "thinking",
            "signature": "provider-signature"
        }]);
        let marker = run_marker(json!({
            "openai_chat_reasoning_details": raw_details,
            "copilot_reasoning_opaque": "opaque-state"
        }));
        let think = part("think", json!({ "summary": ["thinking"] }));
        let text = part("text", json!({ "text": "answer" }));
        let mut target = ChatMessage::assistant(Some("answer".into()), None);

        let source = crate::provider::project_completion_input(&[marker, think, text]);
        apply_raw_assistant_reasoning_state(&source, &mut target, "thinking");

        assert_eq!(target.reasoning_details, Some(raw_details));
        assert_eq!(target.reasoning_text.as_deref(), Some("thinking"));
        assert_eq!(target.reasoning_opaque.as_deref(), Some("opaque-state"));
    }

    #[test]
    fn reasoning_content_replays_the_persisted_reasoning_text() {
        // Regression: reasoning parts used to be dropped on projection, so
        // `assistant_reasoning_text` was always empty and a `reasoning_content`
        // model received `"reasoning_content": ""`. Now the reasoning survives
        // projection into `CompletionInputPart::Reasoning` and is replayed.
        let marker = run_marker(json!({ "assistant_reasoning_field": "reasoning_content" }));
        let think = part("think", json!({ "summary": ["think step by step"] }));
        let text = part("text", json!({ "text": "visible answer" }));
        let source = crate::provider::project_completion_input(&[marker, think, text]);

        let messages = super::request_to_chat_messages_with_assistant_reasoning_field(
            &agena_provider::CompletionRequest {
                model: agena_domain::ModelId::new("test-model"),
                system: None,
                turns: vec![source.clone()],
                tool_api_functions: Vec::new(),
                provider_native_tools: Default::default(),
                disable_tools: false,
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
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
            },
            Some("reasoning_content"),
        );
        let assistant = messages
            .iter()
            .find(|message| message.role == "assistant")
            .expect("assistant message");
        assert_eq!(
            assistant
                .reasoning_content
                .as_ref()
                .and_then(|value| value.as_str()),
            Some("think step by step"),
            "reasoning_content must carry the persisted reasoning text, not an empty string"
        );
    }

    #[test]
    fn empty_reasoning_content_is_omitted_not_sent_as_an_empty_string() {
        let mut chat_message = ChatMessage::assistant(Some("answer".into()), None);
        super::apply_assistant_reasoning_field(&mut chat_message, Some("reasoning_content"), "");
        assert_eq!(
            chat_message.reasoning_content, None,
            "empty reasoning must omit the field rather than send \"reasoning_content\": \"\""
        );

        let mut chat_message = ChatMessage::assistant(Some("answer".into()), None);
        super::apply_assistant_reasoning_field(
            &mut chat_message,
            Some("reasoning_content"),
            "real reasoning",
        );
        assert_eq!(
            chat_message.reasoning_content,
            Some(serde_json::Value::String("real reasoning".to_owned()))
        );
    }

    #[test]
    fn notification_on_an_assistant_run_wires_as_a_system_message_never_assistant_text() {
        // The notification-leak regression at the Chat Completions boundary: a
        // settled background operation appends its notification as an
        // Assistant-role part onto the launching assistant run. It must wire as
        // a genuine mid-conversation `system` message — never as the
        // assistant's own reply content, which is exactly how notification JSON
        // surfaced as the model's visible output before the fix.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "proc_test".to_string(),
            operation_kind: "shell".to_string(),
            status: "completed".to_string(),
            summary: "exit 0".to_string(),
            body: "<agena_notification>exit 0</agena_notification>".to_string(),
            ..Default::default()
        };
        let marker = run_marker(json!({}));
        let mut body_part = part(
            "system_notification",
            serde_json::to_value(&notification).expect("notification serializes"),
        );
        body_part.run_id = Some(marker.part_id);

        let source = crate::provider::project_completion_input(&[marker, body_part]);
        assert_eq!(
            source.role,
            agena_domain::Role::Assistant,
            "the notification rides the launching assistant run (no new run)"
        );

        let messages = super::request_to_chat_messages_with_assistant_reasoning_field(
            &agena_provider::CompletionRequest {
                model: agena_domain::ModelId::new("test-model"),
                system: None,
                turns: vec![source],
                tool_api_functions: Vec::new(),
                provider_native_tools: Default::default(),
                disable_tools: false,
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
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
            },
            None,
        );

        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            1,
            "the notification must reach the wire as its own system message"
        );
        let system = messages
            .iter()
            .find(|message| message.role == "system")
            .expect("the notification system message");
        assert_eq!(
            system.content,
            Some(Value::String(
                "<agena_notification>exit 0</agena_notification>".to_owned()
            ))
        );
        assert!(
            !messages.iter().any(|message| message.role == "assistant"),
            "the notification must never wire as assistant reply content"
        );
    }

    #[test]
    fn hook_continuation_on_an_assistant_run_wires_as_a_system_message_never_assistant_text() {
        // The hook-leak regression at the Chat Completions boundary, mirroring
        // the notification test: an agent.stop continuation rides an
        // Assistant-role `hook` part on the launching assistant run. It must
        // wire as a genuine mid-conversation `system` message — never as the
        // assistant's own reply content, which is exactly how hook content
        // would surface as the model's visible output before the fix.
        let marker = run_marker(json!({}));
        let mut hook_part = part(
            "hook",
            serde_json::json!({
                "hook": "agent.stop",
                "summary": "agent.stop hook blocked stop: workflow plan autorun",
                "message": "<plan_context>continue with the next plan step</plan_context>",
            }),
        );
        hook_part.run_id = Some(marker.part_id);

        let source = crate::provider::project_completion_input(&[marker, hook_part]);
        assert_eq!(
            source.role,
            agena_domain::Role::Assistant,
            "the hook rides the launching assistant run (no new run)"
        );

        let messages = super::request_to_chat_messages_with_assistant_reasoning_field(
            &agena_provider::CompletionRequest {
                model: agena_domain::ModelId::new("test-model"),
                system: None,
                turns: vec![source],
                tool_api_functions: Vec::new(),
                provider_native_tools: Default::default(),
                disable_tools: false,
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
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
            },
            None,
        );

        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            1,
            "the hook continuation must reach the wire as its own system message"
        );
        let system = messages
            .iter()
            .find(|message| message.role == "system")
            .expect("the hook continuation system message");
        assert_eq!(
            system.content,
            Some(Value::String(
                "<plan_context>continue with the next plan step</plan_context>".to_owned()
            ))
        );
        assert!(
            !messages.iter().any(|message| message.role == "assistant"),
            "the hook continuation must never wire as assistant reply content"
        );
    }

    #[test]
    fn scheduled_delivery_wires_as_a_system_message_never_assistant_text() {
        // The cron-identity regression at the Chat Completions boundary: a
        // scheduled job's prompt is delivered as an Assistant-role
        // `system_notification` part (`operation_kind` "scheduled_delivery")
        // appended onto the existing run. It must wire as a genuine
        // mid-conversation `system` message — never as the assistant's own
        // reply content, and never as a user message.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "delivery-key-1".to_string(),
            operation_kind: "scheduled_delivery".to_string(),
            status: "submitted".to_string(),
            summary: "Scheduled job job-1 fired".to_string(),
            body: "check the background task list and report".to_string(),
            ..Default::default()
        };
        let marker = run_marker(json!({}));
        let mut body_part = part(
            "system_notification",
            serde_json::to_value(&notification).expect("notification serializes"),
        );
        body_part.run_id = Some(marker.part_id);

        let source = crate::provider::project_completion_input(&[marker, body_part]);
        assert_eq!(
            source.role,
            agena_domain::Role::Assistant,
            "the scheduled delivery rides the existing assistant run (no new run)"
        );

        let messages = super::request_to_chat_messages_with_assistant_reasoning_field(
            &agena_provider::CompletionRequest {
                model: agena_domain::ModelId::new("test-model"),
                system: None,
                turns: vec![source],
                tool_api_functions: Vec::new(),
                provider_native_tools: Default::default(),
                disable_tools: false,
                temperature: None,
                max_output_tokens: None,
                prompt_cache_key: None,
                previous_response_id: None,
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
            },
            None,
        );

        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            1,
            "the scheduled prompt must reach the wire as its own system message"
        );
        let system = messages
            .iter()
            .find(|message| message.role == "system")
            .expect("the scheduled prompt system message");
        assert_eq!(
            system.content,
            Some(Value::String(
                "check the background task list and report".to_owned()
            ))
        );
        assert!(
            !messages.iter().any(|message| message.role == "assistant"),
            "the scheduled prompt must never wire as assistant reply content"
        );
        assert!(
            !messages.iter().any(|message| message.role == "user"),
            "the scheduled prompt must never wire as a user message"
        );
    }
}

pub fn extract_reasoning_text_from_delta_or_message(value: &ChatDeltaOrMessage) -> Option<String> {
    extract_reasoning_text_from_fields(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
        value.reasoning_text.as_ref(),
    )
}

pub fn parse_required_chat_tool_calls(
    provider_id: &str,
    calls: Option<&Vec<ChatToolCallWire>>,
) -> Result<Vec<CompletionToolCall>, ProviderError> {
    calls
        .into_iter()
        .flatten()
        .map(|call| {
            let id = utils::normalize_optional_text(call.id.clone()).ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned tool_call without id in completion response"
                ))
            })?;

            let function = call.function.as_ref().ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned tool_call without function payload"
                ))
            })?;

            let name = utils::optional_non_empty(function.name.clone()).ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned tool_call without function.name"
                ))
            })?;

            Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: function.arguments.clone().unwrap_or_default(),
            })
        })
        .collect()
}

fn parse_completion_response_with_tool_parser(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
    parse_tool_calls: impl FnOnce(
        &str,
        Option<&Vec<ChatToolCallWire>>,
    ) -> Result<Vec<CompletionToolCall>, ProviderError>,
) -> Result<CompletionResponse, ProviderError> {
    if let Some(error) = payload.error.as_ref() {
        let envelope = serde_json::json!({ "error": error });
        return Err(
            utils::chat_stream_error(provider_id, &envelope).unwrap_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned an empty chat error envelope"
                ))
            }),
        );
    }
    let response_message = payload
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .or_else(|| payload.choices.first().and_then(|c| c.delta.as_ref()));
    let reasoning_text = response_message.and_then(extract_reasoning_text_from_delta_or_message);
    let text = response_message
        .and_then(|m| m.content.as_ref())
        .map(extract_text_from_content)
        .or_else(|| payload.choices.first().and_then(|c| c.text.clone()))
        .unwrap_or_default();

    let finish_reason = CompletionFinishReason::from_provider(
        payload
            .choices
            .first()
            .and_then(|c| c.finish_reason.as_deref()),
    );

    let tool_calls = parse_tool_calls(
        provider_id,
        response_message.and_then(|m| m.tool_calls.as_ref()),
    )?;
    let finish_reason =
        CompletionFinishReason::normalize_with_tool_calls(finish_reason, !tool_calls.is_empty());

    if text.is_empty()
        && reasoning_text.is_none()
        && tool_calls.is_empty()
        && finish_reason.is_none()
    {
        return Err(ProviderError::Provider(format!(
            "{provider_id} returned empty completion payload without finish reason"
        )));
    }

    let usage = payload.usage.map(chat_usage_to_completion);
    let response_id = payload.id;
    let provider_metadata = utils::provider_metadata_with_chat_reasoning_state(
        utils::response_id_metadata(response_id),
        response_message.and_then(agena_provider::openai_chat_reasoning_field_from_delta),
        response_message.and_then(|message| message.reasoning_details.clone()),
        response_message.and_then(|message| message.reasoning_opaque.clone()),
    );

    Ok(CompletionResponse {
        provider_id: ProviderId::new(provider_id),
        model: ModelId::new(payload.model.unwrap_or_else(|| default_model.to_owned())),
        text,
        reasoning_text,
        finish_reason,
        tool_calls,
        usage,
        provider_metadata,
    })
}

pub fn parse_completion_response(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, ProviderError> {
    parse_completion_response_with_tool_parser(
        provider_id,
        default_model,
        payload,
        parse_required_chat_tool_calls,
    )
}

pub fn parse_completion_response_with_required_tool_calls(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, ProviderError> {
    parse_completion_response_with_tool_parser(
        provider_id,
        default_model,
        payload,
        parse_required_chat_tool_calls,
    )
}

// ─── Message conversion ───────────────────────────────────────────────────────

/// Convert a `CompletionRequest` into the flat `Vec<ChatMessage>` wire format
/// used by Chat Completions endpoints.
pub fn request_to_chat_messages_with_assistant_reasoning_field(
    request: &CompletionRequest,
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(ChatMessage::system(system.clone()));
    }

    for run in &request.turns {
        let parts = wire_message::project(run);
        match run.role {
            Role::System => messages.push(ChatMessage::system(session_text_lossy(
                run,
                parts.as_slice(),
            ))),
            Role::User => messages.push(ChatMessage::user(message_content_value(
                run,
                parts.as_slice(),
            ))),
            Role::Assistant => {
                messages.extend(assistant_messages_from_parts(
                    run,
                    &parts,
                    assistant_reasoning_field,
                ));
            }
            Role::Tool => messages.extend(tool_messages_from_parts(&parts)),
        }
    }

    messages
}

pub fn backfill_assistant_reasoning_field_on_request(
    request: &mut CompletionRequest,
    assistant_reasoning_field: Option<&str>,
    assistant_reasoning_interleaved: bool,
) {
    let Some(field) = assistant_reasoning_field else {
        return;
    };

    for run in &mut request.turns {
        if !matches!(run.role, Role::Assistant) {
            continue;
        }
        if assistant_reasoning_field_from_message_metadata(run).is_some() {
            continue;
        }
        if !assistant_reasoning_interleaved && assistant_reasoning_text(run).trim().is_empty() {
            continue;
        }
        let assistant_reasoning_field = match field {
            "reasoning_content" => AssistantReasoningField::ReasoningContent,
            "reasoning_details" => AssistantReasoningField::ReasoningDetails,
            _ => continue,
        };
        run.provider_state.assistant_reasoning_field = Some(assistant_reasoning_field);
    }
}

fn session_text_lossy(run: &CompletionInputRun, parts: &[wire_message::WirePart]) -> String {
    if parts.is_empty() {
        run.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(parts)
    }
}

fn message_content_value(run: &CompletionInputRun, parts: &[wire_message::WirePart]) -> Value {
    if parts.is_empty() {
        return Value::String(run.as_text_lossy());
    }
    wire_message::parts_to_openai_content_array(parts)
}

fn assistant_messages_from_parts(
    run: &CompletionInputRun,
    parts: &[wire_message::WirePart],
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let assistant_reasoning_field =
        assistant_reasoning_field_from_message_metadata(run).or(assistant_reasoning_field);
    let assistant_reasoning_text = assistant_reasoning_text(run);

    // Accumulated assistant content, flushed as an assistant ChatMessage each
    // time the stream is interrupted by a tool result or a system notice.
    let mut messages = Vec::new();
    let mut buffered = Vec::new();
    let flush = |messages: &mut Vec<ChatMessage>, buffered: &mut Vec<wire_message::WirePart>| {
        if buffered.is_empty() {
            return;
        }
        let (content, tool_calls) = assistant_content_and_tool_calls(run, buffered);
        let mut chat_message =
            ChatMessage::assistant(content, (!tool_calls.is_empty()).then_some(tool_calls));
        apply_assistant_reasoning_field(
            &mut chat_message,
            assistant_reasoning_field,
            assistant_reasoning_text.as_str(),
        );
        apply_raw_assistant_reasoning_state(run, &mut chat_message, &assistant_reasoning_text);
        messages.push(chat_message);
        buffered.clear();
    };

    for part in parts {
        match part {
            // A system notice (background-operation notification) is a genuine
            // mid-conversation `system` message, never the assistant's reply.
            // Split the stream so the notice lands as its own system message,
            // keeping the surrounding assistant content in role.
            wire_message::WirePart::SystemMessage { text } => {
                flush(&mut messages, &mut buffered);
                if !text.trim().is_empty() {
                    messages.push(ChatMessage::system(text.clone()));
                }
            }
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if !tool_call_id.trim().is_empty() => {
                flush(&mut messages, &mut buffered);
                messages.push(ChatMessage::tool_result(
                    tool_call_id.clone(),
                    Value::String(output_json.clone()),
                ));
            }
            wire_message::WirePart::ToolResult { output_json, .. } => {
                buffered.push(wire_message::WirePart::Text {
                    text: output_json.clone(),
                });
            }
            other => buffered.push(other.clone()),
        }
    }
    flush(&mut messages, &mut buffered);

    messages
}

fn tool_messages_from_parts(parts: &[wire_message::WirePart]) -> Vec<ChatMessage> {
    parts
        .iter()
        .filter_map(|part| match part {
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if !tool_call_id.trim().is_empty() => Some(ChatMessage::tool_result(
                tool_call_id.clone(),
                Value::String(output_json.clone()),
            )),
            _ => None,
        })
        .collect()
}

fn assistant_reasoning_text(run: &CompletionInputRun) -> String {
    run.parts
        .iter()
        .filter_map(|part| match part {
            CompletionInputPart::Reasoning { text } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assistant_reasoning_field_from_message_metadata(
    run: &CompletionInputRun,
) -> Option<&'static str> {
    match run.provider_state.assistant_reasoning_field {
        Some(AssistantReasoningField::ReasoningContent) => Some("reasoning_content"),
        Some(AssistantReasoningField::ReasoningDetails) => Some("reasoning_details"),
        None => None,
    }
}

fn apply_assistant_reasoning_field(
    message: &mut ChatMessage,
    field: Option<&str>,
    reasoning_text: &str,
) {
    match field {
        Some("reasoning_content") => {
            // Never send an empty `reasoning_content` to a provider that
            // requires prior reasoning be passed back: `""` can be rejected
            // just like a missing field, and omitting it lets providers that
            // tolerate a missing field proceed.
            if !reasoning_text.trim().is_empty() {
                message.reasoning_content = Some(Value::String(reasoning_text.to_owned()));
            }
        }
        Some("reasoning_details") => {
            let details = if reasoning_text.trim().is_empty() {
                Vec::new()
            } else {
                vec![serde_json::json!({
                    "type": "reasoning.text",
                    "text": reasoning_text,
                    "index": 0
                })]
            };
            message.reasoning_details = Some(Value::Array(details));
        }
        _ => {}
    }
}

fn apply_raw_assistant_reasoning_state(
    source: &CompletionInputRun,
    target: &mut ChatMessage,
    reasoning_text: &str,
) {
    let state = &source.provider_state;
    if let Some(details) = state.openai_chat_reasoning_details.as_ref() {
        target.reasoning_details = Some(details.clone());
    }
    if let Some(opaque) = state
        .copilot_reasoning_opaque
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        target.reasoning_text = (!reasoning_text.is_empty()).then(|| reasoning_text.to_owned());
        target.reasoning_opaque = Some(opaque.to_owned());
    }
}

fn assistant_content_and_tool_calls(
    run: &CompletionInputRun,
    parts: &[wire_message::WirePart],
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    if parts.is_empty() {
        return (Some(Value::String(run.as_text_lossy())), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            wire_message::WirePart::Text { text } => text_chunks.push(text.clone()),
            wire_message::WirePart::Reasoning { .. } => {
                // Reasoning is replayed through the dedicated reasoning field
                // (see `assistant_reasoning_text`), never as visible content.
            }
            // Defensive fallback: `assistant_content_and_tool_calls` only ever
            // sees assistant-content parts (SystemMessage is intercepted and
            // split out in `assistant_messages_from_parts`), but keep the match
            // total so a notice can never be silently dropped.
            wire_message::WirePart::SystemMessage { text } => text_chunks.push(text.clone()),
            wire_message::WirePart::ToolCall {
                id,
                function,
                arguments_json,
            } => {
                tool_calls.push(ChatToolCallRequest {
                    kind: "function".to_owned(),
                    id: id.clone(),
                    function: ChatFunctionCallRequest {
                        name: function.function_name().to_owned(),
                        arguments: arguments_json.clone(),
                    },
                });
            }
            wire_message::WirePart::Attachment { item } => {
                text_chunks.push(wire_message::hint_text(item));
            }
            wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                text_chunks.push(format!("[tool_result:{tool_call_id}]"));
            }
        }
    }
    let content = (!text_chunks.is_empty()).then(|| Value::String(text_chunks.join("")));
    (content, tool_calls)
}
