use serde::Serialize;
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::{provider::project_completion_input, tool::ToolApiBinding};
use agena_domain::{Role, ToolCallId};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentSource, CompletionInputPart,
    CompletionInputRun, PromptCacheShape, PromptCacheShapeDiff, ProviderCompactionContext,
};
use agena_storage::store::{Part, PartRole, PartState};

use super::Session;
use super::model::{PromptCompactionContent, PromptCompactionMessage, PromptCompactionRuntime};
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
    pub turns: Vec<CompletionInputRun>,
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
    NoDeltaRuns,
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
            Self::NoDeltaRuns => "no_delta_runs",
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
    run: CompletionInputRun,
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
    let active_parts = session.active_window_parts();
    let mut items = project_window_items(
        active_parts,
        compaction,
        session,
        provider_id,
        adapter_id,
        model_id,
        native_compaction_enabled,
    );
    let active_ids = active_parts
        .iter()
        .map(|part| part.part_id)
        .collect::<std::collections::BTreeSet<_>>();
    // A delivery committed before a compaction checkpoint must not disappear
    // before the provider round that handles it. Re-append only protocol-owned
    // notifications that are outside the active suffix and have no successful
    // round receipt yet. Once handled, the checkpoint summary is sufficient
    // history and the exact hook is no longer pinned.
    for notification in session.parts().iter().filter(|part| {
        uses_provider_round_delivery(part)
            && !active_ids.contains(&part.part_id)
            && !notification_has_completed_provider_round(session.parts(), part.part_id)
    }) {
        items.push(WindowItem {
            id: Some(notification.part_id),
            run: project_completion_input(&[notification.clone()]),
        });
    }
    items
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
                run: compaction_summary_run(session, summary.as_str()),
            });
            items.extend(recent_messages.iter().map(|run| WindowItem {
                id: None,
                run: checkpoint_recent_run(run),
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
        PromptCompactionContent::OpenAiResponses { .. } => window_items_from_parts(parts),
    }
}

/// Compaction source: the active window projected into the provider input
/// contract (including any installed TextSummary checkpoint injection), minus
/// assistant runs that failed or were cancelled — they carry no provider-visible
/// content worth summarizing or counting toward compaction safety.
pub(crate) fn compactable_prompt_runs(
    session: &Session,
    provider_id: Option<&str>,
    adapter_id: Option<&str>,
    model_id: Option<&str>,
    native_compaction_enabled: bool,
) -> Vec<CompletionInputRun> {
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
    .map(|item| item.run)
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

/// Durable proof that a successful assistant provider round actually received
/// `notification_part_id` in its prompt. This is shared by delivery recovery
/// and prompt compaction pinning so both paths use one definition of handled.
pub(crate) fn notification_has_completed_provider_round(
    parts: &[Part],
    notification_part_id: i64,
) -> bool {
    parts.iter().any(|part| {
        part.is_run_marker()
            && part.role == PartRole::Assistant
            && part.state == PartState::Completed
            && part
                .content
                .get("run_kind")
                .and_then(serde_json::Value::as_str)
                == Some("continue")
            && part
                .content
                .get(crate::session::processor::MARKER_ROUNDS_KEY)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|rounds| {
                    rounds.iter().any(|round| {
                        round
                            .get("input_notification_part_ids")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|ids| {
                                ids.iter()
                                    .any(|id| id.as_i64() == Some(notification_part_id))
                            })
                    })
                })
    })
}

fn uses_provider_round_delivery(part: &Part) -> bool {
    part.kind == "system_notification"
        && part
            .content
            .get("delivery_protocol")
            .and_then(serde_json::Value::as_str)
            == Some("provider_round_v1")
}

/// Notification ids represented in the exact prompt built for the next
/// provider request. A newly committed delivery is normally in the active
/// window. If compaction ran before its response, the protocol marker pins the
/// still-unhandled notification after the checkpoint until a round receipt
/// proves consumption.
pub(crate) fn provider_visible_notification_part_ids(session: &Session) -> Vec<i64> {
    let active_ids = session
        .active_window_parts()
        .iter()
        .map(|part| part.part_id)
        .collect::<std::collections::BTreeSet<_>>();
    session
        .parts()
        .iter()
        .filter(|part| {
            part.kind == "system_notification"
                && (active_ids.contains(&part.part_id)
                    || (uses_provider_round_delivery(part)
                        && !notification_has_completed_provider_round(
                            session.parts(),
                            part.part_id,
                        )))
        })
        .map(|part| part.part_id)
        .collect()
}

fn window_items_from_parts(parts: &[Part]) -> Vec<WindowItem> {
    // Presentation groups an AI-owned background hook under the assistant run
    // that launched it. Provider order has a different responsibility: a hook
    // that arrives while a later turn is active must appear at its delivery
    // boundary, never retroactively before conversation that already happened.
    // Remove Assistant notifications from run grouping and reinsert each one
    // immediately before the first provider round that durably lists it as an
    // input. An unhandled notification stays at the prompt tail. Thus the
    // pre-response prefix is `[...existing rounds, hook]`; after the response
    // commits it grows append-only to `[...existing rounds, hook, response]`.
    let mut assistant_notifications = parts
        .iter()
        .filter(|part| part.kind == "system_notification" && part.role == PartRole::Assistant)
        .cloned()
        .collect::<Vec<_>>();
    assistant_notifications.sort_by_key(|part| (part.created_at_ms, part.part_id));
    let grouped_parts = parts
        .iter()
        .filter(|part| !(part.kind == "system_notification" && part.role == PartRole::Assistant))
        .cloned()
        .collect::<Vec<_>>();
    let mut notification_consumers = std::collections::BTreeMap::new();
    for marker in parts.iter().filter(|part| part.is_run_marker()) {
        let Some(rounds) = marker
            .content
            .get(crate::session::processor::MARKER_ROUNDS_KEY)
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for (round_index, round) in rounds.iter().enumerate() {
            let Some(ids) = round
                .get("input_notification_part_ids")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for notification_id in ids.iter().filter_map(serde_json::Value::as_i64) {
                notification_consumers
                    .entry(notification_id)
                    .or_insert((marker.part_id, round_index));
            }
        }
    }

    let mut items = parts_into_runs(&grouped_parts)
        .into_iter()
        .flat_map(|run| {
            let marker = run.first().expect("run group has a marker");
            let marker_id = marker.part_id;
            // Multi-round turns (one user message == one run marker) carry the
            // per-round records on the marker's `content["rounds"]` (written by
            // the processor at each round's end). Re-split the merged run into
            // one wire message per round so each provider round-trip keeps its
            // own reasoning passback (required by reasoning gateways) while all
            // parts still persist under a single run marker.
            let Some(rounds) = marker
                .content
                .get(crate::session::processor::MARKER_ROUNDS_KEY)
                .and_then(serde_json::Value::as_array)
                .filter(|rounds| !rounds.is_empty())
            else {
                return vec![WindowItem {
                    id: Some(marker_id),
                    run: project_completion_input(&run),
                }];
            };
            let content_parts = &run[1..];
            let mut items: Vec<WindowItem> = Vec::with_capacity(rounds.len());
            let mut claimed: Vec<i64> = Vec::new();
            for round in rounds {
                let part_ids = round
                    .get("part_ids")
                    .and_then(serde_json::Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(serde_json::Value::as_i64)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let round_parts = content_parts
                    .iter()
                    .filter(|part| part_ids.contains(&part.part_id))
                    .cloned()
                    .collect::<Vec<_>>();
                claimed.extend(part_ids.iter().copied());
                // Project the round's parts (think/tool_call etc.), then
                // override the replay state with the round's own record so each
                // wire message carries the reasoning passback that belongs to
                // exactly its provider round-trip.
                let provider_state = round
                    .get("provider_state")
                    .cloned()
                    .filter(|value| !value.is_null())
                    .and_then(|value| crate::provider::completion_input_provider_state(&value));
                let mut projected = project_completion_input(&round_parts);
                if let Some(provider_state) = provider_state {
                    projected.provider_state = provider_state;
                }
                items.push(WindowItem {
                    id: Some(marker_id),
                    run: projected,
                });
            }
            // Any content parts not claimed by a round record (interaction
            // parts appended between rounds, legacy parts) attach to the last
            // wire message so nothing is silently dropped from the prompt.
            let unclaimed = content_parts
                .iter()
                .filter(|part| !claimed.contains(&part.part_id))
                .cloned()
                .collect::<Vec<_>>();
            if !unclaimed.is_empty() {
                if let Some(last) = items.last_mut() {
                    let tail = crate::provider::project_session_parts(&unclaimed);
                    let mut appended = tail
                        .into_iter()
                        .map(crate::provider::completion_input_part_from_wire)
                        .collect::<Vec<_>>();
                    last.run.parts.append(&mut appended);
                } else {
                    items.push(WindowItem {
                        id: Some(marker_id),
                        run: project_completion_input(&run),
                    });
                }
            }
            items
        })
        .collect::<Vec<_>>();

    for notification in assistant_notifications {
        let insertion_index = notification_consumers
            .get(&notification.part_id)
            .and_then(|(marker_id, round_index)| {
                let mut occurrence = 0usize;
                items.iter().enumerate().find_map(|(index, item)| {
                    if item.id != Some(*marker_id) {
                        return None;
                    }
                    let is_target = occurrence == *round_index;
                    occurrence += 1;
                    is_target.then_some(index)
                })
            })
            .unwrap_or(items.len());
        let notification_id = notification.part_id;
        items.insert(
            insertion_index,
            WindowItem {
                id: Some(notification_id),
                run: project_completion_input(&[notification]),
            },
        );
    }
    items
}

fn checkpoint_recent_run(stored: &PromptCompactionMessage) -> CompletionInputRun {
    CompletionInputRun {
        role: stored.role,
        parts: vec![CompletionInputPart::Text {
            text: stored.text.clone(),
        }],
        provider_state: Default::default(),
    }
}

fn compaction_summary_run(session: &Session, summary: &str) -> CompletionInputRun {
    CompletionInputRun {
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

pub(crate) fn normalize_prompt_runs(messages: &[CompletionInputRun]) -> Vec<CompletionInputRun> {
    messages
        .iter()
        .filter(|run| run_has_visible_prompt_payload(run))
        .cloned()
        .collect()
}

fn prompt_runs_for_request(items: Vec<WindowItem>) -> Vec<WindowItem> {
    items
        .into_iter()
        .filter(|item| run_has_visible_prompt_payload(&item.run))
        .collect()
}

fn run_has_visible_prompt_payload(run: &CompletionInputRun) -> bool {
    run.parts.iter().any(prompt_part_has_visible_payload)
}

fn prompt_part_has_visible_payload(part: &CompletionInputPart) -> bool {
    match part {
        CompletionInputPart::Text { text } => !text.trim().is_empty(),
        CompletionInputPart::Reasoning { text } => !text.trim().is_empty(),
        CompletionInputPart::SystemMessage { text } => !text.trim().is_empty(),
        CompletionInputPart::Attachment { .. } => true,
        CompletionInputPart::ToolCall { .. } => true,
        CompletionInputPart::ToolResult { output_json, .. } => !output_json.trim().is_empty(),
    }
}

pub(crate) fn approximate_prompt_payload_chars(parts: &[Part]) -> usize {
    parts_into_runs(parts)
        .into_iter()
        .map(|run| {
            let projected_run = project_completion_input(&run);
            if !run_has_visible_prompt_payload(&projected_run) {
                return 0;
            }
            approximate_run_payload_chars(&projected_run)
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
    let prompt_runs = prompt_runs_for_request(window_items_from_parts(parts));
    let anchor_index = prompt_runs
        .iter()
        .position(|item| item.id == Some(assistant_message_id))?;
    if !runtime.transcript_digest.is_empty()
        && prompt_prefix_transcript_digest(prompt_runs.as_slice(), anchor_index)
            != runtime.transcript_digest
    {
        return None;
    }
    // The provider's previous response includes the request prefix, but not the
    // assistant output itself. Include the anchor response plus later deltas.
    let delta_chars: usize = prompt_runs[anchor_index..]
        .iter()
        .map(|item| approximate_run_payload_chars(&item.run))
        .sum();
    let delta_tokens = agena_runtime::estimate_prompt_tokens_from_chars(delta_chars);

    Some(PromptTokenEstimate {
        total_tokens: last_successful_prompt_tokens.saturating_add(delta_tokens),
        delta_tokens,
        delta_chars: delta_chars as u64,
    })
}

pub(crate) fn prompt_transcript_digest(parts: &[Part]) -> String {
    let normalized = prompt_runs_for_request(window_items_from_parts(parts));
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
    let transcript = runs_to_provider_transcript(&items[..end]);
    transcript.digest_hex()
}

fn runs_to_provider_transcript(items: &[WindowItem]) -> ProviderTranscript {
    let mut transcript = ProviderTranscript::new();
    for item in items {
        let run = &item.run;
        match run.role {
            Role::Assistant => {
                let mut content_blocks = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();
                let mut had_any = false;
                for part in &run.parts {
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
                        CompletionInputPart::SystemMessage { text } => {
                            had_any = true;
                            if !text.is_empty() {
                                content_blocks.push(TranscriptBlock::Text { text: text.clone() });
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
                    let fallback = run.as_text_lossy();
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
                for part in &run.parts {
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
                for part in &run.parts {
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
                    let fallback = run.as_text_lossy();
                    if !fallback.trim().is_empty() {
                        content_blocks.push(TranscriptBlock::Text { text: fallback });
                        had_any = true;
                    }
                }
                if !had_any {
                    continue;
                }
                let fragment = if matches!(run.role, Role::System) {
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
        CompletionInputAttachmentSource::LocalPath { path } => {
            format!("local_path:{}", path.trim())
        }
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
/// [`approximate_total_request_tokens`] over [`CompletionInputRun`]s.
pub(crate) fn approximate_request_tokens_from_runs(
    messages: &[CompletionInputRun],
    system: Option<&str>,
    tools: &[ToolApiBinding],
) -> u64 {
    let payload_chars = messages
        .iter()
        .map(approximate_run_payload_chars)
        .sum::<usize>();
    let total_chars =
        payload_chars.saturating_add(approximate_request_overhead_chars(system, tools));
    agena_runtime::estimate_prompt_tokens_from_chars(total_chars)
}

/// [`approximate_request_tokens_from_runs`] with native compaction payload.
pub(crate) fn approximate_request_tokens_from_runs_with_compaction(
    messages: &[CompletionInputRun],
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
        .map(approximate_run_payload_chars)
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
    let active_runs = prompt_window_items(
        session,
        Some(options.provider_id),
        options.adapter_id,
        Some(options.model_id),
        options.native_compaction_enabled,
    );
    let prompt_runs = prompt_runs_for_request(active_runs);
    let provider_request_shape = options.provider_request_shape.cloned();
    let PromptRequestFingerprint {
        system_fingerprint,
        request_options_fingerprint,
    } = prompt_request_fingerprints(&options);

    let continuation = evaluate_prompt_continuation(
        session,
        prompt_runs.as_slice(),
        &options,
        system_fingerprint.as_str(),
        request_options_fingerprint.as_str(),
    );

    let continuation_reason = match &continuation {
        PromptContinuationOutcome::Reuse { .. } => PromptContinuationReason::ProviderContinuation,
        PromptContinuationOutcome::Restart { reason, .. } => *reason,
    };
    let continuation_diagnostic = continuation.diagnostic();
    let (turns, previous_response_id, provider_compaction) = match continuation {
        PromptContinuationOutcome::Reuse {
            previous_response_id,
            delta_runs,
        } => (delta_runs, Some(previous_response_id), None),
        PromptContinuationOutcome::Restart { .. } => (
            prompt_runs.into_iter().map(|item| item.run).collect(),
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
        turns,
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
        delta_runs: Vec<CompletionInputRun>,
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
    prompt_runs: &[WindowItem],
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

    // With multi-round turns (one user message == one run marker) the anchor's
    // `assistant_message_id` (the run marker part id) is shared by every round
    // item of the turn. `.rposition` lands on the turn's *last* round so
    // `delta_runs` starts after the whole turn instead of re-sending rounds
    // 2..n as if they were new input.
    let Some(anchor_index) = prompt_runs
        .iter()
        .rposition(|item| item.id == Some(anchor.assistant_message_id))
    else {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::AnchorAssistantMissing,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    };

    if !anchor.transcript_digest.is_empty()
        && prompt_prefix_transcript_digest(prompt_runs, anchor_index) != anchor.transcript_digest
    {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::TranscriptDigestMismatch,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    let delta_runs = prompt_runs[anchor_index + 1..]
        .iter()
        .map(|item| item.run.clone())
        .collect::<Vec<_>>();
    if delta_runs.is_empty() {
        return PromptContinuationOutcome::Restart {
            reason: PromptContinuationReason::NoDeltaRuns,
            diagnostic: PromptContinuationDiagnostic::default(),
        };
    }

    PromptContinuationOutcome::Reuse {
        previous_response_id: anchor.previous_response_id.clone(),
        delta_runs,
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
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn provider_metadata_field<'a>(
    metadata: &'a serde_json::Value,
    field: &str,
) -> Option<&'a serde_json::Value> {
    metadata
        .as_object()
        .and_then(|object| object.get(field))
        .filter(|value| agena_provider::provider_metadata_value_is_meaningful(value))
        .or_else(|| {
            metadata
                .as_object()
                .and_then(|object| object.get("provider_metadata"))
                .and_then(serde_json::Value::as_object)
                .and_then(|object| object.get(field))
                .filter(|value| agena_provider::provider_metadata_value_is_meaningful(value))
        })
}

#[cfg(test)]
mod response_id_tests {
    use super::extract_response_id;

    #[test]
    fn blank_direct_response_id_does_not_mask_nested_anchor() {
        let metadata = serde_json::json!({
            "response_id": "   ",
            "provider_metadata": { "response_id": "  resp_nested  " }
        });
        assert_eq!(
            extract_response_id(Some(&metadata)).as_deref(),
            Some("resp_nested")
        );
    }
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

fn approximate_run_payload_chars(run: &CompletionInputRun) -> usize {
    run.parts
        .iter()
        .map(|part| match part {
            CompletionInputPart::Text { text }
            | CompletionInputPart::Reasoning { text }
            | CompletionInputPart::SystemMessage { text } => text.len(),
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
    use crate::session::model::{
        PromptCompactionContent, PromptCompactionMessage, PromptCompactionRuntime,
    };
    use crate::session::store::{run_marker_content, text_content, typed_content_to_value};
    use agena_domain::{MessageSource, PromptCompactionStrategy, PromptCompactionTrigger};
    use agena_provider::CompletionUsage;
    use agena_runtime_contracts::part_content::TypedContent;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
    use chrono::{DateTime, Utc};

    /// Build one run marker part (`part_id` = the durable message id) plus its
    /// text content part, the v2 parts shape the prompt path projects onto
    /// provider input messages via `project_completion_input`.
    fn run_parts(
        id: i64,
        role: PartRole,
        source: &str,
        text: &str,
        now: DateTime<Utc>,
    ) -> Vec<Part> {
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
                content: typed_content_to_value(&TypedContent::Text(text_content(text.to_owned())))
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
                content: typed_content_to_value(&TypedContent::Hook(
                    agena_runtime_contracts::part_content::HookContent {
                        hook: "agent.stop".to_owned(),
                        plugin_id: Some("agena.plan".to_owned()),
                        summary: "agent.stop hook blocked stop: workflow plan autorun".to_owned(),
                        detail: Some("continue with the next plan step".to_owned()),
                        message: None,
                        extra: Default::default(),
                    },
                ))
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
        parts.extend(run_parts(
            2,
            PartRole::Assistant,
            "tool",
            "old assistant",
            now,
        ));
        parts.push(compaction_marker(50, "durable state", now));
        parts.extend(run_parts(3, PartRole::User, "user", "future user", now));
        session.install_projected_parts(parts);
        session
    }

    fn text_lossy_list(runs: &[CompletionInputRun]) -> Vec<String> {
        runs.iter().map(|run| run.as_text_lossy()).collect()
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

        let active = compactable_prompt_runs(&session, Some("p"), None, Some("m"), false);
        let texts = text_lossy_list(&active);
        assert_eq!(texts.len(), 3);
        assert!(texts[0].contains("durable state"));
        assert_eq!(texts[1], "retained assistant");
        assert_eq!(texts[2], "future user");
        assert!(
            texts
                .iter()
                .all(|text| text != "old user" && text != "old assistant"),
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

        let prompt_runs = prompt_runs_for_request(window_items_from_parts(session.parts()));
        assert!(
            prompt_runs.iter().all(|item| item.id != Some(99)),
            "hook-only assistant message must not be sent to the model; got {:?}",
            prompt_runs
                .iter()
                .map(|item| (item.id, item.run.as_text_lossy()))
                .collect::<Vec<_>>()
        );
        // The base messages plus the trailing user message remain; the
        // hook-only assistant message and the checkpoint marker project to no
        // provider payload.
        assert_eq!(
            prompt_runs.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3), Some(100)]
        );
    }

    #[test]
    fn hook_message_is_projected_to_the_model_prompt_as_assistant_text() {
        let now = Utc::now();
        let mut session = session_with_checkpoint();

        // A stop hook that blocked the run carries its continuation in
        // `message`; that message must reach the model as assistant text on
        // the next run (this is how the workflow plan autorun continues).
        let mut parts = session.parts().to_vec();
        let mut marker = run_marker_content("execution", None, None, None, None);
        marker["source"] = serde_json::json!("system");
        let hook_content = agena_runtime_contracts::part_content::HookContent {
            hook: "agent.stop".to_owned(),
            plugin_id: Some("agena.plan".to_owned()),
            summary: "agent.stop hook blocked stop: workflow plan autorun".to_owned(),
            detail: None,
            message: Some(
                "<plan_context>continue with the next plan step</plan_context>".to_owned(),
            ),
            extra: Default::default(),
        };
        parts.push(Part {
            part_id: 99,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: marker,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 7,
            revision: 0,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        });
        parts.push(Part {
            part_id: 99 * 1000,
            kind: "hook".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: typed_content_to_value(&TypedContent::Hook(hook_content))
                .expect("hook content is always serializable"),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: Some(99),
            origin_session_id: 7,
            revision: 0,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        });
        session.install_projected_parts(parts);

        let prompt_runs = prompt_runs_for_request(window_items_from_parts(session.parts()));
        let hook_run = prompt_runs
            .iter()
            .find(|item| item.id == Some(99))
            .expect("the hook run with a message reaches the model prompt");
        assert_eq!(hook_run.run.role, agena_domain::Role::Assistant);
        assert!(
            hook_run
                .run
                .as_text_lossy()
                .contains("continue with the next plan step"),
            "the hook message is projected as assistant text: {}",
            hook_run.run.as_text_lossy()
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

        let matching = compactable_prompt_runs(
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

        let locally_compacted = compactable_prompt_runs(
            &session,
            Some("openai"),
            Some("responses"),
            Some("gpt"),
            false,
        );
        // With native compaction disabled the window is still the parts after
        // the checkpoint marker; the opaque checkpoint never replays as text.
        let projected = compactable_prompt_runs(&session, None, None, None, false);
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

        let wrong_adapter =
            compactable_prompt_runs(&session, Some("openai"), Some("chat"), Some("gpt"), true);
        assert_eq!(wrong_adapter.len(), 1);

        let switched =
            compactable_prompt_runs(&session, Some("anthropic"), None, Some("claude"), true);
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
        parts.extend(run_parts(
            2,
            PartRole::Assistant,
            "tool",
            "old assistant",
            now,
        ));
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
            text_lossy_list(
                &window_items_from_parts(window)
                    .into_iter()
                    .map(|item| item.run)
                    .collect::<Vec<_>>()
            ),
            vec!["future user".to_owned()]
        );
    }

    #[test]
    fn unhandled_delivery_is_pinned_across_compaction_until_a_round_receipt_exists() {
        let now = Utc::now();
        let mut session = Session::new(7, 11, "notification pin", now);
        let mut parts = run_parts(
            1,
            PartRole::Assistant,
            "tool",
            "launched background work",
            now,
        );
        parts.push(Part {
            part_id: 40,
            kind: "system_notification".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: serde_json::json!({
                "operation_id": "proc_compacted",
                "operation_kind": "shell",
                "status": "completed",
                "summary": "completed after compaction",
                "body": "completed after compaction",
                "delivery_protocol": "provider_round_v1"
            }),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: Some(1),
            origin_session_id: 7,
            revision: 1,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        });
        parts.push(compaction_marker(50, "durable state", now));
        parts.extend(run_parts(60, PartRole::User, "user", "future user", now));
        session.install_projected_parts(parts);

        let before_receipt = prompt_window_items(&session, None, None, None, false);
        assert!(
            matches!(
                before_receipt.last().and_then(|item| item.run.parts.first()),
                Some(CompletionInputPart::SystemMessage { text })
                    if text == "completed after compaction"
            ),
            "an unhandled notification remains the prompt tail after its launch marker was compacted"
        );
        assert_eq!(
            provider_visible_notification_part_ids(&session),
            vec![40],
            "the provider round durably records the pinned notification input"
        );

        let mut handled_marker = run_marker_content("continue", None, None, None, None);
        handled_marker["rounds"] = serde_json::json!([{
            "part_ids": [71000],
            "provider_state": null,
            "input_notification_part_ids": [40]
        }]);
        let mut handled_parts = session.parts().to_vec();
        handled_parts.push(Part {
            part_id: 71,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: handled_marker,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 7,
            revision: 1,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        });
        handled_parts.push(Part {
            part_id: 71000,
            kind: "text".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: typed_content_to_value(&TypedContent::Text(text_content(
                "notification handled".to_owned(),
            )))
            .expect("response text serializes"),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: Some(71),
            origin_session_id: 7,
            revision: 1,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        });
        session.install_projected_parts(handled_parts);
        assert!(provider_visible_notification_part_ids(&session).is_empty());
        let after_receipt = prompt_window_items(&session, None, None, None, false);
        assert!(
            after_receipt.iter().all(|item| item.run.parts.iter().all(|part| {
                !matches!(part, CompletionInputPart::SystemMessage { text } if text == "completed after compaction")
            })),
            "the exact notification is unpinned once a completed provider round receipts it"
        );
    }

    /// One multi-round assistant turn (one user message == one run marker):
    /// the marker carries `content["rounds"]` (part ids + per-round provider
    /// replay state). `window_items_from_parts` must re-split it into one wire
    /// message per round, each with its own parts and reasoning passback, while
    /// every item shares the single run marker's id.
    #[test]
    fn multi_round_turn_projects_one_wire_message_per_round_under_one_run_id() {
        let now = Utc::now();
        let mut marker = run_marker_content("continue", None, None, None, None);
        marker["rounds"] = serde_json::json!([
            {
                "part_ids": [1001, 1002],
                "provider_state": { "openai_reasoning_items": [{"type": "reasoning", "id": "r1"}] }
            },
            {
                "part_ids": [2001, 2002, 2003],
                "input_notification_part_ids": [1500],
                "provider_state": { "openai_reasoning_items": [{"type": "reasoning", "id": "r2"}] }
            }
        ]);
        let run_marker = Part {
            part_id: 999,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: marker,
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
        };
        let content_part = |id: i64, text: &str| Part {
            part_id: id,
            kind: "text".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: typed_content_to_value(&TypedContent::Text(text_content(text.to_owned())))
                .expect("text content is always serializable"),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: Some(999),
            origin_session_id: 7,
            revision: 0,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: None,
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        };
        let notification_part = Part {
            part_id: 1500,
            kind: "system_notification".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: typed_content_to_value(&TypedContent::SystemNotification(
                agena_runtime_contracts::part_content::SystemNotificationContent {
                    operation_id: "proc_mid_round".to_owned(),
                    operation_kind: "shell".to_owned(),
                    status: "completed".to_owned(),
                    summary: "background command completed".to_owned(),
                    body: "background command completed".to_owned(),
                    ..Default::default()
                },
            ))
            .expect("notification content is always serializable"),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: Some(999),
            origin_session_id: 7,
            revision: 0,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        };

        let parts = vec![
            run_marker,
            content_part(1001, "round one thinking"),
            content_part(1002, "round one call"),
            notification_part,
            content_part(2001, "round two thinking"),
            content_part(2002, "round two call"),
            content_part(2003, "round two final"),
        ];
        let items = window_items_from_parts(&parts);
        assert_eq!(
            items.len(),
            3,
            "two provider rounds plus the Assistant hook between them, got {}",
            items.len()
        );
        // Both provider rounds map back to the single run marker id; the hook
        // keeps its own ordering anchor in the provider timeline.
        assert_eq!(items[0].id, Some(999));
        assert_eq!(items[1].id, Some(1500));
        assert_eq!(items[2].id, Some(999));
        // Round one carries exactly its own parts and its own reasoning replay.
        assert_eq!(
            items[0]
                .run
                .parts
                .iter()
                .filter_map(|part| match part {
                    CompletionInputPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["round one thinking", "round one call"]
        );
        assert_eq!(
            items[0].run.provider_state.openai_reasoning_items.len(),
            1,
            "round one replays its own reasoning"
        );
        assert!(
            matches!(
                items[1].run.parts.as_slice(),
                [CompletionInputPart::SystemMessage { text }]
                    if text == "background command completed"
            ),
            "the hook seen by round two is ordered after round one and before round two output"
        );
        // Round two carries its own parts and its own (different) reasoning.
        assert_eq!(
            items[2]
                .run
                .parts
                .iter()
                .filter_map(|part| match part {
                    CompletionInputPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["round two thinking", "round two call", "round two final"]
        );
        assert_eq!(
            items[2].run.provider_state.openai_reasoning_items[0]["id"],
            serde_json::json!("r2"),
            "round two replays its own reasoning"
        );
    }

    /// A run marker without `rounds` (single-round turns, legacy rows, user /
    /// hook / execution runs) projects as one item exactly as before.
    #[test]
    fn single_round_or_legacy_run_projects_one_item_without_round_splitting() {
        let now = Utc::now();
        let parts = run_parts(7, PartRole::Assistant, "tool", "plain assistant", now);
        let items = window_items_from_parts(&parts);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, Some(7));
        assert_eq!(
            text_lossy_list(&[items[0].run.clone()]),
            vec!["plain assistant".to_owned()]
        );
    }
}
