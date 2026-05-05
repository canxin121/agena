use std::{
    collections::{BTreeSet, HashSet},
    sync::LazyLock,
};

use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::{
    message::{
        AttachmentSource, Message, MessagePart, MessageSource, PartContent, ToolExecutionPart,
        ToolInvocation,
    },
    provider::{
        PRUNED_TOOL_RESULT_PLACEHOLDER, ProjectedSessionPart, PromptCacheShape,
        PromptCacheShapeDiff, project_session_parts, project_session_text_lossy,
    },
    role::Role,
    tool::EntryDefinition,
};

use super::Session;
use super::history::{
    ProviderTranscript, TranscriptBlock, TranscriptContent, TranscriptFragment, TranscriptToolCall,
    TranscriptToolOutput,
};
use super::ids::ToolCallId;
use super::model::{
    MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_PROMPT_COMPACTED,
    MESSAGE_TAG_PROMPT_SUMMARY, MESSAGE_TAG_TOOL_RESULT_PRUNED,
};

const ATTACHMENT_PAYLOAD_STRIP_MIN_BYTES: usize = 256_000;
const ATTACHMENT_PAYLOAD_STRIP_PROTECT_BYTES: usize = 512_000;
const ATTACHMENT_PAYLOAD_STRIP_PROTECTED_USER_TURNS: usize = 2;
const APPROX_CHARS_PER_TOKEN: usize = 4;
const COMPACTION_SUMMARY_BUDGET_CHARS: usize = 4_000;
const MIN_PROMPT_BUDGET_TOKENS: u32 = 512;
const MIN_CONTEXT_RESERVE_TOKENS: u32 = 1_024;
const MAX_CONTEXT_RESERVE_TOKENS: u32 = 20_000;
const PROMPT_PROTOCOL_OVERHEAD_CHARS: usize = 2_048;
const PROMPT_REQUEST_SHAPE_VERSION: u32 = 3;
const SYNTHETIC_TOOL_COMPLETED_PLACEHOLDER: &str =
    "[Tool execution completed without persisted output]";
const SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER: &str = "[Tool execution was interrupted]";
const SYNTHETIC_TOOL_FAILED_PLACEHOLDER: &str = "[Tool execution failed without persisted output]";
const TOOL_RESULT_PRUNE_MIN_CHARS: usize = 12_000;
const TOOL_RESULT_PRUNE_PROTECT_CHARS: usize = 24_000;
const TOOL_RESULT_PROTECTED_USER_TURNS: usize = 2;
const COMPACTION_SUMMARY_HEADING: &str = "Conversation summary (compacted):";
const COMPACTION_SUMMARY_SECTIONS: [&str; 5] = [
    "Goal",
    "Instructions",
    "Discoveries",
    "Accomplished",
    "Relevant files / directories",
];

static FILE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
        (?:
            [A-Za-z]:[\\/][^\s"'`]+
            |
            (?:\.{1,2}[\\/])?[A-Za-z0-9._-]+(?:[\\/][A-Za-z0-9._-]+)+(?:\.[A-Za-z0-9._-]+)?
            |
            [A-Za-z0-9._-]+\.(?:rs|toml|json|md|txt|yaml|yml|ts|tsx|js|jsx|py|go|java|sql|html|css)
        )
        "#,
    )
    .expect("file path regex should compile")
});

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
    pub tools: &'a [EntryDefinition],
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
    ReusedPreviousResponseId,
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
            Self::ReusedPreviousResponseId => "reused_previous_response_id",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptCompactionPlan {
    pub compacted_message_ids: Vec<i64>,
    pub summary_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolResultPrunePlan {
    pub pruned_message_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentPayloadStripPlan {
    pub stripped_message_ids: Vec<i64>,
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
    output_text: String,
}

pub(crate) fn active_prompt_messages(session: &Session) -> Vec<Message> {
    let active = session
        .messages
        .iter()
        .filter(|message| !message.metadata.has_tag(MESSAGE_TAG_PROMPT_COMPACTED))
        .cloned()
        .collect::<Vec<_>>();

    let Some(latest_summary_id) = active
        .iter()
        .rev()
        .find(|message| message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY))
        .map(|message| message.id)
    else {
        return active;
    };

    let mut prompt_messages = Vec::with_capacity(active.len());
    if let Some(summary) = active
        .iter()
        .find(|message| message.id == latest_summary_id)
    {
        prompt_messages.push(summary.clone());
    }
    prompt_messages.extend(active.into_iter().filter(|message| {
        message.id != latest_summary_id && !message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY)
    }));
    prompt_messages
}

pub(crate) fn can_compact(
    messages: &[Message],
    keep_tail_messages: usize,
    max_prompt_chars: usize,
) -> bool {
    if messages.len() <= 1 {
        return false;
    }

    tail_messages_to_keep(messages, keep_tail_messages, max_prompt_chars) < messages.len()
}

pub(crate) fn plan_compaction(
    messages: &[Message],
    keep_tail_messages: usize,
    max_prompt_chars: usize,
) -> Option<PromptCompactionPlan> {
    if messages.is_empty() || !can_compact(messages, keep_tail_messages, max_prompt_chars) {
        return None;
    }

    let keep_tail = tail_messages_to_keep(messages, keep_tail_messages, max_prompt_chars);
    let split = messages.len().saturating_sub(keep_tail);
    if split == 0 {
        return None;
    }

    let head = &messages[..split];
    let summary_text = build_compaction_summary(head);

    Some(PromptCompactionPlan {
        compacted_message_ids: head.iter().map(|message| message.id).collect(),
        summary_text,
    })
}

pub(crate) fn plan_tool_result_pruning(messages: &[Message]) -> Option<ToolResultPrunePlan> {
    let mut user_turns = 0_usize;
    let mut protected_chars = 0_usize;
    let mut prunable_chars = 0_usize;
    let mut pruned_message_ids = Vec::new();

    for message in messages.iter().rev() {
        if message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY) {
            break;
        }

        if message.role == Role::User {
            user_turns += 1;
        }
        if user_turns < TOOL_RESULT_PROTECTED_USER_TURNS {
            continue;
        }

        if message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
            continue;
        }

        let output_chars = tool_result_output_chars(message);
        if output_chars == 0 {
            continue;
        }

        protected_chars += output_chars;
        if protected_chars > TOOL_RESULT_PRUNE_PROTECT_CHARS {
            prunable_chars += output_chars;
            pruned_message_ids.push(message.id);
        }
    }

    (prunable_chars >= TOOL_RESULT_PRUNE_MIN_CHARS && !pruned_message_ids.is_empty())
        .then_some(ToolResultPrunePlan { pruned_message_ids })
}

pub(crate) fn plan_attachment_payload_stripping(
    messages: &[Message],
) -> Option<AttachmentPayloadStripPlan> {
    let mut user_turns = 0_usize;
    let mut protected_bytes = 0_usize;
    let mut strippable_bytes = 0_usize;
    let mut stripped_message_ids = Vec::new();

    for message in messages.iter().rev() {
        if message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY) {
            break;
        }

        if message.role == Role::User {
            user_turns += 1;
        }
        if user_turns < ATTACHMENT_PAYLOAD_STRIP_PROTECTED_USER_TURNS {
            continue;
        }

        if message
            .metadata
            .has_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED)
            || message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED)
        {
            continue;
        }

        let payload_bytes = attachment_payload_bytes(message);
        if payload_bytes == 0 {
            continue;
        }

        protected_bytes += payload_bytes;
        if protected_bytes > ATTACHMENT_PAYLOAD_STRIP_PROTECT_BYTES {
            strippable_bytes += payload_bytes;
            stripped_message_ids.push(message.id);
        }
    }

    (strippable_bytes >= ATTACHMENT_PAYLOAD_STRIP_MIN_BYTES && !stripped_message_ids.is_empty())
        .then_some(AttachmentPayloadStripPlan {
            stripped_message_ids,
        })
}

pub(crate) fn normalize_prompt_messages(messages: &[Message]) -> Vec<Message> {
    let mut normalized = Vec::with_capacity(messages.len());
    let mut pending_tool_outputs = Vec::<PendingToolCallOutput>::new();
    let mut next_synthetic_message_id = -1_i64;

    for message in messages {
        match message.role {
            Role::Tool => {
                let tool_call_ids = tool_result_ids(message);
                if !tool_call_ids.is_empty() {
                    let matched_tool_call_ids = tool_call_ids
                        .into_iter()
                        .filter(|tool_call_id| {
                            pending_tool_outputs
                                .iter()
                                .any(|pending| pending.tool_call_id == *tool_call_id)
                        })
                        .collect::<HashSet<_>>();
                    if !matched_tool_call_ids.is_empty() {
                        pending_tool_outputs.retain(|pending| {
                            !matched_tool_call_ids.contains(pending.tool_call_id.as_str())
                        });
                        append_normalized_prompt_tool_messages(
                            &mut normalized,
                            message,
                            &matched_tool_call_ids,
                            &mut next_synthetic_message_id,
                        );
                    }
                } else {
                    flush_synthetic_tool_results(
                        &mut normalized,
                        &mut pending_tool_outputs,
                        &mut next_synthetic_message_id,
                    );
                    normalized.push(message.clone());
                }
            }
            _ => {
                flush_synthetic_tool_results(
                    &mut normalized,
                    &mut pending_tool_outputs,
                    &mut next_synthetic_message_id,
                );
                normalized.push(message.clone());
                if message.role == Role::Assistant {
                    extend_pending_tool_outputs(&mut pending_tool_outputs, message);
                }
            }
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

fn append_normalized_prompt_tool_messages(
    normalized: &mut Vec<Message>,
    message: &Message,
    matched_tool_call_ids: &HashSet<String>,
    next_synthetic_message_id: &mut i64,
) {
    if matched_tool_call_ids.is_empty() {
        normalized.push(message.clone());
        return;
    }

    let projected_parts = project_session_parts(message);
    let has_visible_payload = projected_parts.iter().any(prompt_part_has_visible_payload);
    if has_visible_payload {
        normalized.push(message.clone());
        return;
    }

    let mut synthetic_results = Vec::new();
    let mut seen = HashSet::new();
    for part in &message.parts {
        let Some(PartContent::ToolExecution(exec)) = part.content.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = tool_execution_call_id(part, exec) else {
            continue;
        };
        if !matched_tool_call_ids.contains(tool_call_id.as_str())
            || !seen.insert(tool_call_id.clone())
        {
            continue;
        }

        let output_text = fallback_tool_result_output(exec);
        if output_text.trim().is_empty() {
            continue;
        }
        synthetic_results.push((tool_call_id, output_text));
    }

    if synthetic_results.is_empty() {
        normalized.push(message.clone());
        return;
    }

    for (index, (tool_call_id, output_text)) in synthetic_results.into_iter().enumerate() {
        let message_id = if index == 0 {
            message.id
        } else {
            let synthetic_message_id = *next_synthetic_message_id;
            *next_synthetic_message_id -= 1;
            synthetic_message_id
        };
        let mut synthetic = synthetic_tool_result_message(message_id, tool_call_id, output_text);
        synthetic.created_at = message.created_at;
        normalized.push(synthetic);
    }
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

    let last_successful_total_tokens = runtime.total_tokens()?;
    let assistant_message_id = runtime.last_successful_assistant_message_id?;
    let prompt_messages = normalize_prompt_messages(messages);
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
        total_tokens: last_successful_total_tokens.saturating_add(delta_tokens),
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
            Role::Tool => {
                for part in parts {
                    if let ProjectedSessionPart::ToolResult {
                        tool_call_id,
                        output_json,
                    } = part
                    {
                        if output_json.is_empty() {
                            // Empty tool results are equivalent to "no result yet" —
                            // keep digest stable across the placeholder/synthesized
                            // states (see existing tests covering this behavior).
                            continue;
                        }
                        let output = if output_json == PRUNED_TOOL_RESULT_PLACEHOLDER {
                            TranscriptToolOutput::Pruned {
                                replacement: output_json,
                            }
                        } else {
                            TranscriptToolOutput::Text { text: output_json }
                        };
                        transcript.push(TranscriptFragment::ToolResult {
                            call_id: ToolCallId::from(SmolStr::from(tool_call_id)),
                            output,
                        });
                    }
                }
            }
            Role::Assistant => {
                let mut content = TranscriptContent::default();
                let mut tool_calls = Vec::new();
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
                        ProjectedSessionPart::ToolResult { .. } => {
                            // Tool results never appear under assistant role.
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
    tools: &[EntryDefinition],
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

pub(crate) fn prompt_char_budget(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    fallback_max_prompt_chars: usize,
    system: Option<&str>,
    tools: &[EntryDefinition],
) -> usize {
    let overhead_chars = approximate_request_overhead_chars(system, tools);
    let fallback_budget = fallback_max_prompt_chars
        .saturating_sub(overhead_chars)
        .max(APPROX_CHARS_PER_TOKEN * MIN_PROMPT_BUDGET_TOKENS as usize);

    let Some(context_window_tokens) = context_window_tokens.filter(|value| *value > 0) else {
        return fallback_budget;
    };

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
    let prompt_tokens = context_window_tokens
        .saturating_sub(reserve_tokens)
        .max(min_prompt_tokens);
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
    let prompt_messages = normalize_prompt_messages(active_messages.as_slice());
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

    if let PromptContinuationOutcome::Reuse {
        previous_response_id,
        delta_messages,
    } = continuation
    {
        return PreparedPrompt {
            system: None,
            messages: delta_messages,
            prompt_cache_key: prompt_cache_key_for_session(session),
            previous_response_id: Some(previous_response_id),
            prompt_window_generation: session.runtime.prompt_window.generation,
            system_fingerprint,
            request_options_fingerprint,
            provider_request_shape,
            continuation_reason: PromptContinuationReason::ReusedPreviousResponseId,
            continuation_diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    PreparedPrompt {
        system: options.system.map(ToOwned::to_owned),
        messages: prompt_messages,
        prompt_cache_key: prompt_cache_key_for_session(session),
        previous_response_id: None,
        prompt_window_generation: session.runtime.prompt_window.generation,
        system_fingerprint,
        request_options_fingerprint,
        provider_request_shape,
        continuation_reason: continuation.reason(),
        continuation_diagnostic: continuation.diagnostic(),
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
    fn reason(&self) -> PromptContinuationReason {
        match self {
            Self::Restart { reason, .. } => *reason,
            Self::Reuse { .. } => PromptContinuationReason::ReusedPreviousResponseId,
        }
    }

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

#[cfg(test)]
pub(crate) fn prune_tool_result_message(message: &mut Message) -> bool {
    if message.role != Role::Tool || message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
        return false;
    }

    if tool_result_output_chars(message) > 0 {
        message.metadata.add_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED);
        return true;
    }

    false
}

#[cfg(test)]
pub(crate) fn strip_attachment_payloads(message: &mut Message) -> bool {
    if message
        .metadata
        .has_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED)
        || message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED)
    {
        return false;
    }

    if attachment_payload_bytes(message) > 0 {
        message
            .metadata
            .add_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED);
        return true;
    }

    false
}

fn build_compaction_summary(messages: &[Message]) -> String {
    let sections = summary_sections(messages);
    let mut lines = vec![COMPACTION_SUMMARY_HEADING.to_string(), String::new()];
    lines.extend(render_summary_section(
        COMPACTION_SUMMARY_SECTIONS[0],
        sections.goal,
    ));
    lines.push(String::new());
    lines.extend(render_summary_section(
        COMPACTION_SUMMARY_SECTIONS[1],
        sections.instructions,
    ));
    lines.push(String::new());
    lines.extend(render_summary_section(
        COMPACTION_SUMMARY_SECTIONS[2],
        sections.discoveries,
    ));
    lines.push(String::new());
    lines.extend(render_summary_section(
        COMPACTION_SUMMARY_SECTIONS[3],
        sections.accomplished,
    ));
    lines.push(String::new());
    lines.extend(render_summary_section(
        COMPACTION_SUMMARY_SECTIONS[4],
        sections.relevant_files,
    ));
    lines.join("\n")
}

fn render_summary_section(title: &str, items: Vec<String>) -> Vec<String> {
    let mut lines = vec![format!("## {title}")];
    if items.is_empty() {
        lines.push("- No durable context captured.".to_string());
    } else {
        lines.extend(items.into_iter().map(|item| format!("- {item}")));
    }
    lines
}

fn summary_sections(messages: &[Message]) -> SummarySections {
    let mut sections = SummarySections::default();
    for message in messages {
        let Some(text) = summary_message_text(message) else {
            continue;
        };

        collect_relevant_files(text.as_str(), &mut sections.relevant_files);

        match message.role {
            Role::System => push_unique(&mut sections.instructions, text, 4),
            Role::User => {
                if sections.goal.is_empty() {
                    push_unique(&mut sections.goal, text.clone(), 3);
                } else {
                    push_unique(
                        &mut sections.instructions,
                        format!("User request: {text}"),
                        4,
                    );
                }
            }
            Role::Assistant => push_unique(&mut sections.accomplished, text, 5),
            Role::Tool => push_unique(&mut sections.discoveries, text, 5),
        }
    }

    if sections.goal.is_empty()
        && let Some(instruction) = sections.instructions.first().cloned()
    {
        sections.goal.push(instruction);
    }
    if sections.discoveries.is_empty() {
        for item in sections.accomplished.iter().take(2).cloned() {
            push_unique(&mut sections.discoveries, item, 3);
        }
    }
    if sections.accomplished.is_empty() {
        for item in sections.discoveries.iter().take(2).cloned() {
            push_unique(&mut sections.accomplished, item, 3);
        }
    }

    sections
}

fn summary_message_text(message: &Message) -> Option<String> {
    let text =
        if message.role == Role::Tool && message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
            PRUNED_TOOL_RESULT_PLACEHOLDER.to_owned()
        } else {
            project_session_text_lossy(message)
        };
    normalize_summary_text(text)
}

fn normalize_summary_text(text: String) -> Option<String> {
    let compact = text
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let compact = compact.trim();
    if compact.is_empty() {
        return None;
    }

    Some(truncate_chars(compact, 320))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn push_unique(items: &mut Vec<String>, value: String, limit: usize) {
    if value.trim().is_empty() || items.len() >= limit || items.contains(&value) {
        return;
    }
    items.push(value);
}

fn collect_relevant_files(text: &str, files: &mut Vec<String>) {
    let mut discovered = BTreeSet::new();
    for candidate in FILE_PATH_RE.find_iter(text).map(|m| m.as_str()) {
        if candidate.contains("://") {
            continue;
        }
        let cleaned = candidate
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '(' | ')' | '[' | ']'))
            .trim();
        if cleaned.is_empty() || cleaned.starts_with("[tool_") {
            continue;
        }
        discovered.insert(cleaned.replace('\\', "/"));
    }

    for item in discovered {
        push_unique(files, item, 8);
    }
}

fn tail_messages_to_keep(
    messages: &[Message],
    keep_tail_messages: usize,
    max_prompt_chars: usize,
) -> usize {
    if messages.is_empty() {
        return 0;
    }
    if messages.len() == 1 {
        return 1;
    }

    let total_chars = approximate_prompt_payload_chars(messages);
    let count_limited_keep = keep_tail_messages.max(1).min(messages.len() - 1);
    if total_chars <= max_prompt_chars {
        if messages.len() > keep_tail_messages.max(1) {
            return count_limited_keep;
        }
        return messages.len();
    }

    let tail_budget = max_prompt_chars
        .saturating_sub(COMPACTION_SUMMARY_BUDGET_CHARS)
        .max(1);
    let mut keep_tail = 1_usize;
    let mut tail_chars = approximate_message_payload_chars(&messages[messages.len() - 1]);
    while keep_tail < count_limited_keep {
        let next_index = messages.len() - keep_tail - 1;
        let next_chars = approximate_message_payload_chars(&messages[next_index]);
        if tail_chars.saturating_add(next_chars) > tail_budget {
            break;
        }
        tail_chars = tail_chars.saturating_add(next_chars);
        keep_tail += 1;
    }

    keep_tail
}

fn tool_result_output_chars(message: &Message) -> usize {
    if message.role != Role::Tool {
        return 0;
    }

    message
        .parts
        .iter()
        .map(|part| match part.content.as_ref() {
            Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                output_text, ..
            })) => output_text.len(),
            Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                output_text,
                error_message,
                ..
            })) => {
                if output_text.trim().is_empty() {
                    error_message.len()
                } else {
                    output_text.len()
                }
            }
            _ => 0,
        })
        .sum()
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
            let Some(PartContent::ToolExecution(exec)) = part.content.as_ref() else {
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
    if message.role != Role::Tool || message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
        return 0;
    }

    message
        .parts
        .iter()
        .map(|part| match part.content.as_ref() {
            Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                output_text, ..
            })) => output_text.len(),
            Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                output_text,
                error_message,
                ..
            })) => {
                if output_text.trim().is_empty() {
                    error_message.len()
                } else {
                    output_text.len()
                }
            }
            _ => 0,
        })
        .sum()
}

fn extend_pending_tool_outputs(pending: &mut Vec<PendingToolCallOutput>, assistant: &Message) {
    let mut seen = HashSet::new();
    for part in &assistant.parts {
        let Some(PartContent::ToolExecution(exec)) = part.content.as_ref() else {
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
            output_text: fallback_tool_result_output(exec),
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
            item.output_text,
        ));
        *next_synthetic_message_id -= 1;
    }
}

fn synthetic_tool_result_message(
    message_id: i64,
    tool_call_id: String,
    output_text: String,
) -> Message {
    let mut message = Message::prompt_tool_result(tool_call_id, output_text);
    message.id = message_id;
    message.metadata.source = MessageSource::Tool;
    message
}

fn tool_result_ids(message: &Message) -> Vec<String> {
    if message.role != Role::Tool {
        return Vec::new();
    }

    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for part in &message.parts {
        let Some(PartContent::ToolExecution(exec)) = part.content.as_ref() else {
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

fn fallback_tool_result_output(exec: &ToolExecutionPart) -> String {
    match exec {
        ToolExecutionPart::Pending { .. } | ToolExecutionPart::InProgress { .. } => {
            SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER.to_string()
        }
        ToolExecutionPart::Completed { output_text, .. } => {
            if output_text.trim().is_empty() {
                SYNTHETIC_TOOL_COMPLETED_PLACEHOLDER.to_string()
            } else {
                output_text.clone()
            }
        }
        ToolExecutionPart::Failed {
            output_text,
            error_message,
            ..
        } => {
            if !output_text.trim().is_empty() {
                output_text.clone()
            } else if !error_message.trim().is_empty() {
                error_message.clone()
            } else {
                SYNTHETIC_TOOL_FAILED_PLACEHOLDER.to_string()
            }
        }
    }
}

fn tool_execution_call_id(part: &MessagePart, exec: &ToolExecutionPart) -> Option<String> {
    let fallback = match exec {
        ToolExecutionPart::Pending { call_id, .. }
        | ToolExecutionPart::InProgress { call_id, .. }
        | ToolExecutionPart::Completed { call_id, .. }
        | ToolExecutionPart::Failed { call_id, .. } => call_id.to_string(),
    };
    let tool_call_id = part
        .operation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(fallback);

    (!tool_call_id.trim().is_empty()).then_some(tool_call_id)
}

fn tool_execution_invocation(exec: &ToolExecutionPart) -> &ToolInvocation {
    match exec {
        ToolExecutionPart::Pending { invocation, .. }
        | ToolExecutionPart::InProgress { invocation, .. }
        | ToolExecutionPart::Completed { invocation, .. }
        | ToolExecutionPart::Failed { invocation, .. } => invocation,
    }
}

fn tool_invocation_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation::Custom { name, .. } = invocation;
    name.clone()
}

fn tool_invocation_arguments_json(invocation: &ToolInvocation) -> String {
    let ToolInvocation::Custom { input, .. } = invocation;
    serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string())
}

fn attachment_payload_bytes(message: &Message) -> usize {
    message
        .parts
        .iter()
        .map(|part| match part.content.as_ref() {
            Some(PartContent::Attachment(attachment)) => attachment
                .attachments
                .iter()
                .map(attachment_item_payload_bytes)
                .sum(),
            _ => 0,
        })
        .sum()
}

fn attachment_item_payload_bytes(item: &crate::message::AttachmentItem) -> usize {
    match &item.source {
        AttachmentSource::DataUrl { url } | AttachmentSource::Url { url } => url.trim().len(),
        AttachmentSource::Base64 { data } => data.trim().len(),
        AttachmentSource::FileId { file_id } => file_id.trim().len(),
        AttachmentSource::LocalPath { .. } => 0,
    }
}

#[derive(Debug, Default)]
struct SummarySections {
    goal: Vec<String>,
    instructions: Vec<String>,
    discoveries: Vec<String>,
    accomplished: Vec<String>,
    relevant_files: Vec<String>,
}

fn fingerprint_request_options(
    provider_id: &str,
    model_id: &str,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
    tools: &[EntryDefinition],
    provider_request_shape: Option<&PromptCacheShape>,
) -> String {
    #[derive(Serialize)]
    struct RequestOptionsFingerprint<'a> {
        prompt_request_shape_version: u32,
        provider_id: &'a str,
        model_id: &'a str,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        tools: &'a [EntryDefinition],
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
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::{
        role::Role,
        tool::{EntryBehavior, EntryDefinition},
    };

    use super::*;
    use crate::session::{PromptWindowRuntime, ProviderPromptAnchor, SessionRuntimeState};

    #[test]
    fn active_prompt_messages_skips_compacted_and_promotes_latest_summary() {
        let mut compacted = Message::prompt_text(Role::User, "old");
        compacted.id = 1;
        compacted.metadata.add_tag(MESSAGE_TAG_PROMPT_COMPACTED);

        let mut current = Message::prompt_text(Role::User, "new");
        current.id = 2;

        let mut summary = Message::prompt_text(Role::System, "summary");
        summary.id = 3;
        summary.metadata.add_tag(MESSAGE_TAG_PROMPT_SUMMARY);

        let session = Session::new(7, 1, "prompt", Utc::now()).with_messages(vec![
            compacted,
            current.clone(),
            summary.clone(),
        ]);

        let prompt_messages = active_prompt_messages(&session);
        assert_eq!(prompt_messages.len(), 2);
        assert_eq!(prompt_messages[0].id, summary.id);
        assert_eq!(prompt_messages[1].id, current.id);
    }

    #[test]
    fn build_prepared_prompt_uses_previous_response_id_for_strict_extensions() {
        let mut assistant = Message::prompt_text(Role::Assistant, "done");
        assistant.id = 11;
        let mut user = Message::prompt_text(Role::User, "follow up");
        user.id = 12;

        let mut session =
            Session::new(99, 1, "continuation", Utc::now()).with_messages(vec![assistant, user]);
        session.runtime = SessionRuntimeState {
            turn: Default::default(),
            prompt_window: PromptWindowRuntime { generation: 2 },
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

        assert_eq!(prepared.previous_response_id.as_deref(), Some("resp_123"));
        assert_eq!(prepared.system, None);
        assert_eq!(prepared.messages.len(), 1);
        assert_eq!(prepared.messages[0].id, 12);
        assert_eq!(
            prepared.continuation_reason,
            PromptContinuationReason::ReusedPreviousResponseId
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
        session.runtime.prompt_window = PromptWindowRuntime { generation: 2 };
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
            fingerprint_optional_text(Some("system")).as_str(),
            fingerprint_request_options("openai", "gpt-5", Some(0.2), Some(256), &[], None)
                .as_str(),
        )
        .expect("runtime prompt token estimate should be available");

        let delta_tokens = approximate_tokens_from_chars(approximate_prompt_payload_chars(&[
            Message::prompt_text(Role::User, "follow up"),
        ]));
        assert_eq!(estimate.delta_tokens, delta_tokens);
        assert_eq!(estimate.total_tokens, 1_450 + delta_tokens);
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
        session.runtime.prompt_window = PromptWindowRuntime { generation: 2 };
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
            prompt_window: PromptWindowRuntime { generation: 2 },
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
        session.runtime.prompt_window = PromptWindowRuntime { generation: 2 };
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
    fn plan_compaction_compacts_head_and_keeps_tail() {
        let mut first = Message::prompt_text(Role::User, "one");
        first.id = 1;
        let mut second = Message::prompt_text(Role::Assistant, "two");
        second.id = 2;
        let mut third = Message::prompt_text(Role::User, "three");
        third.id = 3;

        let plan = plan_compaction(&[first, second, third], 1, 32_000).expect("plan should exist");
        assert_eq!(plan.compacted_message_ids, vec![1, 2]);
        assert!(plan.summary_text.contains("## Goal"));
        assert!(plan.summary_text.contains("- one"));
        assert!(plan.summary_text.contains("## Accomplished"));
        assert!(plan.summary_text.contains("- two"));
    }

    #[test]
    fn compaction_summary_template_stays_stable() {
        let mut system = Message::prompt_text(Role::System, "Always answer in Chinese.");
        system.id = 1;
        let mut user =
            Message::prompt_text(Role::User, "Update crates/agena/src/session/manager.rs");
        user.id = 2;
        let mut tool = Message::prompt_tool_result("call_1", "Found compact_prompt_window");
        tool.id = 3;
        let mut assistant = Message::prompt_text(Role::Assistant, "Wired the compaction worker.");
        assistant.id = 4;
        let mut tail = Message::prompt_text(Role::User, "Continue");
        tail.id = 5;

        let plan = plan_compaction(&[system, user, tool, assistant, tail], 1, 32_000)
            .expect("plan should exist");

        assert_eq!(
            plan.summary_text,
            "Conversation summary (compacted):\n\n## Goal\n- Update crates/agena/src/session/manager.rs\n\n## Instructions\n- Always answer in Chinese.\n\n## Discoveries\n- [tool_result:call_1]\n\n## Accomplished\n- Wired the compaction worker.\n\n## Relevant files / directories\n- crates/agena/src/session/manager.rs"
        );
    }

    #[test]
    fn plan_tool_result_pruning_targets_only_older_large_results() {
        let mut messages = Vec::new();
        let mut first = Message::prompt_tool_result("call_1", "x".repeat(13_000));
        first.id = 1;
        let mut first_user = Message::prompt_text(Role::User, "first turn");
        first_user.id = 2;
        let mut second = Message::prompt_tool_result("call_2", "y".repeat(13_000));
        second.id = 3;
        let mut second_user = Message::prompt_text(Role::User, "second turn");
        second_user.id = 4;
        let mut latest_tool = Message::prompt_tool_result("call_3", "z".repeat(2_000));
        latest_tool.id = 5;
        let mut latest_user = Message::prompt_text(Role::User, "latest turn");
        latest_user.id = 6;

        messages.extend([
            first,
            first_user,
            second,
            second_user,
            latest_tool,
            latest_user,
        ]);

        let plan = plan_tool_result_pruning(messages.as_slice()).expect("prune plan should exist");
        assert_eq!(plan.pruned_message_ids, vec![1]);
    }

    #[test]
    fn plan_attachment_payload_stripping_targets_only_older_large_inline_payloads() {
        let mut first = Message::prompt_parts(
            Role::User,
            vec![
                PartContent::text("old screenshot"),
                PartContent::attachments(vec![crate::message::AttachmentItem {
                    kind: crate::message::AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::DataUrl {
                        url: format!("data:image/png;base64,{}", "A".repeat(700_000)),
                    },
                    filename: Some("old.png".to_string()),
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
        first.id = 1;
        let mut second_user = Message::prompt_text(Role::User, "recent turn");
        second_user.id = 2;
        let mut third_user = Message::prompt_text(Role::User, "latest turn");
        third_user.id = 3;

        let plan = plan_attachment_payload_stripping(&[first, second_user, third_user])
            .expect("attachment strip plan should exist");
        assert_eq!(plan.stripped_message_ids, vec![1]);
    }

    #[test]
    fn prune_tool_result_message_replaces_old_output_with_placeholder() {
        let mut message = Message::prompt_tool_result("call_1", "very long output");
        message.id = 9;

        assert!(prune_tool_result_message(&mut message));
        assert!(message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED));
        assert_eq!(message.as_text_lossy(), "very long output".to_string());
        assert_eq!(
            crate::provider::project_session_text_lossy(&message),
            "[tool_result:call_1]".to_string()
        );
    }

    #[test]
    fn strip_attachment_payloads_keeps_persisted_message_but_changes_projection() {
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

        assert!(strip_attachment_payloads(&mut message));
        assert!(
            message
                .metadata
                .has_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED)
        );
        assert!(message.as_text_lossy().contains("shot.png"));
        assert_eq!(
            crate::provider::project_session_text_lossy(&message),
            "see screenshot[image:shot.png]".to_string()
        );
    }

    #[test]
    fn normalize_prompt_messages_synthesizes_missing_tool_results_before_next_turn() {
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 17,
                invocation: ToolInvocation::Custom {
                    name: "edit".to_string(),
                    input: crate::message::StructuredObject::try_from(
                        serde_json::json!({ "path": "src/main.rs" }),
                    )
                    .expect("structured tool input"),
                },
                output_text: "patched".to_string(),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: crate::message::ToolOutput::None,
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 2;

        let normalized = normalize_prompt_messages(&[assistant, user.clone()]);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].id, 1);
        assert_eq!(normalized[2].id, user.id);
        assert_eq!(normalized[1].role, Role::Tool);
        assert_eq!(
            primary_tool_result_id(&normalized[1]).as_deref(),
            Some("call_edit")
        );
        assert!(normalized[1].as_text_lossy().contains("patched"));
    }

    #[test]
    fn normalize_prompt_messages_matches_multi_tool_result_message_without_synthesizing_duplicates()
    {
        let invocation = ToolInvocation::Custom {
            name: "edit".to_string(),
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 17,
                    invocation: invocation.clone(),
                    output_text: "patched main".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::None,
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 18,
                    invocation: invocation.clone(),
                    output_text: "patched lib".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::None,
                    lifecycle: crate::message::TimeRange::default(),
                }),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut tool = Message::prompt_parts(
            Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 17,
                    invocation: invocation.clone(),
                    output_text: "patched main".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::None,
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 18,
                    invocation,
                    output_text: "patched lib".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: crate::message::ToolOutput::None,
                    lifecycle: crate::message::TimeRange::default(),
                }),
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
    fn normalize_prompt_messages_drops_orphan_tool_results() {
        let orphan = Message::prompt_tool_result("call_missing", "stale output");
        let normalized = normalize_prompt_messages(&[orphan]);
        assert!(normalized.is_empty());
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
        let invocation = ToolInvocation::Custom {
            name: "edit".to_string(),
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: 17,
                invocation: invocation.clone(),
                title: "editing".to_string(),
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut tool = Message::prompt_parts(
            Role::Tool,
            vec![PartContent::ToolExecution(ToolExecutionPart::InProgress {
                call_id: 17,
                invocation,
                title: String::new(),
                output_text: String::new(),
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        tool.id = 2;
        tool.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant, tool.clone(), user]);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[1].id, tool.id);
        assert_eq!(normalized[1].role, Role::Tool);
        assert_eq!(
            primary_tool_result_id(&normalized[1]).as_deref(),
            Some("call_edit")
        );
        assert_eq!(
            project_session_text_lossy(&normalized[1]),
            "[tool_result:call_edit]".to_string()
        );
        assert!(
            normalized[1]
                .as_text_lossy()
                .contains(SYNTHETIC_TOOL_INTERRUPTED_PLACEHOLDER)
        );
    }

    #[test]
    fn normalize_prompt_messages_expands_empty_multi_tool_results_into_placeholders() {
        let invocation = ToolInvocation::Custom {
            name: "edit".to_string(),
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id: 17,
                    invocation: invocation.clone(),
                    title: "editing main".to_string(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id: 18,
                    invocation: invocation.clone(),
                    title: "editing lib".to_string(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut tool = Message::prompt_parts(
            Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::InProgress {
                    call_id: 17,
                    invocation: invocation.clone(),
                    title: String::new(),
                    output_text: String::new(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::InProgress {
                    call_id: 18,
                    invocation,
                    title: String::new(),
                    output_text: String::new(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
            ],
        );
        tool.id = 2;
        tool.parts[0].operation_id = Some("call_edit_main".to_string());
        tool.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let normalized = normalize_prompt_messages(&[assistant, tool, user]);
        assert_eq!(normalized.len(), 4);
        assert_eq!(
            normalized
                .iter()
                .filter(|message| message.role == Role::Tool)
                .map(|message| primary_tool_result_id(message).expect("tool result id"))
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
        let invocation = ToolInvocation::Custom {
            name: "edit".to_string(),
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: 17,
                invocation: invocation.clone(),
                title: "editing".to_string(),
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit".to_string());

        let mut empty_tool = Message::prompt_parts(
            Role::Tool,
            vec![PartContent::ToolExecution(ToolExecutionPart::InProgress {
                call_id: 17,
                invocation,
                title: String::new(),
                output_text: String::new(),
                lifecycle: crate::message::TimeRange::default(),
            })],
        );
        empty_tool.id = 2;
        empty_tool.parts[0].operation_id = Some("call_edit".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let digest_without_tool = prompt_transcript_digest(&[assistant.clone(), user.clone()]);
        let digest_with_empty_tool = prompt_transcript_digest(&[assistant, empty_tool, user]);

        assert_eq!(digest_with_empty_tool, digest_without_tool);
    }

    #[test]
    fn prompt_transcript_digest_treats_empty_multi_tool_results_like_synthesized_placeholders() {
        let invocation = ToolInvocation::Custom {
            name: "edit".to_string(),
            input: crate::message::StructuredObject::try_from(
                serde_json::json!({ "path": "src/main.rs" }),
            )
            .expect("structured tool input"),
        };
        let mut assistant = Message::prompt_parts(
            Role::Assistant,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id: 17,
                    invocation: invocation.clone(),
                    title: "editing main".to_string(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id: 18,
                    invocation: invocation.clone(),
                    title: "editing lib".to_string(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
            ],
        );
        assistant.id = 1;
        assistant.parts[0].operation_id = Some("call_edit_main".to_string());
        assistant.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut empty_tool = Message::prompt_parts(
            Role::Tool,
            vec![
                PartContent::ToolExecution(ToolExecutionPart::InProgress {
                    call_id: 17,
                    invocation: invocation.clone(),
                    title: String::new(),
                    output_text: String::new(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
                PartContent::ToolExecution(ToolExecutionPart::InProgress {
                    call_id: 18,
                    invocation,
                    title: String::new(),
                    output_text: String::new(),
                    lifecycle: crate::message::TimeRange::default(),
                }),
            ],
        );
        empty_tool.id = 2;
        empty_tool.parts[0].operation_id = Some("call_edit_main".to_string());
        empty_tool.parts[1].operation_id = Some("call_edit_lib".to_string());

        let mut user = Message::prompt_text(Role::User, "continue");
        user.id = 3;

        let digest_without_tool = prompt_transcript_digest(&[assistant.clone(), user.clone()]);
        let digest_with_empty_tool = prompt_transcript_digest(&[assistant, empty_tool, user]);

        assert_eq!(digest_with_empty_tool, digest_without_tool);
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
    fn plan_compaction_compacts_small_histories_when_visible_payload_is_too_large() {
        let mut first = Message::prompt_text(Role::User, "A".repeat(8_000));
        first.id = 1;
        let mut second = Message::prompt_text(Role::Assistant, "B".repeat(8_000));
        second.id = 2;

        let plan = plan_compaction(&[first, second.clone()], 12, 6_000)
            .expect("large visible payload should trigger compaction");
        assert_eq!(plan.compacted_message_ids, vec![1]);
        assert!(plan.summary_text.contains("## Goal"));
        assert!(plan.summary_text.contains("A"));
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
        let tool = EntryDefinition::builtin::<serde_json::Value>(
            "grep",
            "Search files.",
            EntryBehavior::ReadOnly,
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
            prompt_window: PromptWindowRuntime { generation: 2 },
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
        let tool = EntryDefinition::builtin::<serde_json::Value>(
            "grep",
            "Search files.",
            EntryBehavior::ReadOnly,
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
