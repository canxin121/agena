use serde::Serialize;
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::{
    provider::project_completion_input,
    tool::ToolApiBinding,
};
use agena_domain::{Role, ToolCallId};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentSource, CompletionInputMessage,
    CompletionInputPart, PromptCacheShape, PromptCacheShapeDiff, ProviderCompactionContext,
};
use agena_storage::store::{Part, PartRole, PartState};

use super::Session;
use super::model::{
    PromptCompactionContent, PromptCompactionMessage, PromptCompactionRuntime,
};
use super::store::parts_into_runs;
use super::transcript::{
    ProviderTranscript, TranscriptBlock, TranscriptContent, TranscriptFragment, TranscriptToolCall,
    TranscriptToolOutput,
};

const PROMPT_PROTOCOL_OVERHEAD_CHARS: usize = 2_048;
/// Fixed discriminator for the one current development request shape.
/// Incompatible development state is reset instead of assigning a new value.
const PROMPT_REQUEST_SHAPE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedPrompt {
    pub system: Option<String>,
    pub messages: Vec<CompletionInputMessage>,
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

/// One provider-visible prompt window item. Run-projected items carry the
/// durable run marker part id (the v2 message identity, design 4.1) so the
/// continuation / estimate paths can locate an anchor by id; synthetic
/// checkpoint and recent-suffix messages injected from a local-summary
/// compaction carry `None`.
#[derive(Debug, Clone)]
struct WindowItem {
    id: Option<i64>,
    message: CompletionInputMessage,
}

/// Project the session's active model window — [`Session::active_window_parts`],
/// the parts strictly after the last compaction checkpoint (13.4) — run by
/// run into the provider input contract. A local-summary compaction's
/// durable checkpoint summary and retained recent suffix are injected at the
/// front (the summary lives on the compaction marker part; the bounded recent
/// suffix snapshot lives in the runtime).
fn prompt_window_items(
    session: &Session,
    provider_id: Option<&str>,
    adapter_id: Option<&str>,
    model_id: Option<&str>,
    native_compaction_enabled: bool,
) -> Vec<WindowItem> {
    let compaction = session.runtime.prompt_window.compaction.as_ref();
    project_window_items(
        session.active_window_parts(),
        compaction,
        session,
        provider_id,
        adapter_id,
        model_id,
        native_compaction_enabled,
    )
}

/// Project an explicit parts slice run by run into the provider input contract,
/// with the installed local-summary checkpoint injection prepended when present.
fn project_window_items(
    parts: &[Part],
    compaction: Option<&PromptCompactionRuntime>,
    session: &Session,
    provider_id: Option<&str>,
    adapter_id: Option<&str>,
    model_id: Option<&str>,
    native_compaction_enabled: bool,
) -> Vec<WindowItem> {
    let Some(compaction) = compaction.filter(|value| !value.is_empty()) else {
        // Prompt-cache affinity depends on every later request preserving the
        // exact provider-visible prefix from earlier requests. Without an
        // installed compaction snapshot, the prompt path stays append-only.
        return window_items_from_parts(parts);
    };

    match &compaction.content {
        PromptCompactionContent::TextSummary {
            summary,
            recent_messages,
        } => {
            let mut items = Vec::with_capacity(recent_messages.len().saturating_add(4));
            items.push(WindowItem {
                id: None,
                message: compaction_summary_message(session, summary.as_str()),
            });
            items.extend(recent_messages.iter().map(|message| WindowItem {
                id: None,
                message: checkpoint_recent_message(message),
            }));
            items.extend(window_items_from_parts(parts));
            items
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
            // The window is still the parts after the last checkpoint marker;
            // the opaque native checkpoint travels as a
            // `ProviderCompactionContext` instead of replayed transcript.
            window_items_from_parts(parts)
        }
        // Native provider checkpoints are not portable. A model switch must
        // replay canonical Agena history rather than interpreting opaque data.
        PromptCompactionContent::OpenAiResponses { .. } => {
            window_items_from_parts(parts)
        }
    }
}

/// Compaction source: the active window projected into the provider input
/// contract (including any installed TextSummary checkpoint injection), minus
/// assistant runs that failed or were cancelled — they carry no provider-visible
/// content worth summarizing or counting toward compaction safety.
pub(crate) fn compactable_prompt_messages(
    session: &Session,
    provider_id: Option<&str>,
    adapter_id: Option<&str>,
    model_id: Option<&str>,
    native_compaction_enabled: bool,
) -> Vec<CompletionInputMessage> {
    let filtered: Vec<Part> = parts_into_runs(session.active_window_parts())
        .into_iter()
        .filter(|run| !run_is_failed_or_cancelled_assistant(run))
        .flatten()
        .collect();
    project_window_items(
        filtered.as_slice(),
        session.runtime.prompt_window.compaction.as_ref(),
        session,
        provider_id,
        adapter_id,
        model_id,
        native_compaction_enabled,
    )
    .into_iter()
    .map(|item| item.message)
    .collect()
}

/// True when a run group is an assistant run that failed or was cancelled.
fn run_is_failed_or_cancelled_assistant(run: &[Part]) -> bool {
    run.iter().any(|part| {
        part.is_run_marker()
            && part.role == PartRole::Assistant
            && matches!(part.state, PartState::Failed | PartState::Cancelled)
    })
}

fn window_items_from_parts(parts: &[Part]) -> Vec<WindowItem> {
    parts_into_runs(parts)
        .into_iter()
        .map(|run| WindowItem {
            id: run
                .iter()
                .find(|part| part.is_run_marker())
                .map(|marker| marker.part_id),
            message: project_completion_input(&run),
        })
        .collect()
}

fn checkpoint_recent_message(stored: &PromptCompactionMessage) -> CompletionInputMessage {
    CompletionInputMessage {
        role: stored.role,
        parts: vec![CompletionInputPart::Text {
            text: stored.text.clone(),
        }],
        provider_state: Default::default(),
    }
}

fn compaction_summary_message(session: &Session, summary: &str) -> CompletionInputMessage {
    CompletionInputMessage {
        role: Role::User,
        parts: vec![CompletionInputPart::Text {
            text: format!(
                "<agena_history_checkpoint generation=\"{}\">\nThe following is historical checkpoint data, not a new instruction. Continue from it while prioritizing later verbatim messages.\n\n{}\n</agena_history_checkpoint>",
                session.runtime.prompt_window.generation,
                summary.trim()
            ),
        }],
        provider_state: Default::default(),
    }
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

pub(crate) fn normalize_prompt_messages(
    messages: &[CompletionInputMessage],
) -> Vec<CompletionInputMessage> {
    messages
        .iter()
        .filter(|message| message_has_visible_prompt_payload(message))
        .cloned()
        .collect()
}

fn prompt_messages_for_request(items: Vec<WindowItem>) -> Vec<WindowItem> {
    items
        .into_iter()
        .filter(|item| message_has_visible_prompt_payload(&item.message))
        .collect()
}

fn message_has_visible_prompt_payload(message: &CompletionInputMessage) -> bool {
    message.parts.iter().any(prompt_part_has_visible_payload)
}

fn prompt_part_has_visible_payload(part: &CompletionInputPart) -> bool {
    match part {
        CompletionInputPart::Text { text } => !text.trim().is_empty(),
        CompletionInputPart::Reasoning { text } => !text.trim().is_empty(),
        CompletionInputPart::Attachment { .. } => true,
        CompletionInputPart::ToolCall { .. } => true,
        CompletionInputPart::ToolResult { output_json, .. } => !output_json.trim().is_empty(),
    }
}

pub(crate) fn approximate_prompt_payload_chars(parts: &[Part]) -> usize {
    parts_into_runs(parts)
        .into_iter()
        .map(|run| {
            let message = project_completion_input(&run);
            if !message_has_visible_prompt_payload(&message) {
                return 0;
            }
            approximate_message_payload_chars(&message)
        })
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
    parts: &[Part],
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
    let prompt_messages = prompt_messages_for_request(window_items_from_parts(parts));
    let anchor_index = prompt_messages
        .iter()
        .position(|item| item.id == Some(assistant_message_id))?;
    if !runtime.transcript_digest.is_empty()
        && prompt_prefix_transcript_digest(prompt_messages.as_slice(), anchor_index)
            != runtime.transcript_digest
    {
        return None;
    }
    // The provider's previous response includes the request prefix, but not the
    // assistant output itself. Include the anchor response plus later deltas.
    let delta_chars: usize = prompt_messages[anchor_index..]
        .iter()
        .map(|item| approximate_message_payload_chars(&item.message))
        .sum();
    let delta_tokens = agena_runtime::estimate_prompt_tokens_from_chars(delta_chars);

    Some(PromptTokenEstimate {
        total_tokens: last_successful_prompt_tokens.saturating_add(delta_tokens),
        delta_tokens,
        delta_chars: delta_chars as u64,
    })
}

pub(crate) fn prompt_transcript_digest(parts: &[Part]) -> String {
    let normalized = prompt_messages_for_request(window_items_from_parts(parts));
    prompt_prefix_transcript_digest(normalized.as_slice(), normalized.len().saturating_sub(1))
}

/// Compute the prompt-prefix transcript digest by projecting
/// `items[..=inclusive_end]` into a [`ProviderTranscript`] and hashing it with
/// [`ProviderTranscript::digest_hex`].
///
/// Append-only refactor invariant: the digest depends only on cache-stable
/// content (role, text/reasoning blocks, attachment source, tool call
/// name+arguments, tool result output) — never on mutable per-message state
/// (status, timestamps, in-memory ids).
fn prompt_prefix_transcript_digest(items: &[WindowItem], inclusive_end: usize) -> String {
    let end = inclusive_end.saturating_add(1).min(items.len());
    let transcript = messages_to_provider_transcript(&items[..end]);
    transcript.digest_hex()
}

fn messages_to_provider_transcript(items: &[WindowItem]) -> ProviderTranscript {
    let mut transcript = ProviderTranscript::new();
    for item in items {
        let message = &item.message;
        match message.role {
            Role::Assistant => {
                let mut content_blocks = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();
                let mut had_any = false;
                for part in &message.parts {
                    match part {
                        CompletionInputPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Text { text: text.clone() });
                            }
                        }
                        CompletionInputPart::Reasoning { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks
                                    .push(TranscriptBlock::Reasoning { text: text.clone() });
                            }
                        }
                        CompletionInputPart::Attachment { attachment } => {
                            had_any = true;
                            content_blocks.push(attachment_to_transcript_block(attachment));
                        }
                        CompletionInputPart::ToolCall {
                            id,
                            function,
                            arguments_json,
                        } => {
                            had_any = true;
                            tool_calls.push(TranscriptToolCall {
                                call_id: ToolCallId::from(SmolStr::from(id.as_str())),
                                name: SmolStr::from(function.function_name()),
                                arguments: arguments_json.clone(),
                            });
                        }
                        CompletionInputPart::ToolResult {
                            tool_call_id,
                            output_json,
                            ..
                        } => {
                            if output_json.is_empty() {
                                continue;
                            }
                            tool_results.push(TranscriptFragment::ToolResult {
                                call_id: ToolCallId::from(SmolStr::from(tool_call_id.as_str())),
                                output: TranscriptToolOutput::Text {
                                    text: output_json.clone(),
                                },
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
                for part in &message.parts {
                    if let CompletionInputPart::ToolResult {
                        tool_call_id,
                        output_json,
                        ..
                    } = part
                    {
                        if output_json.is_empty() {
                            continue;
                        }
                        transcript.push(TranscriptFragment::ToolResult {
                            call_id: ToolCallId::from(SmolStr::from(tool_call_id.as_str())),
                            output: TranscriptToolOutput::Text {
                                text: output_json.clone(),
                            },
                        });
                    }
                }
            }
            Role::User | Role::System => {
                let mut content_blocks = Vec::new();
                let mut had_any = false;
                for part in &message.parts {
                    match part {
                        CompletionInputPart::Text { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Text { text: text.clone() });
                            }
                        }
                        CompletionInputPart::Attachment { attachment } => {
                            had_any = true;
                            content_blocks.push(attachment_to_transcript_block(attachment));
                        }
                        // ToolCall / ToolResult are not produced under user/system roles
                        // by `project_completion_input`; ignore for digest stability.
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

fn attachment_to_transcript_block(item: &CompletionInputAttachment) -> TranscriptBlock {
    // Encode attachment identity into a stable text block. The exact wire bytes
    // here are part of the cache-stability contract; only fields that survive
    // a round-trip through the provider participate.
    let source_marker = match &item.source {
        CompletionInputAttachmentSource::Url { url } => format!("url:{}", url.trim()),
        CompletionInputAttachmentSource::DataUrl { url } => format!("data_url:{}", url.trim()),
        CompletionInputAttachmentSource::Base64 { data } => {
            format!("base64:{}", digest_bytes(data.trim().as_bytes()))
        }
        CompletionInputAttachmentSource::FileId { id } => format!("file_id:{}", id.trim()),
        CompletionInputAttachmentSource::LocalPath { path } => format!("local_path:{}", path.trim()),
    };
    let kind_marker = match item.kind {
        agena_provider::CompletionInputAttachmentKind::Image => "image",
        agena_provider::CompletionInputAttachmentKind::Audio => "audio",
        agena_provider::CompletionInputAttachmentKind::Video => "video",
        agena_provider::CompletionInputAttachmentKind::Pdf => "pdf",
        agena_provider::CompletionInputAttachmentKind::File => "file",
    };
    TranscriptBlock::Attachment {
        file_id: SmolStr::from(format!(
            "{}|{}|{}|{}",
            kind_marker,
            item.mime.trim(),
            item.title.as_deref().unwrap_or("").trim(),
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

pub(crate) fn approximate_total_request_tokens_with_compaction(
    parts: &[Part],
    system: Option<&str>,
    tools: &[ToolApiBinding],
    provider_compaction: Option<&ProviderCompactionContext>,
) -> u64 {
    let native_chars = provider_compaction
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    let total_chars = approximate_prompt_payload_chars(parts)
        .saturating_add(approximate_request_overhead_chars(system, tools))
        .saturating_add(native_chars);
    agena_runtime::estimate_prompt_tokens_from_chars(total_chars)
}

/// Char/token estimate over an already-projected message list (e.g. a
/// compaction candidate assembled from a synthetic checkpoint message plus a
/// hardened recent suffix, which has no backing parts). Mirrors
/// [`approximate_total_request_tokens`] over [`CompletionInputMessage`]s.
pub(crate) fn approximate_request_tokens_from_messages(
    messages: &[CompletionInputMessage],
    system: Option<&str>,
    tools: &[ToolApiBinding],
) -> u64 {
    let payload_chars = messages
        .iter()
        .map(approximate_message_payload_chars)
        .sum::<usize>();
    let total_chars = payload_chars.saturating_add(approximate_request_overhead_chars(system, tools));
    agena_runtime::estimate_prompt_tokens_from_chars(total_chars)
}

/// [`approximate_request_tokens_from_messages`] with native compaction payload.
pub(crate) fn approximate_request_tokens_from_messages_with_compaction(
    messages: &[CompletionInputMessage],
    system: Option<&str>,
    tools: &[ToolApiBinding],
    provider_compaction: Option<&ProviderCompactionContext>,
) -> u64 {
    let native_chars = provider_compaction
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| bytes.len())
        .unwrap_or_default();
    let payload_chars = messages
        .iter()
        .map(approximate_message_payload_chars)
        .sum::<usize>();
    let total_chars = payload_chars
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
    let active_messages = prompt_window_items(
        session,
        Some(options.provider_id),
        options.adapter_id,
        Some(options.model_id),
        options.native_compaction_enabled,
    );
    let prompt_messages = prompt_messages_for_request(active_messages);
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
            prompt_messages
                .into_iter()
                .map(|item| item.message)
                .collect(),
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
        delta_messages: Vec<CompletionInputMessage>,
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
    prompt_messages: &[WindowItem],
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
        .position(|item| item.id == Some(anchor.assistant_message_id))
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

    let delta_messages = prompt_messages[anchor_index + 1..]
        .iter()
        .map(|item| item.message.clone())
        .collect::<Vec<_>>();
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
pub(crate) fn project_transcript(parts: &[Part], budget_chars: usize) -> String {
    let projected = parts_into_runs(parts)
        .iter()
        .map(|run| crate::provider::project_session_text_lossy(run))
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

fn approximate_message_payload_chars(message: &CompletionInputMessage) -> usize {
    message
        .parts
        .iter()
        .map(|part| match part {
            CompletionInputPart::Text { text } | CompletionInputPart::Reasoning { text } => {
                text.len()
            }
            CompletionInputPart::Attachment { .. } => 64,
            CompletionInputPart::ToolCall {
                id,
                function,
                arguments_json,
                ..
            } => id
                .len()
                .saturating_add(function.function_name().len())
                .saturating_add(arguments_json.len())
                .saturating_add(16),
            CompletionInputPart::ToolResult { output_json, .. } => output_json.len(),
        })
        .sum()
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
    use crate::message::PartContent;
    use crate::session::model::{
        PromptCompactionContent, PromptCompactionMessage, PromptCompactionRuntime,
    };
    use crate::session::store::{part_content_to_value, run_marker_content};
    use agena_domain::{MessageSource, PromptCompactionStrategy, PromptCompactionTrigger};
    use agena_provider::CompletionUsage;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
    use chrono::{DateTime, Utc};

    /// Build one run marker part (`part_id` = the durable message id) plus its
    /// text content part, the v2 parts shape the prompt path projects onto
    /// provider input messages via `project_completion_input`.
    fn run_parts(id: i64, role: PartRole, source: &str, text: &str, now: DateTime<Utc>) -> Vec<Part> {
        let mut content = run_marker_content("user_send", None, None, None, None);
        content["source"] = serde_json::json!(source);
        vec![
            Part {
                part_id: id,
                kind: "run".to_owned(),
                role,
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
                role,
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

    /// A durable compaction checkpoint run marker closing the preceding window.
    fn compaction_marker(id: i64, summary: &str, now: DateTime<Utc>) -> Part {
        Part {
            part_id: id,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: serde_json::json!({
                "run_kind": "compaction",
                "summary": summary,
                "window": "through_message:2"
            }),
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
        }
    }

    /// Session with two pre-compaction messages, a compaction checkpoint, and
    /// one post-compaction user message: the active window (parts after the
    /// checkpoint marker) is exactly the post-compaction message.
    fn session_with_checkpoint() -> Session {
        let now = Utc::now();
        let mut session = Session::new(7, 11, "test", now);
        let mut parts = run_parts(1, PartRole::User, "user", "old user", now);
        parts.extend(run_parts(2, PartRole::Assistant, "tool", "old assistant", now));
        parts.push(compaction_marker(50, "durable state", now));
        parts.extend(run_parts(3, PartRole::User, "user", "future user", now));
        session.install_projected_parts(parts);
        session
    }

    fn text_lossy_list(messages: &[CompletionInputMessage]) -> Vec<String> {
        messages.iter().map(|message| message.as_text_lossy()).collect()
    }

    #[test]
    fn text_checkpoint_uses_summary_recent_suffix_and_future_only() {
        let mut session = session_with_checkpoint();
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

        let active = compactable_prompt_messages(&session, Some("p"), None, Some("m"), false);
        let texts = text_lossy_list(&active);
        assert_eq!(texts.len(), 3);
        assert!(texts[0].contains("durable state"));
        assert_eq!(texts[1], "retained assistant");
        assert_eq!(texts[2], "future user");
        assert!(
            texts.iter().all(|text| text != "old user" && text != "old assistant"),
            "pre-checkpoint transcript must be excluded from the window; got {texts:?}"
        );
    }

    #[test]
    fn hook_only_activity_is_filtered_from_the_model_prompt() {
        let now = Utc::now();
        let mut session = session_with_checkpoint();

        // A recorded hook run (human-only activity) must never reach the
        // model: it projects to no provider payload.
        let mut parts = session.parts().to_vec();
        parts.extend(hook_run_parts(99, now));

        // A real user message following the hook-only activity is unaffected.
        parts.extend(run_parts(100, PartRole::User, "user", "trailing user", now));
        session.install_projected_parts(parts);

        let prompt_messages = prompt_messages_for_request(window_items_from_parts(session.parts()));
        assert!(
            prompt_messages.iter().all(|item| item.id != Some(99)),
            "hook-only assistant message must not be sent to the model; got {:?}",
            prompt_messages
                .iter()
                .map(|item| (item.id, item.message.as_text_lossy()))
                .collect::<Vec<_>>()
        );
        // The base messages plus the trailing user message remain; the
        // hook-only assistant message and the checkpoint marker project to no
        // provider payload.
        assert_eq!(
            prompt_messages
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3), Some(100)]
        );
    }

    #[test]
    fn native_checkpoint_is_opaque_and_model_scoped() {
        let mut session = session_with_checkpoint();
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

        let matching = compactable_prompt_messages(
            &session,
            Some("openai"),
            Some("responses"),
            Some("gpt"),
            true,
        );
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].as_text_lossy(), "future user");
        assert!(
            provider_compaction_for_model(&session, "openai", Some("responses"), "gpt", true)
                .is_some()
        );

        let locally_compacted = compactable_prompt_messages(
            &session,
            Some("openai"),
            Some("responses"),
            Some("gpt"),
            false,
        );
        // With native compaction disabled the window is still the parts after
        // the checkpoint marker; the opaque checkpoint never replays as text.
        let projected = compactable_prompt_messages(&session, None, None, None, false);
        assert_eq!(locally_compacted.len(), projected.len());
        for (actual, expected) in locally_compacted.iter().zip(projected.iter()) {
            assert_eq!(
                (actual.role, actual.as_text_lossy()),
                (expected.role, expected.as_text_lossy()),
            );
        }
        assert!(
            provider_compaction_for_model(&session, "openai", Some("responses"), "gpt", false)
                .is_none()
        );

        let wrong_adapter = compactable_prompt_messages(
            &session,
            Some("openai"),
            Some("chat"),
            Some("gpt"),
            true,
        );
        assert_eq!(wrong_adapter.len(), 1);

        let switched = compactable_prompt_messages(
            &session,
            Some("anthropic"),
            None,
            Some("claude"),
            true,
        );
        assert_eq!(switched.len(), 1);
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
        let now = Utc::now();
        let mut session = Session::new(7, 11, "test", now);
        let mut parts = run_parts(1, PartRole::User, "user", "old user", now);
        parts.extend(run_parts(2, PartRole::Assistant, "tool", "old assistant", now));
        session.install_projected_parts(parts);
        let window = session.parts().to_vec();
        let digest = prompt_transcript_digest(&window[..window.len()]);
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
        let estimate = estimate_prompt_tokens_from_runtime(&session, &window, "system", "options")
            .expect("matching runtime estimate");
        assert!(estimate.delta_chars >= "old assistant".len() as u64);
        assert!(estimate.total_tokens > 100);
    }

    #[test]
    fn active_window_projects_parts_after_the_checkpoint_marker() {
        let session = session_with_checkpoint();
        let window = session.active_window_parts();
        assert_eq!(window.len(), 2, "only the post-checkpoint run's parts");
        assert_eq!(
            text_lossy_list(&window_items_from_parts(window)
                .into_iter()
                .map(|item| item.message)
                .collect::<Vec<_>>()),
            vec!["future user".to_owned()]
        );
    }
}
