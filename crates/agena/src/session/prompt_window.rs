#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;

use serde::Serialize;
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::{
    message::{
        AttachmentSource, ExecutionStatus, Message, MessageMetadata, MessagePart, MessageSource,
        OperationPart, PartContent, ToolInvocation,
    },
    plugin::registry::RegisteredTool,
    provider::{
        ProjectedSessionPart, PromptCacheShape, PromptCacheShapeDiff, project_session_parts,
        project_session_text_lossy, project_session_tool_result_output,
    },
    role::Role,
};

use super::Session;
use super::history::{
    ProviderTranscript, TranscriptBlock, TranscriptContent, TranscriptFragment, TranscriptToolCall,
    TranscriptToolOutput,
};
use super::ids::ToolCallId;

const APPROX_CHARS_PER_TOKEN: usize = 4;
const MIN_PROMPT_BUDGET_TOKENS: u32 = 512;
const MIN_CONTEXT_RESERVE_TOKENS: u32 = 1_024;
const MAX_CONTEXT_RESERVE_TOKENS: u32 = 20_000;
const PROMPT_PROTOCOL_OVERHEAD_CHARS: usize = 2_048;
const PROMPT_REQUEST_SHAPE_VERSION: u32 = 4;
const SYNTHETIC_COMPACTION_MESSAGE_ID: i64 = -9_000_000_000;
const SYNTHETIC_TOOL_COMPLETED_PLACEHOLDER: &str =
    "[Tool execution completed without persisted output]";
const SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER: &str = "[Tool execution was interrupted]";
const SYNTHETIC_TOOL_FAILED_PLACEHOLDER: &str = "[Tool execution failed without persisted output]";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedPrompt {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub prompt_cache_key: String,
    pub previous_response_id: Option<String>,
    pub prompt_window_generation: u64,
    pub system_fingerprint: String,
    pub request_options_fingerprint: String,
    pub provider_request_shape: Option<PromptCacheShape>,
    pub continuation_reason: PromptContinuationReason,
    pub continuation_diagnostic: PromptContinuationDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptRequestFingerprint {
    pub system_fingerprint: String,
    pub request_options_fingerprint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptRequestOptions<'a> {
    pub provider_id: &'a str,
    pub model_id: &'a str,
    pub system: Option<&'a str>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub tools: &'a [RegisteredTool],
    pub provider_request_shape: Option<&'a PromptCacheShape>,
    pub continuation_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PromptContinuationDiagnostic {
    pub provider_shape_diff: PromptCacheShapeDiff,
}

impl PromptContinuationDiagnostic {
    pub(crate) fn provider_shape_changed(&self) -> bool {
        !self.provider_shape_diff.is_empty()
    }

    pub(crate) fn provider_shape_change_keys(&self) -> Vec<&str> {
        self.provider_shape_diff.changed_keys()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptContinuationReason {
    Unsupported,
    MissingProviderAnchor,
    PromptWindowGenerationMismatch,
    SystemFingerprintMismatch,
    RequestOptionsFingerprintMismatch,
    AnchorAssistantMissing,
    TranscriptDigestMismatch,
    NoDeltaMessages,
    ProviderContinuation,
}

impl PromptContinuationReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::MissingProviderAnchor => "missing_provider_anchor",
            Self::PromptWindowGenerationMismatch => "prompt_window_generation_mismatch",
            Self::SystemFingerprintMismatch => "system_fingerprint_mismatch",
            Self::RequestOptionsFingerprintMismatch => "request_options_fingerprint_mismatch",
            Self::AnchorAssistantMissing => "anchor_assistant_missing",
            Self::TranscriptDigestMismatch => "transcript_digest_mismatch",
            Self::NoDeltaMessages => "no_delta_messages",
            Self::ProviderContinuation => "provider_continuation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptTokenEstimate {
    pub total_tokens: u64,
    pub delta_tokens: u64,
    pub delta_chars: u64,
}

pub(crate) fn active_prompt_messages(session: &Session) -> Vec<Message> {
    let Some(compaction) = session
        .runtime
        .prompt_window
        .compaction
        .as_ref()
        .filter(|value| !value.summary.trim().is_empty())
    else {
        // Prompt-cache affinity depends on every later request preserving the
        // exact provider-visible prefix from earlier requests. Without an
        // installed compaction snapshot, the prompt path stays append-only.
        return session.messages.clone();
    };

    let mut messages = Vec::new();
    messages.push(compaction_summary_message(
        session,
        compaction.summary.as_str(),
    ));
    messages.extend(
        session
            .messages
            .iter()
            .filter(|message| message_visible_after_compaction(message, compaction))
            .cloned(),
    );
    messages
}

fn compaction_summary_message(session: &Session, summary: &str) -> Message {
    let mut message = Message::prompt_text(
        Role::User,
        format!(
            "Conversation summary before the current active context:\n\n{}",
            summary.trim()
        ),
    );
    message.id = SYNTHETIC_COMPACTION_MESSAGE_ID - session.runtime.prompt_window.generation as i64;
    message.created_at = session.created_at;
    message.metadata = MessageMetadata {
        source: MessageSource::System,
        ..Default::default()
    };
    for (idx, part) in message.parts.iter_mut().enumerate() {
        part.id = message.id - idx as i64 - 1;
        part.message_id = message.id;
        part.created_at = session.created_at;
    }
    message
}

fn message_visible_after_compaction(
    message: &Message,
    compaction: &super::model::PromptCompactionRuntime,
) -> bool {
    let preserved_tail = match (
        compaction.tail_start_message_id,
        compaction.compacted_at_message_id,
    ) {
        (Some(start), Some(end)) => message.id >= start && message.id <= end,
        (Some(start), None) => message.id >= start,
        _ => false,
    };
    let boundary = compaction
        .compacted_by_message_id
        .or(compaction.compacted_at_message_id);
    let future_message = boundary.is_some_and(|id| message.id > id);
    preserved_tail || future_message
}

pub(crate) fn normalize_prompt_messages(messages: &[Message]) -> Vec<Message> {
    let mut normalized = Vec::with_capacity(messages.len());
    let mut next_synthetic_message_id = -1_i64;

    for message in messages {
        if message.role != Role::Assistant {
            normalized.push(message.clone());
            continue;
        }

        if let Some(stripped) = assistant_prompt_message_without_local_tool_results(message) {
            normalized.push(stripped);
            extend_completed_tool_outputs(
                &mut normalized,
                message,
                &mut next_synthetic_message_id,
            );
            continue;
        }

        normalized.push(message.clone());
    }
    normalized.retain(message_has_visible_prompt_payload);
    normalized
}

fn prompt_messages_for_request(messages: &[Message]) -> Vec<Message> {
    normalize_prompt_messages(messages)
}

fn message_has_visible_prompt_payload(message: &Message) -> bool {
    let projected_parts = project_session_parts(message);
    projected_parts.iter().any(prompt_part_has_visible_payload)
        || (projected_parts.is_empty() && !message.as_text_lossy().trim().is_empty())
}

fn prompt_part_has_visible_payload(part: &crate::provider::ProjectedSessionPart) -> bool {
    match part {
        crate::provider::ProjectedSessionPart::Text { text } => !text.trim().is_empty(),
        crate::provider::ProjectedSessionPart::Attachment { .. } => true,
        crate::provider::ProjectedSessionPart::ToolCall { .. } => true,
        crate::provider::ProjectedSessionPart::ToolResult { output_json, .. } => {
            !output_json.trim().is_empty()
        }
    }
}

pub(crate) fn approximate_prompt_payload_chars(messages: &[Message]) -> usize {
    normalize_prompt_messages(messages)
        .iter()
        .map(approximate_message_payload_chars)
        .sum()
}

pub(crate) fn approximate_tokens_from_chars(chars: usize) -> u64 {
    if chars == 0 {
        return 0;
    }

    chars
        .saturating_add(APPROX_CHARS_PER_TOKEN.saturating_sub(1))
        .checked_div(APPROX_CHARS_PER_TOKEN)
        .unwrap_or(usize::MAX) as u64
}

pub(crate) fn prompt_request_fingerprints(
    options: &PromptRequestOptions<'_>,
) -> PromptRequestFingerprint {
    PromptRequestFingerprint {
        system_fingerprint: fingerprint_optional_text(options.system),
        request_options_fingerprint: fingerprint_request_options(
            options.provider_id,
            options.model_id,
            options.temperature,
            options.max_output_tokens,
            options.tools,
            options.provider_request_shape,
        ),
    }
}

pub(crate) fn estimate_prompt_tokens_from_runtime(
    session: &Session,
    messages: &[Message],
    system_fingerprint: &str,
    request_options_fingerprint: &str,
) -> Option<PromptTokenEstimate> {
    let runtime = &session.runtime.prompt_tokens;
    if !runtime.matches_request(
        session.runtime.prompt_window.generation,
        system_fingerprint,
        request_options_fingerprint,
    ) {
        return None;
    }

    let last_successful_prompt_tokens = runtime.prompt_tokens()?;
    let assistant_message_id = runtime.last_successful_assistant_message_id?;
    let prompt_messages = prompt_messages_for_request(messages);
    let anchor_index = prompt_messages
        .iter()
        .position(|message| message.id == assistant_message_id)?;
    if !runtime.transcript_digest.is_empty()
        && prompt_prefix_transcript_digest(prompt_messages.as_slice(), anchor_index)
            != runtime.transcript_digest
    {
        return None;
    }
    let delta_chars = approximate_prompt_payload_chars(&prompt_messages[anchor_index + 1..]);
    let delta_tokens = approximate_tokens_from_chars(delta_chars);

    Some(PromptTokenEstimate {
        total_tokens: last_successful_prompt_tokens.saturating_add(delta_tokens),
        delta_tokens,
        delta_chars: delta_chars as u64,
    })
}

pub(crate) fn prompt_transcript_digest(messages: &[Message]) -> String {
    let normalized = normalize_prompt_messages(messages);
    prompt_prefix_transcript_digest(normalized.as_slice(), normalized.len().saturating_sub(1))
}

/// Compute the prompt-prefix transcript digest by projecting `messages[..=inclusive_end]`
/// into a [`ProviderTranscript`] and hashing it with [`ProviderTranscript::digest_hex`].
///
/// Append-only refactor invariant: the digest depends only on cache-stable content
/// (role, text/reasoning blocks, attachment source, tool call name+arguments,
/// tool result output) — never on mutable per-message state (status,
/// timestamps, in-memory ids).
fn prompt_prefix_transcript_digest(messages: &[Message], inclusive_end: usize) -> String {
    let end = inclusive_end.saturating_add(1).min(messages.len());
    let transcript = messages_to_provider_transcript(&messages[..end]);
    transcript.digest_hex()
}

fn messages_to_provider_transcript(messages: &[Message]) -> ProviderTranscript {
    let mut transcript = ProviderTranscript::new();
    for message in messages {
        let parts = project_session_parts(message);
        match message.role {
            Role::Assistant => {
                let mut content = TranscriptContent::default();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();
                let mut had_any = false;
                for part in parts {
                    match part {
                        ProjectedSessionPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content.push_text(text);
                            }
                        }
                        ProjectedSessionPart::Attachment { item } => {
                            had_any = true;
                            content.blocks.push(attachment_to_transcript_block(&item));
                        }
                        ProjectedSessionPart::ToolCall {
                            id,
                            name,
                            arguments_json,
                        } => {
                            had_any = true;
                            tool_calls.push(TranscriptToolCall {
                                call_id: ToolCallId::from(SmolStr::from(id)),
                                name: SmolStr::from(name),
                                arguments: arguments_json,
                            });
                        }
                        ProjectedSessionPart::ToolResult {
                            tool_call_id,
                            output_json,
                            ..
                        } => {
                            if output_json.is_empty() {
                                continue;
                            }
                            tool_results.push(TranscriptFragment::ToolResult {
                                call_id: ToolCallId::from(SmolStr::from(tool_call_id)),
                                output: TranscriptToolOutput::Text { text: output_json },
                            });
                        }
                    }
                }
                if !had_any {
                    let fallback = message.as_text_lossy();
                    if !fallback.trim().is_empty() {
                        content.push_text(fallback);
                        had_any = true;
                    }
                }
                if had_any {
                    transcript.push(TranscriptFragment::Assistant {
                        content,
                        tool_calls,
                    });
                }
                for result in tool_results {
                    transcript.push(result);
                }
            }
            Role::Tool => {
                for part in parts {
                    if let ProjectedSessionPart::ToolResult {
                        tool_call_id,
                        output_json,
                        ..
                    } = part
                    {
                        if output_json.is_empty() {
                            continue;
                        }
                        transcript.push(TranscriptFragment::ToolResult {
                            call_id: ToolCallId::from(SmolStr::from(tool_call_id)),
                            output: TranscriptToolOutput::Text { text: output_json },
                        });
                    }
                }
            }
            Role::User | Role::System => {
                let mut content = TranscriptContent::default();
                let mut had_any = false;
                for part in parts {
                    match part {
                        ProjectedSessionPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content.push_text(text);
                            }
                        }
                        ProjectedSessionPart::Attachment { item } => {
                            had_any = true;
                            content.blocks.push(attachment_to_transcript_block(&item));
                        }
                        // ToolCall / ToolResult are not produced under user/system roles
                        // by `project_session_parts`; ignore for digest stability.
                        _ => {}
                    }
                }
                if !had_any {
                    let fallback = message.as_text_lossy();
                    if !fallback.trim().is_empty() {
                        content.push_text(fallback);
                        had_any = true;
                    }
                }
                if !had_any {
                    continue;
                }
                let fragment = if matches!(message.role, Role::System) {
                    TranscriptFragment::System {
                        text: content
                            .blocks
                            .iter()
                            .filter_map(|block| match block {
                                TranscriptBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(""),
                    }
                } else {
                    TranscriptFragment::User { content }
                };
                transcript.push(fragment);
            }
        }
    }
    transcript
}

fn attachment_to_transcript_block(item: &crate::message::AttachmentItem) -> TranscriptBlock {
    // Encode attachment identity into a stable text block. The exact wire bytes
    // here are part of the cache-stability contract; only fields that survive
    // a round-trip through the provider participate.
    let source_marker = match &item.source {
        AttachmentSource::Url { url } => format!("url:{}", url.trim()),
        AttachmentSource::DataUrl { url } => format!("data_url:{}", url.trim()),
        AttachmentSource::Base64 { data } => {
            format!("base64:{}", digest_bytes(data.trim().as_bytes()))
        }
        AttachmentSource::FileId { file_id } => format!("file_id:{}", file_id.trim()),
        AttachmentSource::LocalPath { path } => format!("local_path:{}", path.trim()),
    };
    TranscriptBlock::Attachment {
        file_id: SmolStr::from(format!(
            "{}|{}|{}|{}",
            item.kind.as_str(),
            item.mime.trim(),
            item.summary_label(),
            source_marker
        )),
        media_type: Some(SmolStr::from(item.mime.trim())),
    }
}

pub(crate) fn approximate_request_overhead_chars(
    system: Option<&str>,
    tools: &[RegisteredTool],
) -> usize {
    let system_chars = system
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::len)
        .unwrap_or_default();
    let tools_chars = serde_json::to_vec(tools)
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    PROMPT_PROTOCOL_OVERHEAD_CHARS
        .saturating_add(system_chars)
        .saturating_add(tools_chars)
}

pub(crate) fn approximate_total_request_tokens(
    messages: &[Message],
    system: Option<&str>,
    tools: &[RegisteredTool],
) -> u64 {
    let total_chars = approximate_prompt_payload_chars(messages)
        .saturating_add(approximate_request_overhead_chars(system, tools));
    approximate_tokens_from_chars(total_chars)
}

pub(crate) fn prompt_token_budget(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
) -> Option<u32> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)?;
    let min_prompt_tokens = MIN_PROMPT_BUDGET_TOKENS.min(context_window_tokens);
    let max_reserve_tokens = context_window_tokens
        .saturating_sub(min_prompt_tokens)
        .max(1);
    let min_reserve_tokens = MIN_CONTEXT_RESERVE_TOKENS.min(max_reserve_tokens).max(1);
    let requested_reserve_tokens = max_output_tokens
        .unwrap_or(context_window_tokens / 8)
        .max(context_window_tokens / 8);
    let reserve_tokens = requested_reserve_tokens
        .max(min_reserve_tokens)
        .min(MAX_CONTEXT_RESERVE_TOKENS)
        .min(max_reserve_tokens);
    Some(
        context_window_tokens
            .saturating_sub(reserve_tokens)
            .max(min_prompt_tokens),
    )
}

pub(crate) fn prompt_char_budget(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    fallback_max_prompt_chars: usize,
    system: Option<&str>,
    tools: &[RegisteredTool],
) -> usize {
    let overhead_chars = approximate_request_overhead_chars(system, tools);
    let fallback_budget = fallback_max_prompt_chars
        .saturating_sub(overhead_chars)
        .max(APPROX_CHARS_PER_TOKEN * MIN_PROMPT_BUDGET_TOKENS as usize);

    let Some(prompt_tokens) = prompt_token_budget(context_window_tokens, max_output_tokens) else {
        return fallback_budget;
    };
    let prompt_chars = prompt_tokens as usize * APPROX_CHARS_PER_TOKEN;
    prompt_chars
        .saturating_sub(overhead_chars)
        .max(APPROX_CHARS_PER_TOKEN * MIN_PROMPT_BUDGET_TOKENS as usize)
}

pub(crate) fn build_prepared_prompt(
    session: &Session,
    options: PromptRequestOptions<'_>,
) -> PreparedPrompt {
    let active_messages = active_prompt_messages(session);
    let prompt_messages = prompt_messages_for_request(active_messages.as_slice());
    let provider_request_shape = options.provider_request_shape.cloned();
    let PromptRequestFingerprint {
        system_fingerprint,
        request_options_fingerprint,
    } = prompt_request_fingerprints(&options);

    let continuation = evaluate_prompt_continuation(
        session,
        prompt_messages.as_slice(),
        &options,
        system_fingerprint.as_str(),
        request_options_fingerprint.as_str(),
    );

    let continuation_reason = match &continuation {
        PromptContinuationOutcome::Reuse { .. } => PromptContinuationReason::ProviderContinuation,
        PromptContinuationOutcome::Restart { reason, .. } => *reason,
    };
    let continuation_diagnostic = continuation.diagnostic();
    let (messages, previous_response_id) = match continuation {
        PromptContinuationOutcome::Reuse {
            previous_response_id,
            delta_messages,
        } => (delta_messages, Some(previous_response_id)),
        PromptContinuationOutcome::Restart { .. } => (prompt_messages, None),
    };

    PreparedPrompt {
        system: options.system.map(ToOwned::to_owned),
        messages,
        prompt_cache_key: prompt_cache_key_for_session(session),
        previous_response_id,
        prompt_window_generation: session.runtime.prompt_window.generation,
        system_fingerprint,
        request_options_fingerprint,
        provider_request_shape,
        continuation_reason,
        continuation_diagnostic,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PromptContinuationOutcome {
    Restart {
        reason: PromptContinuationReason,
        diagnostic: PromptContinuationDiagnostic,
    },
    Reuse {
        previous_response_id: String,
        delta_messages: Vec<Message>,
    },
}

impl PromptContinuationOutcome {
    fn diagnostic(&self) -> PromptContinuationDiagnostic {
        match self {
            Self::Restart { diagnostic, .. } => diagnostic.clone(),
            Self::Reuse { .. } => PromptContinuationDiagnostic::default(),
        }
    }
}

fn evaluate_prompt_continuation(
    session: &Session,
    prompt_messages: &[Message],
    options: &PromptRequestOptions<'_>,
    system_fingerprint: &str,
    request_options_fingerprint: &str,
) -> PromptContinuationOutcome {
    if !options.continuation_supported {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::Unsupported,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    let Some(anchor) = session
        .runtime
        .provider_anchor(options.provider_id, options.model_id)
    else {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::MissingProviderAnchor,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    };

    if anchor.prompt_window_generation != session.runtime.prompt_window.generation {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::PromptWindowGenerationMismatch,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    if anchor.system_fingerprint != system_fingerprint {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::SystemFingerprintMismatch,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    if anchor.request_options_fingerprint != request_options_fingerprint {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::RequestOptionsFingerprintMismatch,
            diagnostic: PromptContinuationDiagnostic {
                provider_shape_diff: PromptCacheShape::diff(
                    anchor.provider_request_shape.as_ref(),
                    options.provider_request_shape,
                ),
            },
        };
    }

    let Some(anchor_index) = prompt_messages
        .iter()
        .position(|message| message.id == anchor.assistant_message_id)
    else {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::AnchorAssistantMissing,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    };

    if !anchor.transcript_digest.is_empty()
        && prompt_prefix_transcript_digest(prompt_messages, anchor_index)
            != anchor.transcript_digest
    {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::TranscriptDigestMismatch,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    let delta_messages = prompt_messages[anchor_index + 1..].to_vec();
    if delta_messages.is_empty() {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::NoDeltaMessages,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    PromptContinuationOutcome::Reuse {
        previous_response_id: anchor.previous_response_id.clone(),
        delta_messages,
    }
}

pub(crate) fn prompt_cache_key_for_session(session: &Session) -> String {
    format!(
        "agena:w{}:s{}:c{}",
        session.workspace_id,
        session.id,
        session.created_at.timestamp_millis()
    )
}

pub(crate) fn fingerprint_optional_text(value: Option<&str>) -> String {
    fingerprint_value(&(value.map(str::trim).filter(|text| !text.is_empty())))
}

pub(crate) fn extract_response_id(provider_metadata: Option<&serde_json::Value>) -> Option<String> {
    provider_metadata
        .and_then(|metadata| provider_metadata_field(metadata, "response_id"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn provider_metadata_field<'a>(
    metadata: &'a serde_json::Value,
    field: &str,
) -> Option<&'a serde_json::Value> {
    metadata
        .as_object()
        .and_then(|object| object.get(field))
        .or_else(|| {
            metadata
                .as_object()
                .and_then(|object| object.get("provider_metadata"))
                .and_then(serde_json::Value::as_object)
                .and_then(|object| object.get(field))
        })
}

fn approximate_message_payload_chars(message: &Message) -> usize {
    project_session_text_lossy(message)
        .len()
        .saturating_add(assistant_tool_call_payload_chars(message))
        .saturating_add(tool_result_extra_payload_chars(message))
}

fn assistant_tool_call_payload_chars(message: &Message) -> usize {
    if message.role != Role::Assistant {
        return 0;
    }

    message
        .parts
        .iter()
        .map(|part| {
            let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
                return 0;
            };
            if exec.is_provider_native_only() {
                return 0;
            }
            let Some(tool_call_id) = tool_execution_call_id(part, exec) else {
                return 0;
            };
            let invocation = tool_execution_invocation(exec);
            tool_call_id
                .len()
                .saturating_add(tool_invocation_name(invocation).len())
                .saturating_add(tool_invocation_arguments_json(invocation).len())
                .saturating_add(16)
        })
        .sum()
}

fn tool_result_extra_payload_chars(message: &Message) -> usize {
    message
        .parts
        .iter()
        .map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(exec)) if !exec.is_provider_native_only() => {
                tool_result_output_text(part, exec).len()
            }
            _ => 0,
        })
        .sum()
}

fn assistant_prompt_message_without_local_tool_results(message: &Message) -> Option<Message> {
    let mut stripped = message.clone();
    let mut changed = false;

    for part in &mut stripped.parts {
        let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
            continue;
        };
        if exec.is_provider_native_only() {
            continue;
        }
        if matches!(part.status, ExecutionStatus::Completed | ExecutionStatus::Failed) {
            part.status = ExecutionStatus::Pending;
            changed = true;
        }
    }

    changed.then_some(stripped)
}

fn extend_completed_tool_outputs(
    normalized: &mut Vec<Message>,
    assistant: &Message,
    next_synthetic_message_id: &mut i64,
) {
    let mut seen = HashSet::new();
    for part in &assistant.parts {
        if !matches!(part.status, ExecutionStatus::Completed | ExecutionStatus::Failed) {
            continue;
        }
        let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
            continue;
        };
        if exec.is_provider_native_only() {
            continue;
        }
        let Some(tool_call_id) = tool_execution_call_id(part, exec) else {
            continue;
        };
        if !seen.insert(tool_call_id.clone()) {
            continue;
        }
        normalized.push(synthetic_tool_result_message(
            *next_synthetic_message_id,
            tool_call_id,
            exec.call_id,
            tool_execution_invocation(exec).clone(),
            fallback_tool_result_output(part, exec),
        ));
        *next_synthetic_message_id -= 1;
    }
}

fn synthetic_tool_result_message(
    message_id: i64,
    tool_call_id: String,
    call_id: i64,
    invocation: ToolInvocation,
    output_text: String,
) -> Message {
    let mut message = Message::prompt_parts(
        Role::Tool,
        vec![PartContent::Operation(OperationPart::completed(
            call_id,
            invocation,
            output_text,
            Vec::new(),
            Vec::new(),
            crate::message::ToolOutput::default(),
            crate::message::TimeRange::default(),
        ))],
    );
    message.id = message_id;
    if let Some(part) = message.parts.first_mut() {
        part.operation_id = Some(tool_call_id);
    }
    message
}

fn fallback_tool_result_output(part: &MessagePart, exec: &OperationPart) -> String {
    match part.status {
        ExecutionStatus::Pending | ExecutionStatus::InProgress => {
            SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER.to_string()
        }
        ExecutionStatus::Completed => {
            let output_text = exec.model_output.text.as_str();
            if output_text.trim().is_empty() {
                SYNTHETIC_TOOL_COMPLETED_PLACEHOLDER.to_string()
            } else {
                output_text.to_string()
            }
        }
        ExecutionStatus::Failed => {
            let output_text = exec.model_output.text.as_str();
            if !output_text.trim().is_empty() {
                output_text.to_string()
            } else if let Some(error_message) = exec.error_message() {
                if !error_message.trim().is_empty() {
                    error_message.to_string()
                } else {
                    SYNTHETIC_TOOL_FAILED_PLACEHOLDER.to_string()
                }
            } else {
                SYNTHETIC_TOOL_FAILED_PLACEHOLDER.to_string()
            }
        }
        ExecutionStatus::Cancelled => SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER.to_string(),
    }
}

fn tool_execution_call_id(part: &MessagePart, exec: &OperationPart) -> Option<String> {
    let fallback = exec.call_id.to_string();
    let tool_call_id = part
        .operation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(fallback);

    (!tool_call_id.trim().is_empty()).then_some(tool_call_id)
}

fn tool_execution_invocation(exec: &OperationPart) -> &ToolInvocation {
    &exec.invocation
}

fn tool_result_output_text(part: &MessagePart, exec: &OperationPart) -> String {
    match part.status {
        ExecutionStatus::Failed | ExecutionStatus::Completed => {
            project_session_tool_result_output(part.status, exec)
        }
        _ => String::new(),
    }
}

fn tool_invocation_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

fn tool_invocation_arguments_json(invocation: &ToolInvocation) -> String {
    let ToolInvocation { input, .. } = invocation;
    serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string())
}

fn fingerprint_request_options(
    provider_id: &str,
    model_id: &str,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
    tools: &[RegisteredTool],
    provider_request_shape: Option<&PromptCacheShape>,
) -> String {
    #[derive(Serialize)]
    struct RequestOptionsFingerprint<'a> {
        prompt_request_shape_version: u32,
        provider_id: &'a str,
        model_id: &'a str,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        tools: &'a [RegisteredTool],
        provider_request_shape_fingerprint: Option<String>,
    }

    let provider_request_shape_fingerprint =
        provider_request_shape.map(PromptCacheShape::fingerprint);

    fingerprint_value(&RequestOptionsFingerprint {
        prompt_request_shape_version: PROMPT_REQUEST_SHAPE_VERSION,
        provider_id,
        model_id,
        temperature,
        max_output_tokens,
        tools,
        provider_request_shape_fingerprint,
    })
}

fn fingerprint_value<T>(value: &T) -> String
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    digest_bytes(bytes.as_slice())
}

fn digest_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::message::{StructuredObject, TimeRange, ToolOutput};
    use crate::session::model::ProviderPromptAnchor;

    fn user_message(id: i64, text: &str) -> Message {
        let mut message = Message::prompt_text(Role::User, text);
        message.id = id;
        if let Some(part) = message.parts.first_mut() {
            part.id = id * 100 + 1;
            part.message_id = id;
        }
        message
    }

    fn assistant_tool_message(id: i64, status: ExecutionStatus, output: &str) -> Message {
        let invocation = ToolInvocation::new(
            "process.run",
            StructuredObject::try_from(json!({ "command": "date" })).expect("tool input"),
        );
        let mut message = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::Operation(OperationPart::completed(
                7,
                invocation,
                output.to_string(),
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange::default(),
            ))],
        );
        message.id = id;
        if let Some(part) = message.parts.first_mut() {
            part.id = id * 100 + 1;
            part.message_id = id;
            part.operation_id = Some("call_date_1".to_string());
            part.status = status;
        }
        message
    }

    #[test]
    fn prepared_prompt_reuses_response_id_with_tool_result_delta() {
        let options = PromptRequestOptions {
            provider_id: "recording",
            model_id: "recording-model",
            system: None,
            temperature: None,
            max_output_tokens: None,
            tools: &[],
            provider_request_shape: None,
            continuation_supported: true,
        };
        let fingerprints = prompt_request_fingerprints(&options);
        let initial_messages = vec![
            user_message(1, "run date"),
            assistant_tool_message(2, ExecutionStatus::Pending, ""),
        ];
        let transcript_digest = prompt_transcript_digest(initial_messages.as_slice());

        let now = Utc::now();
        let mut session = Session::new(1, 1, "prompt", now).with_messages(vec![
            user_message(1, "run date"),
            assistant_tool_message(2, ExecutionStatus::Completed, "Sat Jul  4 16:50:20 CST 2026"),
        ]);
        session.runtime.set_provider_anchor(ProviderPromptAnchor {
            provider_id: "recording".to_string(),
            model_id: "recording-model".to_string(),
            previous_response_id: "resp_1".to_string(),
            assistant_message_id: 2,
            prompt_window_generation: session.runtime.prompt_window.generation,
            system_fingerprint: fingerprints.system_fingerprint,
            request_options_fingerprint: fingerprints.request_options_fingerprint,
            provider_request_shape: None,
            transcript_digest,
        });

        let prepared = build_prepared_prompt(&session, options);

        assert_eq!(prepared.previous_response_id.as_deref(), Some("resp_1"));
        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::ProviderContinuation
        );
        assert_eq!(prepared.messages.len(), 1);
        assert_eq!(prepared.messages[0].role, Role::Tool);

        let projected = project_session_parts(&prepared.messages[0]);
        assert!(matches!(
            projected.as_slice(),
            [ProjectedSessionPart::ToolResult {
                tool_call_id,
                output_json,
                ..
            }] if tool_call_id == "call_date_1" && output_json == "Sat Jul  4 16:50:20 CST 2026"
        ));
    }
}
