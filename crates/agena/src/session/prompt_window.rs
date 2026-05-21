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
    plugin::registry::PluginEntry as RegistryPluginEntry,
    provider::{
        ProjectedSessionPart, PromptCacheShape, PromptCacheShapeDiff, project_session_parts,
        project_session_text_lossy,
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
    pub tools: &'a [RegistryPluginEntry],
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
    AppendOnlyFullPrompt,
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
            Self::AppendOnlyFullPrompt => "append_only_full_prompt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptTokenEstimate {
    pub total_tokens: u64,
    pub delta_tokens: u64,
    pub delta_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingToolCallOutput {
    tool_call_id: String,
    call_id: i64,
    invocation: ToolInvocation,
    output_text: String,
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
        tags: vec!["compaction_summary".to_string()],
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
    let mut pending_tool_outputs = Vec::<PendingToolCallOutput>::new();
    let mut next_synthetic_message_id = -1_i64;

    for message in messages {
        if message.role == Role::Assistant {
            remove_pending_outputs_satisfied_by_message(&mut pending_tool_outputs, message);
        }
        if message.role != Role::Assistant {
            flush_synthetic_tool_results(
                &mut normalized,
                &mut pending_tool_outputs,
                &mut next_synthetic_message_id,
            );
        }
        normalized.push(message.clone());
        if message.role == Role::Assistant {
            extend_pending_tool_outputs(&mut pending_tool_outputs, message);
        }
    }

    flush_synthetic_tool_results(
        &mut normalized,
        &mut pending_tool_outputs,
        &mut next_synthetic_message_id,
    );
    normalized.retain(message_has_visible_prompt_payload);
    normalized
}

fn remove_pending_outputs_satisfied_by_message(
    pending: &mut Vec<PendingToolCallOutput>,
    message: &Message,
) {
    let completed_ids = completed_tool_result_ids(message);
    if completed_ids.is_empty() {
        return;
    }
    pending.retain(|item| !completed_ids.contains(item.tool_call_id.as_str()));
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
    tools: &[RegistryPluginEntry],
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
    tools: &[RegistryPluginEntry],
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
    tools: &[RegistryPluginEntry],
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
        PromptContinuationOutcome::Reuse { .. } => PromptContinuationReason::AppendOnlyFullPrompt,
        PromptContinuationOutcome::Restart { reason, .. } => *reason,
    };
    let continuation_diagnostic = continuation.diagnostic();

    PreparedPrompt {
        system: options.system.map(ToOwned::to_owned),
        messages: prompt_messages,
        prompt_cache_key: prompt_cache_key_for_session(session),
        previous_response_id: None,
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
        .and_then(|metadata| metadata.get("response_id"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
pub(crate) fn provider_metadata_with_response_id(
    response_id: Option<String>,
) -> Option<serde_json::Value> {
    response_id.map(|response_id| serde_json::json!({ "response_id": response_id }))
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
            Some(PartContent::Operation(exec)) => tool_result_output_text(part, exec).len(),
            _ => 0,
        })
        .sum()
}

fn extend_pending_tool_outputs(pending: &mut Vec<PendingToolCallOutput>, assistant: &Message) {
    let mut seen = HashSet::new();
    for part in &assistant.parts {
        if !matches!(
            part.status,
            ExecutionStatus::Pending | ExecutionStatus::InProgress
        ) {
            continue;
        }
        let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = tool_execution_call_id(part, exec) else {
            continue;
        };
        if !seen.insert(tool_call_id.clone())
            || pending
                .iter()
                .any(|existing| existing.tool_call_id == tool_call_id)
        {
            continue;
        }
        pending.push(PendingToolCallOutput {
            tool_call_id,
            call_id: exec.call_id,
            invocation: tool_execution_invocation(exec).clone(),
            output_text: fallback_tool_result_output(part, exec),
        });
    }
}

fn flush_synthetic_tool_results(
    normalized: &mut Vec<Message>,
    pending: &mut Vec<PendingToolCallOutput>,
    next_synthetic_message_id: &mut i64,
) {
    for item in pending.drain(..) {
        normalized.push(synthetic_tool_result_message(
            *next_synthetic_message_id,
            item.tool_call_id,
            item.call_id,
            item.invocation,
            item.output_text,
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
        Role::Assistant,
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

fn completed_tool_result_ids(message: &Message) -> HashSet<String> {
    message
        .parts
        .iter()
        .filter(|part| {
            matches!(
                part.status,
                ExecutionStatus::Completed | ExecutionStatus::Failed
            )
        })
        .filter_map(|part| {
            let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
                return None;
            };
            tool_execution_call_id(part, exec)
        })
        .collect()
}

#[cfg(test)]
fn tool_result_ids(message: &Message) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for part in &message.parts {
        let Some(PartContent::Operation(exec)) = part.content.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = tool_execution_call_id(part, exec) else {
            continue;
        };
        if seen.insert(tool_call_id.clone()) {
            ids.push(tool_call_id);
        }
    }
    ids
}

#[cfg(test)]
fn primary_tool_result_id(message: &Message) -> Option<String> {
    tool_result_ids(message).into_iter().next()
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
        ExecutionStatus::Failed => exec
            .output_text()
            .or_else(|| exec.error_message())
            .unwrap_or_default()
            .to_string(),
        ExecutionStatus::Completed => exec.output_text().unwrap_or_default().to_string(),
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
    tools: &[RegistryPluginEntry],
    provider_request_shape: Option<&PromptCacheShape>,
) -> String {
    #[derive(Serialize)]
    struct RequestOptionsFingerprint<'a> {
        prompt_request_shape_version: u32,
        provider_id: &'a str,
        model_id: &'a str,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        tools: &'a [RegistryPluginEntry],
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

    use crate::{
        plugin::PluginToolDecl, plugin::registry::PluginEntry as RegistryPluginEntry, role::Role,
    };

    use super::*;
    use crate::session::{
        PromptCompactionRuntime, PromptCompactionStrategy, PromptWindowRuntime,
        ProviderPromptAnchor, SessionRuntimeState,
    };

    fn test_tool(
        name: &str,
        description: &str,
        tags: impl IntoIterator<Item = crate::plugin::sdk::ToolTag>,
    ) -> RegistryPluginEntry {
        RegistryPluginEntry::new(
            "fixture",
            PluginToolDecl::new(
                name,
                crate::entry::definition::json_schema_for::<serde_json::Value>(),
            )
            .description(description)
            .tags(tags)
            .concurrency_safe(true),
        )
    }

    fn completed_operation(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
    ) -> PartContent {
        PartContent::Operation(OperationPart::completed(
            call_id,
            invocation,
            output_text.into(),
            Vec::new(),
            Vec::new(),
            crate::message::ToolOutput::default(),
            crate::message::TimeRange {
                start_ms: 0,
                end_ms: Some(1),
            },
        ))
    }

    fn pending_operation(
        call_id: i64,
        invocation: ToolInvocation,
        title: impl Into<String>,
    ) -> PartContent {
        PartContent::Operation(OperationPart::pending(
            call_id,
            invocation,
            title.into(),
            crate::message::TimeRange::default(),
        ))
    }

    #[test]
    fn active_prompt_messages_preserves_append_only_order() {
        let mut first = Message::prompt_text(Role::User, "old");
        first.id = 1;
        let first_id = first.id;

        let mut current = Message::prompt_text(Role::User, "new");
        current.id = 2;

        let mut system = Message::prompt_text(Role::System, "system");
        system.id = 3;

        let session = Session::new(7, 1, "prompt", Utc::now()).with_messages(vec![
            first,
            current.clone(),
            system.clone(),
        ]);

        let prompt_messages = active_prompt_messages(&session);
        assert_eq!(prompt_messages.len(), 3);
        assert_eq!(prompt_messages[0].id, first_id);
        assert_eq!(prompt_messages[1].id, current.id);
        assert_eq!(prompt_messages[2].id, system.id);
    }

    #[test]
    fn active_prompt_messages_projects_compacted_summary_tail_and_future_messages() {
        let mut old = Message::prompt_text(Role::User, "old");
        old.id = 1;
        let mut tail = Message::prompt_text(Role::Assistant, "recent answer");
        tail.id = 2;
        let mut compact_request = Message::prompt_text(Role::User, "compact this");
        compact_request.id = 3;
        let mut compact_assistant = Message::prompt_text(Role::Assistant, "summary");
        compact_assistant.id = 4;
        let mut future = Message::prompt_text(Role::User, "next question");
        future.id = 5;

        let mut session = Session::new(7, 1, "prompt", Utc::now()).with_messages(vec![
            old,
            tail.clone(),
            compact_request,
            compact_assistant,
            future.clone(),
        ]);
        session.runtime.prompt_window = PromptWindowRuntime {
            generation: 3,
            compaction: Some(PromptCompactionRuntime {
                summary: "The user discussed the old topic.".to_string(),
                tail_start_message_id: Some(tail.id),
                compacted_at_message_id: Some(tail.id),
                compacted_by_message_id: Some(4),
                strategy: PromptCompactionStrategy::LocalAgent,
                created_at_ms: 123,
            }),
        };

        let prompt_messages = active_prompt_messages(&session);
        assert_eq!(prompt_messages.len(), 3);
        assert_eq!(prompt_messages[0].role, Role::User);
        assert_eq!(prompt_messages[0].metadata.source, MessageSource::System);
        assert!(prompt_messages[0].as_text_lossy().contains("old topic"));
        assert_eq!(prompt_messages[1].id, tail.id);
        assert_eq!(prompt_messages[2].id, future.id);
    }

    #[test]
    fn build_prepared_prompt_keeps_full_prompt_for_strict_extensions() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime = SessionRuntimeState {
            turn: Default::default(),
            prompt_window: PromptWindowRuntime {
                generation: 2,
                ..Default::default()
            },
            prompt_tokens: Default::default(),
            provider_anchors: [(
                SessionRuntimeState::provider_anchor_key("openai", "gpt-5"),
                ProviderPromptAnchor {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-5".to_string(),
                    previous_response_id: "resp_123".to_string(),
                    assistant_message_id: 11,
                    prompt_window_generation: 2,
                    system_fingerprint: fingerprint_optional_text(Some("system")),
                    request_options_fingerprint: fingerprint_request_options(
                        "openai",
                        "gpt-5",
                        Some(0.2),
                        Some(256),
                        &[],
                        None,
                    ),
                    provider_request_shape: None,
                    transcript_digest: String::new(),
                },
            )]
            .into_iter()
            .collect(),
            loaded_deferred_tools: Vec::new(),
            execution: Default::default(),
            goal: Default::default(),
            plan: None,
        };

        let prepared = build_prepared_prompt(
            &session,
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: Some("system"),
                temperature: Some(0.2),
                max_output_tokens: Some(256),
                tools: &[],
                provider_request_shape: None,
                continuation_supported: true,
            },
        );

        assert_eq!(prepared.previous_response_id, None);
        assert_eq!(prepared.system.as_deref(), Some("system"));
        assert_eq!(prepared.messages.len(), 2);
        assert_eq!(prepared.messages[0].id, 11);
        assert_eq!(prepared.messages[1].id, 12);
        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::AppendOnlyFullPrompt
        );
    }

    #[test]
    fn estimate_prompt_tokens_from_runtime_adds_delta_after_last_successful_turn() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let transcript_digest = prompt_transcript_digest(&[assistant.clone()]);
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime.prompt_window = PromptWindowRuntime {
            generation: 2,
            ..Default::default()
        };
        session.runtime.record_prompt_tokens(
            11,
            &crate::message::MessageUsage {
                input_tokens: 1_200,
                output_tokens: 200,
                reasoning_tokens: 50,
                cache_write_tokens: 30,
                cache_read_tokens: 20,
                total_cost: 0.0,
            },
            2,
            Some(4_096),
            fingerprint_optional_text(Some("system")),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None),
            transcript_digest,
        );

        let active_messages = active_prompt_messages(&session);
        let estimate = estimate_prompt_tokens_from_runtime(
            &session,
            active_messages.as_slice(),
            fingerprint_optional_text(Some("system")).as_str(),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None)
                .as_str(),
        )
        .expect("runtime prompt token estimate should be available");

        let delta_tokens = approximate_tokens_from_chars(approximate_prompt_payload_chars(&[
            Message::prompt_text(Role::User, "follow up"),
        ]));
        assert_eq!(estimate.delta_tokens, delta_tokens);
        assert_eq!(estimate.total_tokens, 1_250 + delta_tokens);
    }

    #[test]
    fn estimate_prompt_tokens_from_runtime_requires_matching_request_shape() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let transcript_digest = prompt_transcript_digest(&[assistant.clone()]);
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime.prompt_window = PromptWindowRuntime {
            generation: 2,
            ..Default::default()
        };
        session.runtime.record_prompt_tokens(
            11,
            &crate::message::MessageUsage {
                input_tokens: 1_200,
                output_tokens: 200,
                reasoning_tokens: 50,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            },
            2,
            Some(4_096),
            fingerprint_optional_text(Some("system")),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None),
            transcript_digest,
        );

        let active_messages = active_prompt_messages(&session);
        let estimate = estimate_prompt_tokens_from_runtime(
            &session,
            active_messages.as_slice(),
            fingerprint_optional_text(Some("different system")).as_str(),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None)
                .as_str(),
        );

        assert!(estimate.is_none());
    }

    #[test]
    fn build_prepared_prompt_rejects_previous_response_id_when_transcript_digest_mismatches() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime = SessionRuntimeState {
            turn: Default::default(),
            prompt_window: PromptWindowRuntime {
                generation: 2,
                ..Default::default()
            },
            prompt_tokens: Default::default(),
            provider_anchors: [(
                SessionRuntimeState::provider_anchor_key("openai", "gpt-5"),
                ProviderPromptAnchor {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-5".to_string(),
                    previous_response_id: "resp_123".to_string(),
                    assistant_message_id: 11,
                    prompt_window_generation: 2,
                    system_fingerprint: fingerprint_optional_text(Some("system")),
                    request_options_fingerprint: fingerprint_request_options(
                        "openai",
                        "gpt-5",
                        Some(0.2),
                        Some(256),
                        &[],
                        None,
                    ),
                    provider_request_shape: None,
                    transcript_digest: prompt_transcript_digest(&[Message::prompt_text(
                        Role::Assistant,
                        "different",
                    )]),
                },
            )]
            .into_iter()
            .collect(),
            loaded_deferred_tools: Vec::new(),
            execution: Default::default(),
            goal: Default::default(),
            plan: None,
        };

        let prepared = build_prepared_prompt(
            &session,
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: Some("system"),
                temperature: Some(0.2),
                max_output_tokens: Some(256),
                tools: &[],
                provider_request_shape: None,
                continuation_supported: true,
            },
        );

        assert_eq!(prepared.previous_response_id, None);
        assert_eq!(prepared.system.as_deref(), Some("system"));
        assert_eq!(prepared.messages.len(), 2);
        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::TranscriptDigestMismatch
        );
    }

    #[test]
    fn estimate_prompt_tokens_from_runtime_requires_matching_transcript_digest() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime.prompt_window = PromptWindowRuntime {
            generation: 2,
            ..Default::default()
        };
        session.runtime.record_prompt_tokens(
            11,
            &crate::message::MessageUsage {
                input_tokens: 1_200,
                output_tokens: 200,
                reasoning_tokens: 50,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            },
            2,
            Some(4_096),
            fingerprint_optional_text(Some("system")),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None),
            prompt_transcript_digest(&[Message::prompt_text(Role::Assistant, "different")]),
        );

        let active_messages = active_prompt_messages(&session);
        let estimate = estimate_prompt_tokens_from_runtime(
            &session,
            active_messages.as_slice(),
            fingerprint_optional_text(Some("system")).as_str(),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None)
                .as_str(),
        );

        assert!(estimate.is_none());
    }

    #[test]
    fn normalize_prompt_messages_synthesizes_missing_tool_results_before_next_turn() {
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![completed_operation(
                17,
                ToolInvocation {
                    name: "edit".to_string(),
                    plugin_name: None,
                    input: crate::message::StructuredObject::try_from(
                        serde_json::json!({ "path": "src/main.rs" }),
                    )
                    .expect("structured tool input"),
                },
                "patched",
            )],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 2;

        let normalized = normalize_prompt_messages(&[assistant, user.clone()]);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].id, 1);
        assert_eq!(normalized[1].id, user.id);
        assert_eq!(normalized[0].role, Role::Assistant);
        assert_eq!(
            primary_tool_result_id(&normalized[0]).as_deref(),
            Some("call_edit")
        );
        assert!(normalized[0].as_text_lossy().contains("patched"));
    }

    #[test]
    fn normalize_prompt_messages_matches_multi_tool_result_message_without_synthesizing_duplicates()
    {
        let invocation = ToolInvocation {
            name: "edit".to_string(),
            plugin_name: None,
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                completed_operation(17, invocation.clone(), "patched main"),
                completed_operation(18, invocation.clone(), "patched lib"),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut tool = Message::prompt_parts(
            Role::Assistant,
            vec![
                completed_operation(17, invocation.clone(), "patched main"),
                completed_operation(18, invocation, "patched lib"),
            ],
        );
        tool.id = 2;
        tool.parts[0].operation_id = Some("call_edit_main".to_string());
        tool.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant, tool.clone(), user.clone()]);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[1].id, tool.id);
        assert_eq!(normalized[2].id, user.id);

        let projected = project_session_parts(&normalized[1]);
        let tool_result_ids = projected
            .iter()
            .filter_map(|part| match part {
                crate::provider::ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                    Some(tool_call_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_result_ids, vec!["call_edit_main", "call_edit_lib"]);
    }

    #[test]
    fn normalize_prompt_messages_keeps_standalone_operation_results() {
        let orphan = Message::prompt_tool_result("call_missing", "stale output");
        let normalized = normalize_prompt_messages(&[orphan]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            primary_tool_result_id(&normalized[0]).as_deref(),
            Some("call_missing")
        );
    }

    #[test]
    fn normalize_prompt_messages_drops_empty_messages_without_visible_prompt_payload() {
        let mut user = Message::prompt_text(Role::User, "hello");
        user.id = 1;
        let mut empty = Message::prompt_text(Role::Assistant, "");
        empty.id = 2;
        let mut follow_up = Message::prompt_text(Role::User, "continue");
        follow_up.id = 3;

        let normalized = normalize_prompt_messages(&[user.clone(), empty, follow_up.clone()]);
        assert_eq!(normalized, vec![user, follow_up]);
    }

    #[test]
    fn normalize_prompt_messages_replaces_empty_matching_tool_results_with_placeholder() {
        let invocation = ToolInvocation {
            name: "edit".to_string(),
            plugin_name: None,
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![pending_operation(17, invocation.clone(), "editing")],
        );
        assistant.id = 1;
        assistant.parts[0].status = ExecutionStatus::Pending;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant, user]);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[1].role, Role::Assistant);
        assert_eq!(
            primary_tool_result_id(&normalized[1]).as_deref(),
            Some("call_edit")
        );
        assert_eq!(
            project_session_text_lossy(&normalized[1]),
            "[tool_call:edit:call_edit][tool_result:call_edit]".to_string()
        );
        assert!(
            normalized[1]
                .as_text_lossy()
                .contains(SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER)
        );
    }

    #[test]
    fn normalize_prompt_messages_expands_empty_multi_tool_results_into_placeholders() {
        let invocation = ToolInvocation {
            name: "edit".to_string(),
            plugin_name: None,
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                pending_operation(17, invocation.clone(), "editing main"),
                pending_operation(18, invocation.clone(), "editing lib"),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].status = ExecutionStatus::Pending;
        assistant.parts[1].status = ExecutionStatus::Pending;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant, user]);
        assert_eq!(normalized.len(), 4);
        assert_eq!(
            normalized
                .iter()
                .skip(1)
                .filter_map(primary_tool_result_id)
                .collect::<Vec<_>>(),
            vec!["call_edit_main", "call_edit_lib"]
        );
        assert!(
            normalized[1]
                .as_text_lossy()
                .contains(SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER)
        );
        assert!(
            normalized[2]
                .as_text_lossy()
                .contains(SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER)
        );
    }

    #[test]
    fn prompt_transcript_digest_treats_empty_tool_results_like_synthesized_placeholders() {
        let invocation = ToolInvocation {
            name: "edit".to_string(),
            plugin_name: None,
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![pending_operation(17, invocation.clone(), "editing")],
        );
        assistant.id = 1;
        assistant.parts[0].status = ExecutionStatus::Pending;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant.clone(), user.clone()]);
        let digest_without_tool = prompt_transcript_digest(&[assistant, user]);
        let digest_with_normalized = prompt_transcript_digest(&normalized);

        assert_eq!(digest_with_normalized, digest_without_tool);
    }

    #[test]
    fn prompt_transcript_digest_treats_empty_multi_tool_results_like_synthesized_placeholders() {
        let invocation = ToolInvocation {
            name: "edit".to_string(),
            plugin_name: None,
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                pending_operation(17, invocation.clone(), "editing main"),
                pending_operation(18, invocation.clone(), "editing lib"),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].status = ExecutionStatus::Pending;
        assistant.parts[1].status = ExecutionStatus::Pending;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant.clone(), user.clone()]);
        let digest_without_tool = prompt_transcript_digest(&[assistant, user]);
        let digest_with_normalized = prompt_transcript_digest(&normalized);

        assert_eq!(digest_with_normalized, digest_without_tool);
    }

    #[test]
    fn prompt_transcript_digest_ignores_empty_messages_without_visible_prompt_payload() {
        let mut user = Message::prompt_text(Role::User, "hello");
        user.id = 1;
        let mut empty = Message::prompt_text(Role::Assistant, "");
        empty.id = 2;
        let mut follow_up = Message::prompt_text(Role::User, "continue");
        follow_up.id = 3;

        let digest_without_empty = prompt_transcript_digest(&[user.clone(), follow_up.clone()]);
        let digest_with_empty = prompt_transcript_digest(&[user, empty, follow_up]);

        assert_eq!(digest_with_empty, digest_without_empty);
    }

    #[test]
    fn prompt_transcript_digest_ignores_legacy_pruned_tool_result_tag() {
        let mut message = Message::prompt_tool_result("call_1", "very long output");
        message.id = 9;
        let baseline = prompt_transcript_digest(&[message.clone()]);
        message.metadata.add_tag("tool_result_pruned");
        let tagged = prompt_transcript_digest(&[message]);
        assert_eq!(tagged, baseline);
    }

    #[test]
    fn prompt_transcript_digest_ignores_legacy_attachment_stripped_tag() {
        let mut message = Message::prompt_parts(
            Role::User,
            vec![
                PartContent::text("see screenshot"),
                PartContent::attachments(vec![crate::message::AttachmentItem {
                    kind: crate::message::AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::DataUrl {
                        url: format!("data:image/png;base64,{}", "A".repeat(1024)),
                    },
                    filename: Some("shot.png".to_string()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }]),
            ],
        );
        message.id = 10;
        let baseline = prompt_transcript_digest(&[message.clone()]);
        message.metadata.add_tag("attachment_payload_stripped");
        let tagged = prompt_transcript_digest(&[message]);
        assert_eq!(tagged, baseline);
    }

    #[test]
    fn extract_response_id_reads_provider_metadata() {
        let metadata = provider_metadata_with_response_id(Some("resp_1".to_string()));
        assert_eq!(
            extract_response_id(metadata.as_ref()),
            Some("resp_1".to_string())
        );
    }

    #[test]
    fn fingerprint_request_options_changes_when_tools_change() {
        let tool = test_tool(
            "grep",
            "Search files.",
            [
                crate::plugin::sdk::ToolTag::ReadOnly,
                crate::plugin::sdk::ToolTag::FilesystemRead,
            ],
        );

        let baseline = build_prepared_prompt(
            &Session::new(1, 1, "tools", Utc::now()),
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: None,
                temperature: None,
                max_output_tokens: None,
                tools: &[],
                provider_request_shape: None,
                continuation_supported: false,
            },
        );
        let with_tool = build_prepared_prompt(
            &Session::new(1, 1, "tools", Utc::now()),
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: None,
                temperature: None,
                max_output_tokens: None,
                tools: &[tool],
                provider_request_shape: None,
                continuation_supported: false,
            },
        );

        assert_ne!(
            baseline.request_options_fingerprint,
            with_tool.request_options_fingerprint
        );
    }

    #[test]
    fn fingerprint_request_options_changes_when_provider_shape_changes() {
        let baseline_shape =
            PromptCacheShape::new("openai").with_string("base_url", "https://api.openai.com/v1");
        let changed_shape =
            PromptCacheShape::new("openai").with_string("base_url", "https://proxy.example/v1");
        let baseline =
            fingerprint_request_options("openai", "gpt-5", None, None, &[], Some(&baseline_shape));
        let changed =
            fingerprint_request_options("openai", "gpt-5", None, None, &[], Some(&changed_shape));

        assert_ne!(baseline, changed);
    }

    #[test]
    fn build_prepared_prompt_does_not_synthesize_hidden_goal_context_message() {
        let session = Session::new(1, 1, "goal", Utc::now())
            .with_messages(vec![Message::prompt_text(Role::User, "hello")]);

        let prepared = build_prepared_prompt(
            &session,
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: Some("system"),
                temperature: Some(0.2),
                max_output_tokens: Some(256),
                tools: &[],
                provider_request_shape: None,
                continuation_supported: false,
            },
        );

        assert_eq!(prepared.messages.len(), 1);
        assert_eq!(prepared.messages[0].as_text_lossy(), "hello");
    }

    #[test]
    fn build_prepared_prompt_reports_provider_shape_diff_on_request_mismatch() {
        let current_shape = PromptCacheShape::new("openai")
            .with_string("base_url", "https://proxy.example/v1")
            .with_string("stream_mode", "sse");
        let previous_shape = PromptCacheShape::new("openai")
            .with_string("base_url", "https://api.openai.com/v1")
            .with_string("stream_mode", "sse");

        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime = SessionRuntimeState {
            turn: Default::default(),
            prompt_window: PromptWindowRuntime {
                generation: 2,
                ..Default::default()
            },
            prompt_tokens: Default::default(),
            provider_anchors: [(
                SessionRuntimeState::provider_anchor_key("openai", "gpt-5"),
                ProviderPromptAnchor {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-5".to_string(),
                    previous_response_id: "resp_123".to_string(),
                    assistant_message_id: 11,
                    prompt_window_generation: 2,
                    system_fingerprint: fingerprint_optional_text(Some("system")),
                    request_options_fingerprint: fingerprint_request_options(
                        "openai",
                        "gpt-5",
                        Some(0.2),
                        Some(256),
                        &[],
                        Some(&previous_shape),
                    ),
                    provider_request_shape: Some(previous_shape),
                    transcript_digest: String::new(),
                },
            )]
            .into_iter()
            .collect(),
            loaded_deferred_tools: Vec::new(),
            execution: Default::default(),
            goal: Default::default(),
            plan: None,
        };

        let prepared = build_prepared_prompt(
            &session,
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: Some("system"),
                temperature: Some(0.2),
                max_output_tokens: Some(256),
                tools: &[],
                provider_request_shape: Some(&current_shape),
                continuation_supported: true,
            },
        );

        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::RequestOptionsFingerprintMismatch
        );
        assert!(prepared.continuation_diagnostic.provider_shape_changed());
        assert_eq!(
            prepared
                .continuation_diagnostic
                .provider_shape_change_keys(),
            vec!["base_url"]
        );
    }

    #[test]
    fn build_prepared_prompt_reports_missing_anchor_when_continuation_cannot_start() {
        let session = Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![
            Message::prompt_text(Role::Assistant, "done"),
            Message::prompt_text(Role::User, "follow up"),
        ]);

        let prepared = build_prepared_prompt(
            &session,
            PromptRequestOptions {
                provider_id: "openai",
                model_id: "gpt-5",
                system: Some("system"),
                temperature: Some(0.2),
                max_output_tokens: Some(256),
                tools: &[],
                provider_request_shape: None,
                continuation_supported: true,
            },
        );

        assert_eq!(prepared.previous_response_id, None);
        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::MissingProviderAnchor
        );
    }

    #[test]
    fn prompt_char_budget_uses_model_limits_and_request_overhead() {
        let tool = test_tool(
            "grep",
            "Search files.",
            [
                crate::plugin::sdk::ToolTag::ReadOnly,
                crate::plugin::sdk::ToolTag::FilesystemRead,
            ],
        );

        let budget = prompt_char_budget(
            Some(4_096),
            Some(512),
            96_000,
            Some("system instructions"),
            &[tool],
        );

        assert!(budget < 4_096 * APPROX_CHARS_PER_TOKEN);
        assert!(budget >= APPROX_CHARS_PER_TOKEN * MIN_PROMPT_BUDGET_TOKENS as usize);
    }
}
