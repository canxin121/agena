use std::collections::HashSet;

use serde::Serialize;
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::{
    message::{
        AttachmentSource, Message, MessageMetadata, MessagePart, OperationPart, PartContent,
    },
    provider::{
        ProjectedSessionPart, project_session_parts, project_session_text_lossy,
        project_session_tool_result_output,
    },
    tool::ToolApiBinding,
};
use agena_domain::{ExecutionStatus, MessageSource, Role, ToolInvocation};
use agena_provider::{PromptCacheShape, PromptCacheShapeDiff, ProviderCompactionContext};

use super::Session;
use super::model::PromptCompactionContent;
use super::store::messages_from_parts;
use super::transcript::{
    ProviderTranscript, TranscriptBlock, TranscriptContent, TranscriptFragment, TranscriptToolCall,
    TranscriptToolOutput,
};
use agena_domain::ToolCallId;

const PROMPT_PROTOCOL_OVERHEAD_CHARS: usize = 2_048;
/// Fixed discriminator for the one current development request shape.
/// Incompatible development state is reset instead of assigning a new value.
const PROMPT_REQUEST_SHAPE_VERSION: u32 = 1;
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
    pub provider_compaction: Option<ProviderCompactionContext>,
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
    pub adapter_id: Option<&'a str>,
    pub model_id: &'a str,
    pub system: Option<&'a str>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub tool_api_functions: &'a [ToolApiBinding],
    pub provider_request_shape: Option<&'a PromptCacheShape>,
    pub continuation_supported: bool,
    pub native_compaction_enabled: bool,
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

impl AsRef<str> for PromptContinuationReason {
    fn as_ref(&self) -> &str {
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

/// Project the session's active model window back onto the v1 [`Message`] form
/// the prompt path consumes (v2 has no in-memory message list). The window is
/// parts-native: [`Session::active_window_parts`] — the parts strictly after
/// the last compaction checkpoint part (`kind == "compaction"`), 13.4. The
/// interim v1 `compacted_through_message_id` filter in
/// `active_prompt_messages_for_model` is now redundant against this window and
/// is left in place until T6 deletes the v1 bridge. A part that fails to
/// decode is logged and dropped from the window rather than taking down
/// prompt assembly; the store is the durable source and already produced this
/// projection once at load, so failures here are degenerate.
fn projected_messages(session: &Session) -> Vec<Message> {
    messages_from_parts(session.active_window_parts()).unwrap_or_else(|error| {
        tracing::warn!(
            session_id = session.id,
            "failed to project parts for prompt window: {error}"
        );
        Vec::new()
    })
}

pub(crate) fn active_prompt_messages(session: &Session) -> Vec<Message> {
    active_prompt_messages_for_model(session, None, None, None, false)
}

pub(crate) fn active_prompt_messages_for_model(
    session: &Session,
    provider_id: Option<&str>,
    adapter_id: Option<&str>,
    model_id: Option<&str>,
    native_compaction_enabled: bool,
) -> Vec<Message> {
    let Some(compaction) = session
        .runtime
        .prompt_window
        .compaction
        .as_ref()
        .filter(|value| !value.is_empty())
    else {
        // Prompt-cache affinity depends on every later request preserving the
        // exact provider-visible prefix from earlier requests. Without an
        // installed compaction snapshot, the prompt path stays append-only.
        return projected_messages(session);
    };

    match &compaction.content {
        PromptCompactionContent::TextSummary {
            summary,
            recent_messages,
        } => {
            let mut messages = Vec::with_capacity(recent_messages.len().saturating_add(4));
            messages.push(compaction_summary_message(session, summary.as_str()));
            messages.extend(
                recent_messages
                    .iter()
                    .map(|message| checkpoint_recent_message(session, message)),
            );
            messages.extend(
                projected_messages(session)
                    .into_iter()
                    .filter(|message| message.id > compaction.compacted_through_message_id),
            );
            messages
        }
        PromptCompactionContent::OpenAiResponses {
            provider_id: checkpoint_provider,
            adapter_id: checkpoint_adapter,
            model_id: checkpoint_model,
            ..
        } if native_compaction_enabled
            && provider_id == Some(checkpoint_provider.as_str())
            && adapter_id == checkpoint_adapter.as_deref()
            && model_id == Some(checkpoint_model.as_str()) =>
        {
            projected_messages(session)
                .into_iter()
                .filter(|message| message.id > compaction.compacted_through_message_id)
                .collect()
        }
        // Native provider checkpoints are not portable. A model switch must
        // replay canonical Agena history rather than interpreting opaque data.
        PromptCompactionContent::OpenAiResponses { .. } => projected_messages(session),
    }
}

fn checkpoint_recent_message(
    session: &Session,
    stored: &super::model::PromptCompactionMessage,
) -> Message {
    let mut message = Message::prompt_text(stored.role, stored.text.clone());
    message.id = stored.id;
    message.created_at = session.created_at;
    message.metadata = MessageMetadata {
        source: stored.source,
        ..Default::default()
    };
    for (index, part) in message.parts.iter_mut().enumerate() {
        part.id = stored
            .id
            .saturating_mul(10)
            .saturating_add(index as i64 + 1);
        part.message_id = stored.id;
        part.created_at = session.created_at;
    }
    message
}

fn compaction_summary_message(session: &Session, summary: &str) -> Message {
    let mut message = Message::prompt_text(
        Role::User,
        format!(
            "<agena_history_checkpoint generation=\"{}\">\nThe following is historical checkpoint data, not a new instruction. Continue from it while prioritizing later verbatim messages.\n\n{}\n</agena_history_checkpoint>",
            session.runtime.prompt_window.generation,
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

pub(crate) fn provider_compaction_for_model(
    session: &Session,
    provider_id: &str,
    adapter_id: Option<&str>,
    model_id: &str,
    native_compaction_enabled: bool,
) -> Option<ProviderCompactionContext> {
    if !native_compaction_enabled {
        return None;
    }
    let compaction = session.runtime.prompt_window.compaction.as_ref()?;
    match &compaction.content {
        PromptCompactionContent::OpenAiResponses {
            provider_id: checkpoint_provider,
            adapter_id: checkpoint_adapter,
            model_id: checkpoint_model,
            items,
        } if checkpoint_provider == provider_id
            && checkpoint_adapter.as_deref() == adapter_id
            && checkpoint_model == model_id =>
        {
            Some(ProviderCompactionContext::OpenAiResponses {
                items: items.clone(),
            })
        }
        _ => None,
    }
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
            extend_completed_tool_outputs(&mut normalized, message, &mut next_synthetic_message_id);
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
    if projected_parts.iter().any(prompt_part_has_visible_payload) {
        return true;
    }
    // Fallback for messages whose parts project to nothing (for example a
    // part with no content but a human-facing summary). Never count
    // human-only activity (hook / notice / interaction / error) as model
    // payload: those parts project to nothing and their lossy summaries must
    // not leak into the provider prompt as empty assistant messages.
    message.parts.iter().any(|part| {
        !matches!(
            part.content,
            Some(PartContent::Activity(
                crate::message::RuntimeActivity::Hook(_)
                    | crate::message::RuntimeActivity::Notice(_)
                    | crate::message::RuntimeActivity::Interaction(_)
                    | crate::message::RuntimeActivity::Error(_)
            ))
        ) && !part.summary.as_deref().unwrap_or("").trim().is_empty()
    })
}

fn prompt_part_has_visible_payload(part: &crate::provider::ProjectedSessionPart) -> bool {
    match part {
        crate::provider::ProjectedSessionPart::Text { text } => !text.trim().is_empty(),
        crate::provider::ProjectedSessionPart::Reasoning { text } => !text.trim().is_empty(),
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

pub(crate) fn prompt_request_fingerprints(
    options: &PromptRequestOptions<'_>,
) -> PromptRequestFingerprint {
    PromptRequestFingerprint {
        system_fingerprint: fingerprint_optional_text(options.system),
        request_options_fingerprint: fingerprint_request_options(
            options.provider_id,
            options.adapter_id,
            options.model_id,
            options.temperature,
            options.max_output_tokens,
            options.tool_api_functions,
            options.provider_request_shape,
            options.native_compaction_enabled,
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
    // The provider's previous response includes the request prefix, but not the
    // assistant output itself. Include the anchor response plus later deltas.
    let delta_chars = approximate_prompt_payload_chars(&prompt_messages[anchor_index..]);
    let delta_tokens = agena_runtime::estimate_prompt_tokens_from_chars(delta_chars);

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
                let mut content_blocks = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();
                let mut had_any = false;
                for part in parts {
                    match part {
                        ProjectedSessionPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Text { text });
                            }
                        }
                        ProjectedSessionPart::Reasoning { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Reasoning { text });
                            }
                        }
                        ProjectedSessionPart::Attachment { item } => {
                            had_any = true;
                            content_blocks.push(attachment_to_transcript_block(&item));
                        }
                        ProjectedSessionPart::ToolCall {
                            id,
                            function,
                            arguments_json,
                        } => {
                            had_any = true;
                            tool_calls.push(TranscriptToolCall {
                                call_id: ToolCallId::from(SmolStr::from(id)),
                                name: SmolStr::from(function.function_name()),
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
                        content_blocks.push(TranscriptBlock::Text { text: fallback });
                        had_any = true;
                    }
                }
                if had_any {
                    transcript.push(TranscriptFragment::Assistant {
                        content: TranscriptContent::from_blocks(content_blocks),
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
                let mut content_blocks = Vec::new();
                let mut had_any = false;
                for part in parts {
                    match part {
                        ProjectedSessionPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Text { text });
                            }
                        }
                        ProjectedSessionPart::Attachment { item } => {
                            had_any = true;
                            content_blocks.push(attachment_to_transcript_block(&item));
                        }
                        // ToolCall / ToolResult are not produced under user/system roles
                        // by `project_session_parts`; ignore for digest stability.
                        _ => {}
                    }
                }
                if !had_any {
                    let fallback = message.as_text_lossy();
                    if !fallback.trim().is_empty() {
                        content_blocks.push(TranscriptBlock::Text { text: fallback });
                        had_any = true;
                    }
                }
                if !had_any {
                    continue;
                }
                let fragment = if matches!(message.role, Role::System) {
                    TranscriptFragment::System {
                        text: content_blocks
                            .iter()
                            .filter_map(|block| match block {
                                TranscriptBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(""),
                    }
                } else {
                    TranscriptFragment::User {
                        content: TranscriptContent::from_blocks(content_blocks),
                    }
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
            item.kind,
            item.mime.trim(),
            item.summary_label(),
            source_marker
        )),
        media_type: Some(SmolStr::from(item.mime.trim())),
    }
}

pub(crate) fn approximate_request_overhead_chars(
    system: Option<&str>,
    tools: &[ToolApiBinding],
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
    tools: &[ToolApiBinding],
) -> u64 {
    let total_chars = approximate_prompt_payload_chars(messages)
        .saturating_add(approximate_request_overhead_chars(system, tools));
    agena_runtime::estimate_prompt_tokens_from_chars(total_chars)
}

pub(crate) fn approximate_total_request_tokens_with_compaction(
    messages: &[Message],
    system: Option<&str>,
    tools: &[ToolApiBinding],
    provider_compaction: Option<&ProviderCompactionContext>,
) -> u64 {
    let native_chars = provider_compaction
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    let total_chars = approximate_prompt_payload_chars(messages)
        .saturating_add(approximate_request_overhead_chars(system, tools))
        .saturating_add(native_chars);
    agena_runtime::estimate_prompt_tokens_from_chars(total_chars)
}

pub(crate) fn prompt_char_budget(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    fallback_max_prompt_chars: usize,
    system: Option<&str>,
    tools: &[ToolApiBinding],
) -> usize {
    let overhead_chars = approximate_request_overhead_chars(system, tools);
    let fallback_budget = fallback_max_prompt_chars
        .saturating_sub(overhead_chars)
        .max(
            agena_runtime::APPROX_CHARS_PER_TOKEN
                * agena_runtime::MIN_PROMPT_BUDGET_TOKENS as usize,
        );

    let Some(prompt_tokens) = agena_runtime::prompt_token_budget(
        context_window_tokens,
        max_input_tokens,
        max_output_tokens,
    ) else {
        return fallback_budget;
    };
    let prompt_chars = prompt_tokens as usize * agena_runtime::APPROX_CHARS_PER_TOKEN;
    prompt_chars.saturating_sub(overhead_chars).max(
        agena_runtime::APPROX_CHARS_PER_TOKEN * agena_runtime::MIN_PROMPT_BUDGET_TOKENS as usize,
    )
}

pub(crate) fn build_prepared_prompt(
    session: &Session,
    options: PromptRequestOptions<'_>,
) -> PreparedPrompt {
    let active_messages = active_prompt_messages_for_model(
        session,
        Some(options.provider_id),
        options.adapter_id,
        Some(options.model_id),
        options.native_compaction_enabled,
    );
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
    let (messages, previous_response_id, provider_compaction) = match continuation {
        PromptContinuationOutcome::Reuse {
            previous_response_id,
            delta_messages,
        } => (delta_messages, Some(previous_response_id), None),
        PromptContinuationOutcome::Restart { .. } => (
            prompt_messages,
            None,
            provider_compaction_for_model(
                session,
                options.provider_id,
                options.adapter_id,
                options.model_id,
                options.native_compaction_enabled,
            ),
        ),
    };

    PreparedPrompt {
        system: options.system.map(ToOwned::to_owned),
        messages,
        prompt_cache_key: prompt_cache_key_for_session(session),
        previous_response_id,
        provider_compaction,
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
    fingerprint_value(&(
        session.workspace_id,
        session.id,
        session.created_at.timestamp_millis(),
    ))
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

/// Project a bounded transcript for permission classification, anchoring the
/// first message and the most recent messages, truncating to `budget_chars`.
pub(crate) fn project_transcript(messages: &[Message], budget_chars: usize) -> String {
    let projected = messages
        .iter()
        .map(project_session_text_lossy)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    if projected.is_empty() {
        return String::new();
    }
    let total: usize = projected.iter().map(String::len).sum();
    if total <= budget_chars {
        return projected.join(
            "
",
        );
    }
    if projected.len() == 1 {
        return truncate_chars(&projected[0], budget_chars);
    }
    let head_budget = (budget_chars / 4).max(1);
    let tail_budget = budget_chars.saturating_sub(head_budget);
    let mut parts = vec![truncate_chars(&projected[0], head_budget)];
    let mut tail = Vec::new();
    let mut used = 0usize;
    for text in projected.iter().rev().take(projected.len() - 1) {
        if used + text.len() <= tail_budget {
            tail.push(text.clone());
            used += text.len();
        } else if tail.is_empty() {
            tail.push(truncate_chars(text, tail_budget));
            break;
        } else {
            break;
        }
    }
    tail.reverse();
    parts.extend(tail);
    parts.push("[transcript truncated to fit the approval context window]".to_owned());
    parts.join(
        "
",
    )
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_owned();
    }
    text.chars().take(max_chars).collect()
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
            let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(exec))) =
                part.content.as_ref()
            else {
                return 0;
            };
            if exec.is_provider_only() {
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
            Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(exec)))
                if !exec.is_provider_only() =>
            {
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
        let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(exec))) =
            part.content.as_ref()
        else {
            continue;
        };
        if exec.is_provider_only() {
            continue;
        }
        if matches!(
            part.status,
            ExecutionStatus::Completed
                | ExecutionStatus::PolicyDenied
                | ExecutionStatus::UserDeclined
                | ExecutionStatus::CapabilityUnavailable
                | ExecutionStatus::ToolUnavailable
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
        ) {
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
        if !matches!(
            part.status,
            ExecutionStatus::Completed
                | ExecutionStatus::PolicyDenied
                | ExecutionStatus::UserDeclined
                | ExecutionStatus::CapabilityUnavailable
                | ExecutionStatus::ToolUnavailable
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
        ) {
            continue;
        }
        let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(exec))) =
            part.content.as_ref()
        else {
            continue;
        };
        if exec.is_provider_only() {
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
    let details = serde_json::from_str::<serde_json::Value>(output_text.as_str())
        .ok()
        .and_then(|value| agena_domain::ToolOutput::from_json_payload(Some(&value)).ok())
        .unwrap_or_default();
    let mut message = Message::prompt_parts(
        Role::Tool,
        vec![PartContent::operation(OperationPart::completed(
            call_id,
            invocation,
            crate::message::OperationCompletion::new(
                "Provider tool result",
                "Result available",
                output_text,
                Vec::new(),
                Vec::new(),
                details,
            ),
            agena_domain::TimeRange::default(),
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
            let output_text = project_session_tool_result_output(part.status, exec);
            if output_text.trim().is_empty() {
                SYNTHETIC_TOOL_COMPLETED_PLACEHOLDER.to_string()
            } else {
                output_text
            }
        }
        ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => {
            let output_text = project_session_tool_result_output(part.status, exec);
            if output_text.trim().is_empty() {
                match part.status {
                    ExecutionStatus::PolicyDenied => {
                        "The operation was not executed because it is blocked by the effective permission policy."
                            .to_string()
                    }
                    ExecutionStatus::UserDeclined => {
                        "The operation was not executed because the user declined the permission request."
                            .to_string()
                    }
                    ExecutionStatus::CapabilityUnavailable => {
                        "The operation was not executed because the current runtime does not provide the required capability."
                            .to_string()
                    }
                    ExecutionStatus::ToolUnavailable => {
                        "The operation was not executed because the requested tool is unavailable."
                            .to_string()
                    }
                    _ => unreachable!("matched non-execution status"),
                }
            } else {
                output_text
            }
        }
        ExecutionStatus::Failed => {
            let output_text = project_session_tool_result_output(part.status, exec);
            if !output_text.trim().is_empty() {
                output_text
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
        ExecutionStatus::Failed
        | ExecutionStatus::Completed
        | ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => project_session_tool_result_output(part.status, exec),
        ExecutionStatus::Cancelled => fallback_tool_result_output(part, exec),
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
    adapter_id: Option<&str>,
    model_id: &str,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
    tools: &[ToolApiBinding],
    provider_request_shape: Option<&PromptCacheShape>,
    native_compaction_enabled: bool,
) -> String {
    #[derive(Serialize)]
    struct RequestOptionsFingerprint<'a> {
        prompt_request_shape_version: u32,
        provider_id: &'a str,
        adapter_id: Option<&'a str>,
        model_id: &'a str,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        tools: &'a [ToolApiBinding],
        provider_request_shape_fingerprint: Option<String>,
        native_compaction_enabled: bool,
    }

    let provider_request_shape_fingerprint =
        provider_request_shape.map(PromptCacheShape::fingerprint);

    fingerprint_value(&RequestOptionsFingerprint {
        prompt_request_shape_version: PROMPT_REQUEST_SHAPE_VERSION,
        provider_id,
        adapter_id,
        model_id,
        temperature,
        max_output_tokens,
        tools,
        provider_request_shape_fingerprint,
        native_compaction_enabled,
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
mod compaction_tests {
    use super::*;
    use crate::session::model::{
        PromptCompactionContent, PromptCompactionMessage, PromptCompactionRuntime,
    };
    use crate::session::store::{
        messages_from_parts, part_content_to_value, part_role_from_role, run_marker_content,
    };
    use agena_domain::MessageSource;
    use agena_domain::{PromptCompactionStrategy, PromptCompactionTrigger};
    use agena_provider::CompletionUsage;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
    use chrono::{DateTime, Utc};

    /// Build one run marker part (`part_id` = the durable message id) plus its
    /// text content part, the v2 parts shape the prompt path projects back
    /// onto [`Message`] via `messages_from_parts`.
    fn run_parts(id: i64, role: Role, source: &str, text: &str, now: DateTime<Utc>) -> Vec<Part> {
        let mut content = run_marker_content("user_send", None, None, None, None);
        content["source"] = serde_json::json!(source);
        let part_role = part_role_from_role(role);
        vec![
            Part {
                part_id: id,
                kind: "run".to_owned(),
                role: part_role,
                state: PartState::Completed,
                content,
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                run_id: None,
                origin_session_id: 7,
                revision: 0,
                started_at_ms: now.timestamp_millis(),
                finished_at_ms: None,
                created_at_ms: now.timestamp_millis(),
                updated_at_ms: now.timestamp_millis(),
                provider_state: None,
            },
            Part {
                part_id: id * 1000,
                kind: "text".to_owned(),
                role: part_role,
                state: PartState::Completed,
                content: part_content_to_value(&PartContent::text(text.to_owned()))
                    .expect("text content is always serializable"),
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                run_id: Some(id),
                origin_session_id: 7,
                revision: 0,
                started_at_ms: now.timestamp_millis(),
                finished_at_ms: None,
                created_at_ms: now.timestamp_millis(),
                updated_at_ms: now.timestamp_millis(),
                provider_state: None,
            },
        ]
    }

    /// Build a run marker part plus a `hook` content part (human-only activity
    /// that must never reach the model prompt).
    fn hook_run_parts(id: i64, now: DateTime<Utc>) -> Vec<Part> {
        let mut content = run_marker_content("hook", None, None, None, None);
        content["source"] = serde_json::json!("system");
        vec![
            Part {
                part_id: id,
                kind: "run".to_owned(),
                role: PartRole::Assistant,
                state: PartState::Completed,
                content,
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                run_id: None,
                origin_session_id: 7,
                revision: 0,
                started_at_ms: now.timestamp_millis(),
                finished_at_ms: None,
                created_at_ms: now.timestamp_millis(),
                updated_at_ms: now.timestamp_millis(),
                provider_state: None,
            },
            Part {
                part_id: id * 1000,
                kind: "hook".to_owned(),
                role: PartRole::Assistant,
                state: PartState::Completed,
                content: part_content_to_value(&PartContent::hook(crate::message::HookPart {
                    hook: "agent.stop".to_owned(),
                    plugin_id: Some("agena.plan".to_owned()),
                    summary: "agent.stop hook blocked stop: workflow plan autorun".to_owned(),
                    detail: Some("continue with the next plan step".to_owned()),
                }))
                .expect("hook content is always serializable"),
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                run_id: Some(id),
                origin_session_id: 7,
                revision: 0,
                started_at_ms: now.timestamp_millis(),
                finished_at_ms: None,
                created_at_ms: now.timestamp_millis(),
                updated_at_ms: now.timestamp_millis(),
                provider_state: None,
            },
        ]
    }

    fn session_with_messages() -> Session {
        let now = Utc::now();
        let mut session = Session::new(7, 11, "test", now);
        let mut parts = run_parts(1, Role::User, "user", "old user", now);
        parts.extend(run_parts(2, Role::Assistant, "tool", "old assistant", now));
        parts.extend(run_parts(3, Role::User, "user", "future user", now));
        session.install_projected_parts(parts);
        session
    }

    /// Project the session's parts back onto v1 messages (the same bridge the
    /// prompt path uses internally).
    fn projected_messages(session: &Session) -> Vec<Message> {
        messages_from_parts(session.parts()).expect("test parts project cleanly")
    }

    #[test]
    fn text_checkpoint_uses_summary_recent_suffix_and_future_only() {
        let mut session = session_with_messages();
        session.runtime.prompt_window.generation = 1;
        session.runtime.prompt_window.compaction = Some(PromptCompactionRuntime {
            checkpoint_id: "checkpoint".to_owned(),
            compacted_through_message_id: 2,
            trigger: PromptCompactionTrigger::Auto,
            strategy: PromptCompactionStrategy::LocalSummary,
            content: PromptCompactionContent::TextSummary {
                summary: "durable state".to_owned(),
                recent_messages: vec![PromptCompactionMessage {
                    id: 2,
                    role: Role::Assistant,
                    source: MessageSource::Assistant,
                    text: "retained assistant".to_owned(),
                }],
            },
            before_tokens: 100,
            after_tokens: 20,
            created_at_ms: 1,
        });

        let active = active_prompt_messages_for_model(&session, Some("p"), None, Some("m"), false);
        assert_eq!(active.len(), 3);
        assert!(active[0].as_text_lossy().contains("durable state"));
        assert_eq!(active[1].as_text_lossy(), "retained assistant");
        assert_eq!(active[2].as_text_lossy(), "future user");
        assert!(
            active
                .iter()
                .all(|message| message.as_text_lossy() != "old user")
        );
    }

    #[test]
    fn hook_only_activity_is_filtered_from_the_model_prompt() {
        let now = Utc::now();
        let mut session = session_with_messages();

        // A recorded hook run (human-only activity) must never reach the
        // model: it projects to no provider payload and its human-facing
        // summary is not provider payload.
        let mut parts = session.parts().to_vec();
        parts.extend(hook_run_parts(99, now));

        // A real user message following the hook-only activity is unaffected.
        parts.extend(run_parts(100, Role::User, "user", "future user", now));
        session.install_projected_parts(parts);

        let prompt_messages = prompt_messages_for_request(&projected_messages(&session));
        assert!(
            prompt_messages.iter().all(|m| m.id != 99),
            "hook-only assistant message must not be sent to the model; got {:?}",
            prompt_messages
                .iter()
                .map(|m| (m.id, m.as_text_lossy()))
                .collect::<Vec<_>>()
        );
        // The three base messages plus the trailing user message remain.
        assert_eq!(prompt_messages.len(), 4);
        assert!(
            !prompt_messages
                .iter()
                .any(|m| m.as_text_lossy().contains("agent.stop hook")),
            "hook summaries must not leak into the model prompt"
        );
    }

    #[test]
    fn native_checkpoint_is_opaque_and_model_scoped() {
        let mut session = session_with_messages();
        session.runtime.prompt_window.compaction = Some(PromptCompactionRuntime {
            checkpoint_id: "native".to_owned(),
            compacted_through_message_id: 2,
            trigger: PromptCompactionTrigger::Manual,
            strategy: PromptCompactionStrategy::OpenAiResponses,
            content: PromptCompactionContent::OpenAiResponses {
                provider_id: "openai".to_owned(),
                adapter_id: Some("responses".to_owned()),
                model_id: "gpt".to_owned(),
                items: vec![
                    serde_json::json!({"type": "compaction", "encrypted_content": "opaque"}),
                ],
            },
            before_tokens: 100,
            after_tokens: 10,
            created_at_ms: 1,
        });

        let matching = active_prompt_messages_for_model(
            &session,
            Some("openai"),
            Some("responses"),
            Some("gpt"),
            true,
        );
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].id, 3);
        assert!(
            provider_compaction_for_model(&session, "openai", Some("responses"), "gpt", true)
                .is_some()
        );

        let locally_compacted = active_prompt_messages_for_model(
            &session,
            Some("openai"),
            Some("responses"),
            Some("gpt"),
            false,
        );
        // Segment ids are ephemeral v1 identities the bridge re-mints on
        // each projection (v2 parts are the durable source; there is no
        // retained v1 message list, and the provider transcript excludes
        // them). Compare the stable projection surface — message id, role
        // and text — not the freshly-randomized segment ids.
        let projected = projected_messages(&session);
        assert_eq!(locally_compacted.len(), projected.len());
        for (actual, expected) in locally_compacted.iter().zip(projected.iter()) {
            assert_eq!(
                (actual.id, actual.role, actual.as_text_lossy()),
                (expected.id, expected.role, expected.as_text_lossy()),
            );
        }
        assert!(
            provider_compaction_for_model(&session, "openai", Some("responses"), "gpt", false)
                .is_none()
        );

        let wrong_adapter = active_prompt_messages_for_model(
            &session,
            Some("openai"),
            Some("chat"),
            Some("gpt"),
            true,
        );
        assert_eq!(wrong_adapter.len(), 3);
        assert!(
            provider_compaction_for_model(&session, "openai", Some("chat"), "gpt", true).is_none()
        );

        let switched = active_prompt_messages_for_model(
            &session,
            Some("anthropic"),
            None,
            Some("claude"),
            true,
        );
        assert_eq!(switched.len(), 3);
        assert!(
            provider_compaction_for_model(&session, "anthropic", None, "claude", true).is_none()
        );
    }

    #[test]
    fn native_compaction_policy_is_part_of_the_request_fingerprint() {
        let enabled = PromptRequestOptions {
            provider_id: "openai",
            adapter_id: Some("responses"),
            model_id: "gpt",
            system: None,
            temperature: None,
            max_output_tokens: None,
            tool_api_functions: &[],
            provider_request_shape: None,
            continuation_supported: true,
            native_compaction_enabled: true,
        };
        let mut disabled = enabled.clone();
        disabled.native_compaction_enabled = false;

        assert_ne!(
            prompt_request_fingerprints(&enabled).request_options_fingerprint,
            prompt_request_fingerprints(&disabled).request_options_fingerprint,
        );
    }

    #[test]
    fn runtime_projection_counts_anchor_assistant_output() {
        let mut session = session_with_messages();
        let messages = projected_messages(&session);
        let digest = prompt_transcript_digest(&messages[..2]);
        session.runtime.record_prompt_tokens(
            2,
            &CompletionUsage {
                input_tokens: 100,
                ..Default::default()
            },
            0,
            Some(10_000),
            "system".to_owned(),
            "options".to_owned(),
            digest,
        );
        let estimate =
            estimate_prompt_tokens_from_runtime(&session, messages.as_slice(), "system", "options")
                .expect("matching runtime estimate");
        assert!(estimate.delta_chars >= "old assistant".len() as u64);
        assert!(estimate.total_tokens > 100);
    }
    #[test]
    fn interstitial_text_segment_is_still_sent_to_the_model() {
        use crate::message::PartContent;
        use crate::session::transcript::TranscriptBlock;

        let mut message = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::text("second paragraph after a tool call")],
        );
        message.id = 2;
        // Simulate the processor marking this part as a text-segment Activity:
        // ActivityId set, text segment id cleared. The model must still see
        // the plain text; the Activity is only a transcript presentation.
        if let Some(part) = message.parts.first_mut() {
            part.activity_id = Some(agena_domain::ActivityId::new());
            part.segment_id = None;
        }

        let content = TranscriptContent::from_message_lossy(&message);
        assert_eq!(content.blocks.len(), 1);
        assert!(
            matches!(
                &content.blocks[0],
                TranscriptBlock::Text { text } if text == "second paragraph after a tool call"
            ),
            "interstitial text must be part of the provider transcript"
        );
    }
}
