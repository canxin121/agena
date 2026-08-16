//! Provider-agnostic wire representation of a single chat message.
//!
//! [`WirePart`] is the normalised, provider-ready view of persisted session
//! parts ([`agena_storage::store::Part`]). Callers obtain it via
//! [`project_session_parts`] / [`project_completion_input`] and then map it to
//! whatever payload format their provider expects.
//!
//! The projection step handles all concerns that are shared across every
//! provider:
//!   - stripping UI-only content (file changes, permission requests, …)
//!   - resolving the tool-call ID from the operation's stashed provider id
//!     with fallback to the operation's numeric call id
//!   - carrying operation outputs through one provider-neutral projection path
//!   - emitting an empty output for still-pending / in-progress tool executions

use base64::Engine as _;
use serde::Deserialize;

use crate::ProviderError;
use agena_domain::{ExecutionStatus, ReasoningPart, Role, ToolApiFunction, ToolInvocation};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentKind, CompletionInputAttachmentSource,
    CompletionInputPart, CompletionInputProviderState, CompletionInputRun, ModelToolFunction,
    normalize_optional_text, provider_metadata_value_is_meaningful,
};
use agena_runtime_contracts::part::{
    AttachmentItem, AttachmentKind, AttachmentSource, OperationPart,
};
use agena_runtime_contracts::part_content::{
    ThinkContent, TypedContent, attachment_from_file_ref, decode, operation_from_tool_call,
    skill_reference_from_skill_ref,
};
use agena_runtime_contracts::provider_state::PartProviderState;
use agena_storage::store::{Part, PartRole};

// ─── Runtime-private type ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
/// A wire-level message part.
pub enum WirePart {
    Text {
        text: String,
    },
    /// Assistant reasoning preserved for replay. Kept distinct from [`Self::Text`]
    /// so providers that require a dedicated `reasoning_content` field can
    /// reconstruct it, instead of silently dropping the reasoning on projection.
    Reasoning {
        text: String,
    },
    /// A system-authored notice (the projection of a background-operation
    /// notification). Kept distinct from [`Self::Text`] so a notification can
    /// never be mistaken for the assistant's own reply — the role leak that
    /// surfaced notification JSON as assistant output. Adapters render it as a
    /// mid-conversation `system` message where the protocol supports one.
    SystemMessage {
        text: String,
    },
    Attachment {
        item: AttachmentItem,
    },
    ToolCall {
        id: String,
        function: ModelToolFunction,
        arguments_json: String,
    },
    ToolResult {
        tool_call_id: String,
        /// Tool API function that produced this result. Gemini requires it for
        /// `functionResponse`; OpenAI and Anthropic identify results by call id.
        function: ModelToolFunction,
        arguments_json: String,
        status: agena_provider::CompletionInputToolResultStatus,
        output_json: String,
    },
}

impl WirePart {
    pub fn as_text_lossy(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Reasoning { text } => text.clone(),
            Self::SystemMessage { text } => text.clone(),
            Self::Attachment { item } => hint_text(item),
            Self::ToolCall { id, function, .. } => {
                format!("[tool_call:{}:{id}]", function.function_name())
            }
            Self::ToolResult { tool_call_id, .. } => format!("[tool_result:{tool_call_id}]"),
        }
    }
}

// ─── Projection ───────────────────────────────────────────────────────────────

/// Normalise a [`CompletionInputRun`] into a flat list of provider-ready
/// [`WirePart`]s. Project provider-owned input into the adapter-neutral
/// wire-part view.
pub fn project(run: &CompletionInputRun) -> Vec<WirePart> {
    run.parts
        .iter()
        .cloned()
        .map(wire_part_from_completion_input)
        .collect()
}

/// Decode a storage [`Part`] into the typed [`TypedContent`] the projection
/// operates on. A part that fails to decode (unknown kind or non-object
/// payload) projects to nothing — the same "reload 宁缺勿崩" principle the
/// session reload path applies.
fn projected_content(part: &Part) -> Option<TypedContent> {
    decode(&part.kind, &part.content).ok()
}

/// Resolve the stable provider-visible call id for a stored tool part.
///
/// [`agena_storage::store::Part`] has no `operation_id` column; the session
/// serialization stashes the provider operation id under
/// `agena.operation_id` inside the rich [`OperationPart`] metadata (see
/// `agena-runtime-session` `serialize_part_content`). When that stash is
/// missing we fall back to the operation's numeric call id, matching the v1
/// projection's `part.operation_id` → `exec.call_id()` fallback.
fn project_operation_call_id(exec: &OperationPart) -> String {
    exec.metadata
        .get("agena.operation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| exec.call_id().to_string())
}

// ─── Rich-content recovery (v1 payloads from typed shapes) ────────────────────
//
// The v1 payload structs (`OperationPart`, `AttachmentPart`, `SkillReferencePart`,
// `ReasoningPart`) survive the T8 migration — they ride losslessly in the typed
// content's `extra` bucket and are recovered through the public extractor
// helpers in `agena_runtime_contracts::part_content`
// (`operation_from_tool_call`, `attachment_from_file_ref`,
// `skill_reference_from_skill_ref`, …).

/// Rebuild the reasoning text the v1 [`ReasoningPart`] prefers from the
/// canonical `think` shape (summary wins, raw content otherwise).
fn reasoning_preferred_text(think: &ThinkContent) -> String {
    ReasoningPart {
        summary: think.summary.clone(),
        raw_content: think.raw.clone(),
        encrypted_content: think.encrypted_content.clone(),
    }
    .preferred_text()
}

/// Project a persisted part slice at the session/core boundary.
///
/// Consumes storage [`Part`]s directly (R6-T5); each part is decoded to its
/// typed [`TypedContent`] and projected exactly as the legacy message
/// projection did, so the wire output is unchanged. Run markers and
/// provider-only operations are skipped; the coarse `state` column drives
/// result emission with the fine-grained denial outcomes recovered from the
/// rich content.
pub fn project_persisted(parts: &[Part]) -> Vec<WirePart> {
    let mut wire: Vec<WirePart> = Vec::new();

    for part in parts {
        if part.is_run_marker() {
            continue;
        }
        let Some(content) = projected_content(part) else {
            continue;
        };
        let role = role_from_part_role(part.role);

        match content {
            TypedContent::Text(text) => {
                if !text.text.is_empty() {
                    wire.push(WirePart::Text {
                        text: text.text.clone(),
                    });
                }
            }
            // The v1 fold degraded Run/PasteRef/ToolResult/Compaction to plain
            // text, which the Text arm then projected as text when non-empty —
            // preserve that wire output exactly.
            TypedContent::PasteRef(paste) => {
                if !paste.text.is_empty() {
                    wire.push(WirePart::Text {
                        text: paste.text.clone(),
                    });
                }
            }
            TypedContent::ToolResult(tool_result) => {
                if !tool_result.output.is_empty() {
                    wire.push(WirePart::Text {
                        text: tool_result.output.clone(),
                    });
                }
            }
            TypedContent::Compaction(compaction) => {
                if let Some(summary) = compaction.summary.as_deref().filter(|s| !s.is_empty()) {
                    wire.push(WirePart::Text {
                        text: summary.to_owned(),
                    });
                }
            }
            // Run markers are filtered above; the fold degraded Run to empty
            // text, so nothing projects here.
            TypedContent::Run(_) => {}
            TypedContent::FileRef(file_ref) => {
                for item in &attachment_from_file_ref(&file_ref).attachments {
                    wire.push(WirePart::Attachment { item: item.clone() });
                }
            }
            TypedContent::SkillRef(skill_ref) => {
                let skill_reference = skill_reference_from_skill_ref(&skill_ref);
                if !skill_reference.skills.is_empty() {
                    wire.push(WirePart::Text {
                        text: skill_reference.model_context_text(),
                    });
                }
            }
            TypedContent::ToolCall(tool_call) => {
                let exec = operation_from_tool_call(&tool_call);
                if exec.is_provider_only() {
                    continue;
                }
                let call_id = project_operation_call_id(&exec);
                let status = operation_status(&exec);

                let Some((function, arguments_json)) = invocation_name_and_args(exec.invocation())
                else {
                    continue;
                };
                if matches!(role, Role::Tool) {
                    if is_terminal_result_status(status) {
                        wire.push(WirePart::ToolResult {
                            tool_call_id: call_id,
                            function,
                            arguments_json,
                            status: completion_input_result_status(status),
                            output_json: project_operation_output(status, &exec),
                        });
                    }
                    continue;
                }
                wire.push(WirePart::ToolCall {
                    id: call_id.clone(),
                    function: function.clone(),
                    arguments_json: arguments_json.clone(),
                });

                // Every persisted assistant tool call projects its paired
                // result — even a call whose turn was interrupted before the
                // execution settled (Pending/InProgress). A provider-visible
                // `function_call` with no matching `function_call_output` is
                // rejected by gateways that translate the Responses input into
                // Chat messages ("No tool output found for tool call N"), so
                // an interrupted call must replay as a closed call/result pair
                // rather than an orphaned call.
                wire.push(WirePart::ToolResult {
                    tool_call_id: call_id,
                    function,
                    arguments_json,
                    status: completion_input_result_status(status),
                    output_json: project_operation_output(status, &exec),
                });
            }
            TypedContent::Think(think) => {
                let text = reasoning_preferred_text(&think);
                if !text.is_empty() {
                    wire.push(WirePart::Reasoning { text });
                }
            }
            TypedContent::Hook(hook) => {
                // A hook that blocked the stop carries its continuation in
                // `message` (for example the workflow plan autorun's
                // `<plan_context>` block). The continuation is System-originated
                // (the agent.stop plugin's `continue_with_message`), so it must
                // never project as the assistant's own reply text — that role
                // leak is exactly how runtime content surfaces as the model's
                // visible output. Project it as a dedicated SystemMessage wire
                // part, which adapters deliver as a mid-conversation
                // system/notice message. The next model turn still sees it: both
                // Text and SystemMessage map to `TranscriptBlock::Text` in
                // `runs_to_provider_transcript` (prompt_window.rs).
                if let Some(message) = hook.message.as_deref().filter(|text| !text.is_empty()) {
                    wire.push(WirePart::SystemMessage {
                        text: message.to_owned(),
                    });
                }
            }
            // A background-operation completion/event notification (the agena
            // analog of Claude's `<task-notification>`). It stays appended onto
            // the launching assistant run (no new run marker), but must never
            // project as the assistant's own reply text — that role leak is
            // exactly how notification JSON surfaced as the model's visible
            // output. Project it as a dedicated SystemMessage wire part, which
            // adapters deliver as a mid-conversation system/notice message;
            // `body` carries the full `<agena_notification>…</agena_notification>`
            // payload the model is taught to recognize (§3).
            TypedContent::SystemNotification(notification) => {
                if !notification.body.trim().is_empty() {
                    wire.push(WirePart::SystemMessage {
                        text: notification.body.clone(),
                    });
                }
            }
            TypedContent::Notice(_) | TypedContent::Interaction(_) | TypedContent::Error(_) => {}
        }
    }

    wire
}

fn wire_part_from_completion_input(part: CompletionInputPart) -> WirePart {
    match part {
        CompletionInputPart::Text { text } => WirePart::Text { text },
        CompletionInputPart::Reasoning { text } => WirePart::Reasoning { text },
        CompletionInputPart::SystemMessage { text } => WirePart::SystemMessage { text },
        CompletionInputPart::Attachment { attachment } => WirePart::Attachment {
            item: attachment_item_from_completion_input(attachment),
        },
        CompletionInputPart::ToolCall {
            id,
            function,
            arguments_json,
        } => WirePart::ToolCall {
            id,
            function,
            arguments_json,
        },
        CompletionInputPart::ToolResult {
            tool_call_id,
            function,
            arguments_json,
            status,
            output_json,
        } => WirePart::ToolResult {
            tool_call_id,
            function,
            arguments_json,
            status,
            output_json,
        },
    }
}

fn attachment_item_from_completion_input(attachment: CompletionInputAttachment) -> AttachmentItem {
    AttachmentItem {
        kind: match attachment.kind {
            CompletionInputAttachmentKind::Image => AttachmentKind::Image,
            CompletionInputAttachmentKind::Audio => AttachmentKind::Audio,
            CompletionInputAttachmentKind::Video => AttachmentKind::Video,
            CompletionInputAttachmentKind::Pdf => AttachmentKind::Pdf,
            CompletionInputAttachmentKind::File => AttachmentKind::File,
        },
        mime: attachment.mime,
        source: match attachment.source {
            CompletionInputAttachmentSource::Url { url } => AttachmentSource::Url { url },
            CompletionInputAttachmentSource::DataUrl { url } => AttachmentSource::DataUrl { url },
            CompletionInputAttachmentSource::Base64 { data } => AttachmentSource::Base64 { data },
            CompletionInputAttachmentSource::FileId { id } => {
                AttachmentSource::FileId { file_id: id }
            }
            CompletionInputAttachmentSource::LocalPath { path } => {
                AttachmentSource::LocalPath { path }
            }
        },
        filename: attachment.filename,
        title: attachment.title,
        size_bytes: attachment.size_bytes,
        sha256: attachment.sha256,
        width: attachment.width,
        height: attachment.height,
        duration_ms: attachment.duration_ms,
        page_count: attachment.page_count,
    }
}

/// Project a persisted run group into the provider-owned completion input
/// contract. `parts` is one run's parts — a run marker plus its content
/// parts (or a bare content part for singletons). Role and provider replay
/// state are read from the run marker when present, matching how the session
/// groups storage parts into logical messages. Runtime-only interaction and
/// error Activities are excluded; all provider-visible parts and replay state
/// are retained.
pub fn project_completion_input(parts: &[Part]) -> CompletionInputRun {
    let marker = parts.iter().find(|part| part.is_run_marker());
    let role = marker
        .map(|part| role_from_part_role(part.role))
        .or_else(|| parts.first().map(|part| role_from_part_role(part.role)))
        .unwrap_or(Role::User);
    CompletionInputRun {
        role,
        parts: project_persisted(parts)
            .into_iter()
            .map(completion_input_part_from_wire)
            .collect(),
        provider_state: marker
            .and_then(|part| part.provider_state.as_ref())
            .and_then(completion_input_provider_state)
            .unwrap_or_default(),
    }
}

pub fn completion_input_part_from_wire(part: WirePart) -> CompletionInputPart {
    match part {
        WirePart::Text { text } => CompletionInputPart::Text { text },
        WirePart::Reasoning { text } => CompletionInputPart::Reasoning { text },
        WirePart::SystemMessage { text } => CompletionInputPart::SystemMessage { text },
        WirePart::Attachment { item } => CompletionInputPart::Attachment {
            attachment: completion_input_attachment(item),
        },
        WirePart::ToolCall {
            id,
            function,
            arguments_json,
        } => CompletionInputPart::ToolCall {
            id,
            function,
            arguments_json,
        },
        WirePart::ToolResult {
            tool_call_id,
            function,
            arguments_json,
            status,
            output_json,
        } => CompletionInputPart::ToolResult {
            tool_call_id,
            function,
            arguments_json,
            status,
            output_json,
        },
    }
}

fn completion_input_attachment(item: AttachmentItem) -> CompletionInputAttachment {
    CompletionInputAttachment {
        kind: match item.kind {
            AttachmentKind::Image => CompletionInputAttachmentKind::Image,
            AttachmentKind::Audio => CompletionInputAttachmentKind::Audio,
            AttachmentKind::Video => CompletionInputAttachmentKind::Video,
            AttachmentKind::Pdf => CompletionInputAttachmentKind::Pdf,
            AttachmentKind::File => CompletionInputAttachmentKind::File,
        },
        mime: item.mime,
        source: match item.source {
            AttachmentSource::Url { url } => CompletionInputAttachmentSource::Url { url },
            AttachmentSource::DataUrl { url } => CompletionInputAttachmentSource::DataUrl { url },
            AttachmentSource::Base64 { data } => CompletionInputAttachmentSource::Base64 { data },
            AttachmentSource::FileId { file_id } => {
                CompletionInputAttachmentSource::FileId { id: file_id }
            }
            AttachmentSource::LocalPath { path } => {
                CompletionInputAttachmentSource::LocalPath { path }
            }
        },
        filename: item.filename,
        title: item.title,
        size_bytes: item.size_bytes,
        sha256: item.sha256,
        width: item.width,
        height: item.height,
        duration_ms: item.duration_ms,
        page_count: item.page_count,
    }
}

pub fn completion_input_provider_state(
    value: &serde_json::Value,
) -> Option<CompletionInputProviderState> {
    let mut state: CompletionInputProviderState =
        serde_json::from_value::<PartProviderState>(value.clone())
            .ok()?
            .into();
    state.response_id = normalize_optional_text(state.response_id);
    state.gemini_thought_signatures = std::mem::take(&mut state.gemini_thought_signatures)
        .into_iter()
        .filter_map(|(call_id, signature)| {
            let call_id = normalize_optional_text(Some(call_id))?;
            (!signature.trim().is_empty()).then_some((call_id, signature))
        })
        .collect();
    state.copilot_reasoning_opaque = state
        .copilot_reasoning_opaque
        .filter(|value| !value.trim().is_empty());
    state.openai_chat_reasoning_details = state
        .openai_chat_reasoning_details
        .filter(provider_metadata_value_is_meaningful);
    state
        .anthropic_thinking_blocks
        .retain(provider_metadata_value_is_meaningful);
    state
        .openai_reasoning_items
        .retain(provider_metadata_value_is_meaningful);
    (!state.is_empty()).then_some(state)
}

/// Enforce the boundary between Tool API functions and execution tools before
/// any adapter serializes a request. Every replayed operation must be a known
/// Tool API function; execution-tool names and internal keys are never
/// provider function calls.
pub fn validate_provider_native_tool_input_history(
    _runs: &[CompletionInputRun],
) -> Result<(), ProviderError> {
    // A `CompletionInputPart::ToolCall`/`ToolResult` already carries the
    // closed `ToolApiFunction` identity. Runtime validates dynamic operation
    // invocations before it projects persisted history into this contract.
    Ok(())
}

#[cfg(test)]
pub fn validate_provider_native_tool_history(parts: &[Part]) -> Result<(), ProviderError> {
    for (part_index, part) in parts.iter().enumerate() {
        let Some(content) = projected_content(part) else {
            continue;
        };
        let TypedContent::ToolCall(tool_call) = &content else {
            continue;
        };
        let operation = operation_from_tool_call(tool_call);
        if operation.is_provider_only() {
            continue;
        }
        model_tool_function_for_invocation(operation.invocation()).map_err(|reason| {
            ProviderError::Internal(format!(
                "invalid provider tool history at parts[{part_index}]: {reason}"
            ))
        })?;
    }
    Ok(())
}

/// Like [`project`] but returns a single lossy string — used when the provider
/// only needs plain text (e.g. system messages for non-multimodal endpoints).
pub fn project_text_lossy(run: &CompletionInputRun) -> String {
    let parts = project(run);
    if parts.is_empty() {
        run.as_text_lossy()
    } else {
        parts_text_lossy(parts.as_slice())
    }
}

/// Runtime-private lossy projection used while preparing the persisted prompt window.
pub fn project_persisted_text_lossy(parts: &[Part]) -> String {
    let projected = project_persisted(parts);
    if projected.is_empty() {
        parts_as_text_lossy(parts)
    } else {
        parts_text_lossy(projected.as_slice())
    }
}

// ─── Part helpers ─────────────────────────────────────────────────────────────

/// Map a storage part role onto the domain role, matching the session
/// reload path (`role_from_part_role`).
fn role_from_part_role(role: PartRole) -> Role {
    match role {
        PartRole::User => Role::User,
        PartRole::Assistant => Role::Assistant,
        PartRole::System => Role::System,
        PartRole::Tool => Role::Tool,
        PartRole::Runtime => Role::System,
    }
}

/// Recover the fine-grained execution status for a stored tool part. The
/// coarse `state` column cannot express denial outcomes; the rich operation
/// content carries them (matching the session reload path
/// `part_to_message_part`, which also prefers `operation.status()`).
fn operation_status(exec: &OperationPart) -> ExecutionStatus {
    exec.status()
}

/// Whether a tool execution status has settled enough to emit a provider
/// [`WirePart::ToolResult`] — every status except still-pending/in-progress.
fn is_terminal_result_status(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Completed
            | ExecutionStatus::PolicyDenied
            | ExecutionStatus::UserDeclined
            | ExecutionStatus::CapabilityUnavailable
            | ExecutionStatus::ToolUnavailable
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled
    )
}

/// Best-effort textual rendering of a part slice for the empty-projection
/// fallback, mirroring the legacy `Message::as_text_lossy` semantics over
/// storage parts (run markers excluded; hook/notice/interaction/error parts
/// contribute their human-facing summary).
fn parts_as_text_lossy(parts: &[Part]) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        if part.is_run_marker() {
            continue;
        }
        let Some(content) = projected_content(part) else {
            continue;
        };
        let text = match content {
            TypedContent::Text(text) => Some(text.text.clone()),
            // The v1 fold degraded these kinds to plain text, which the Text
            // arm then rendered — preserve that output exactly.
            TypedContent::PasteRef(paste) => Some(paste.text.clone()),
            TypedContent::ToolResult(tool_result) => Some(tool_result.output.clone()),
            TypedContent::Compaction(compaction) => {
                Some(compaction.summary.clone().unwrap_or_default())
            }
            TypedContent::Run(_) => None,
            TypedContent::Think(think) => {
                let text = reasoning_preferred_text(&think);
                (!text.is_empty()).then_some(text)
            }
            TypedContent::SkillRef(skill_ref) => {
                Some(skill_reference_from_skill_ref(&skill_ref).model_context_text())
            }
            TypedContent::ToolCall(tool_call) => {
                operation_text_lossy(&operation_from_tool_call(&tool_call))
            }
            TypedContent::FileRef(_)
            | TypedContent::Notice(_)
            | TypedContent::Interaction(_)
            | TypedContent::Error(_) => part.summary.clone(),
            TypedContent::Hook(hook) => hook
                .message
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| part.summary.clone()),
            TypedContent::SystemNotification(notification) => {
                let body = notification.body.trim();
                if !body.is_empty() {
                    Some(body.to_owned())
                } else {
                    Some(notification.summary.clone())
                }
            }
        };
        if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
            out.push(text);
        }
    }
    out.join("\n")
}

/// Best-effort textual rendering of an operation for [`parts_as_text_lossy`],
/// mirroring the legacy `Message::as_text_lossy` `tool_text_lossy` helper.
fn operation_text_lossy(operation: &OperationPart) -> Option<String> {
    let candidates = [
        operation.output_text(),
        operation.error_message(),
        operation.title(),
        (!operation.summary.trim().is_empty()).then_some(operation.summary.as_str()),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

pub fn parts_text_lossy(parts: &[WirePart]) -> String {
    parts
        .iter()
        .map(WirePart::as_text_lossy)
        .collect::<Vec<_>>()
        .join("")
}

// ─── Attachment helpers ───────────────────────────────────────────────────────

pub fn hint_text(item: &AttachmentItem) -> String {
    let label = item.summary_label();
    match item.kind {
        AttachmentKind::Image => format!("[image:{label}]"),
        AttachmentKind::Audio => format!("[audio:{label}]"),
        AttachmentKind::Video => format!("[video:{label}]"),
        AttachmentKind::Pdf => format!("[document:{label}]"),
        AttachmentKind::File => format!("[file:{label}]"),
    }
}

pub fn filename(item: &AttachmentItem) -> Option<&str> {
    item.filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn data_url(item: &AttachmentItem) -> Option<String> {
    match &item.source {
        AttachmentSource::DataUrl { url } => {
            let trimmed = url.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        }
        AttachmentSource::Base64 { data } => {
            let mime = item.mime.trim();
            let data = data.trim();
            if mime.is_empty() || data.is_empty() {
                None
            } else {
                Some(format!("data:{mime};base64,{data}"))
            }
        }
        AttachmentSource::Url { .. }
        | AttachmentSource::FileId { .. }
        | AttachmentSource::LocalPath { .. } => None,
    }
}

pub fn media_url(item: &AttachmentItem) -> Option<String> {
    match &item.source {
        AttachmentSource::Url { url } | AttachmentSource::DataUrl { url } => {
            Some(url.trim().to_owned())
        }
        AttachmentSource::Base64 { data } => {
            if item.mime.trim().is_empty() || data.trim().is_empty() {
                None
            } else {
                Some(format!("data:{};base64,{}", item.mime.trim(), data.trim()))
            }
        }
        AttachmentSource::FileId { .. } | AttachmentSource::LocalPath { .. } => None,
    }
}

pub fn base64_with_mime(item: &AttachmentItem) -> Option<(String, String)> {
    match &item.source {
        AttachmentSource::Base64 { data } => {
            let mime = item.mime.trim();
            let data = data.trim();
            if mime.is_empty() || data.is_empty() {
                None
            } else {
                Some((mime.to_owned(), data.to_owned()))
            }
        }
        AttachmentSource::DataUrl { url } => {
            let (detected_mime, data) = parse_data_url(url)?;
            let mime = if detected_mime.trim().is_empty() {
                item.mime.trim()
            } else {
                detected_mime.trim()
            };
            if mime.is_empty() || data.is_empty() {
                None
            } else {
                Some((mime.to_owned(), data))
            }
        }
        AttachmentSource::Url { .. }
        | AttachmentSource::FileId { .. }
        | AttachmentSource::LocalPath { .. } => None,
    }
}

pub fn attachment_text(item: &AttachmentItem) -> Option<String> {
    let mime = item.mime.trim().to_ascii_lowercase();
    let is_text_like = mime.starts_with("text/")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/javascript"
        );
    if !is_text_like {
        return None;
    }

    let bytes = match &item.source {
        AttachmentSource::Base64 { data } => base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .ok()?,
        AttachmentSource::DataUrl { url } => {
            let (_, encoded) = url.split_once(',')?;
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()?
        }
        AttachmentSource::Url { .. }
        | AttachmentSource::FileId { .. }
        | AttachmentSource::LocalPath { .. } => return None,
    };

    String::from_utf8(bytes).ok()
}

/// Serialize one [`AttachmentItem`] to an OpenAI Chat content-part JSON value.
pub fn attachment_to_openai_content_value(item: &AttachmentItem) -> serde_json::Value {
    match item.kind {
        AttachmentKind::Image => media_url(item)
            .map(|url| serde_json::json!({ "type": "image_url", "image_url": { "url": url } }))
            .unwrap_or_else(|| serde_json::json!({ "type": "text", "text": hint_text(item) })),
        AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf
        | AttachmentKind::File => attachment_file_content_value(item)
            .unwrap_or_else(|| serde_json::json!({ "type": "text", "text": hint_text(item) })),
    }
}

/// Serialize a slice of [`WirePart`]s to an OpenAI Chat `content` array value.
pub fn parts_to_openai_content_array(parts: &[WirePart]) -> serde_json::Value {
    let items = parts
        .iter()
        .map(|part| match part {
            WirePart::Text { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            WirePart::Reasoning { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            WirePart::SystemMessage { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            WirePart::Attachment { item } => attachment_to_openai_content_value(item),
            WirePart::ToolCall { function, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_call:{}]", function.function_name()) })
            }
            WirePart::ToolResult { tool_call_id, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
            }
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(items)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

fn invocation_name_and_args(invocation: &ToolInvocation) -> Option<(ModelToolFunction, String)> {
    let function = model_tool_function_for_invocation(invocation).ok()?;
    let json_value: serde_json::Value = invocation
        .tool_api_call
        .as_ref()
        .map(|call| call.arguments.clone())?
        .into();
    Some((
        function,
        serde_json::to_string(&json_value).unwrap_or_else(|_| "{}".to_owned()),
    ))
}

pub fn tool_api_function_for_invocation(
    invocation: &ToolInvocation,
) -> Result<ToolApiFunction, String> {
    let stored_name = invocation.name.as_str();
    if let Some(call) = invocation.tool_api_call.as_ref() {
        if call.function == ToolApiFunction::Call {
            let target = call
                .arguments
                .get("tool")
                .and_then(agena_domain::StructuredValue::as_text)
                .map(str::trim)
                .filter(|target| !target.is_empty());
            if target == Some(stored_name) {
                return Ok(call.function);
            }
            return Err(format!(
                "Tool API function `tools_call` targets {target:?}, but the operation stores execution target `{stored_name}`"
            ));
        }
        if stored_name == call.function.function_name() && invocation.plugin_name.is_none() {
            return Ok(call.function);
        }
        return Err(format!(
            "Tool API function `{}` must store its exact protocol handler name and no plugin identity, but the operation stores name `{stored_name}` with plugin {:?}",
            call.function.function_name(),
            invocation.plugin_name,
        ));
    }

    Err(format!(
        "invocation `{stored_name}` has no explicit Tool API function identity"
    ))
}

pub(crate) fn model_tool_function_for_invocation(
    invocation: &ToolInvocation,
) -> Result<ModelToolFunction, String> {
    tool_api_function_for_invocation(invocation).map(Into::into)
}

pub fn project_operation_output(status: ExecutionStatus, exec: &OperationPart) -> String {
    match status {
        // A call whose turn was interrupted before the execution settled. The
        // result must still carry a non-empty output: gateways that translate
        // the Responses input into Chat messages reject an empty tool message,
        // and a placeholder tells the model the call never completed instead
        // of pretending it produced nothing.
        ExecutionStatus::Pending | ExecutionStatus::InProgress => {
            "Tool call was interrupted before producing a result.".to_string()
        }
        ExecutionStatus::Cancelled => {
            "Tool execution was cancelled before producing a result.".to_owned()
        }
        ExecutionStatus::Completed
        | ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => {
            if exec.result.managed_outputs.is_empty() && !exec.details.is_model_truncated() {
                return structured_operation_output(exec)
                    .or_else(|| generic_structured_operation_output(exec))
                    .unwrap_or_else(|| exec.output_text().unwrap_or_default().to_string());
            }
            managed_operation_output(exec).unwrap_or_else(|| {
                structured_operation_output(exec)
                    .or_else(|| generic_structured_operation_output(exec))
                    .unwrap_or_else(|| exec.output_text().unwrap_or_default().to_string())
            })
        }
        ExecutionStatus::Failed => exec
            .output_text()
            .or_else(|| exec.error_message())
            .unwrap_or_default()
            .to_string(),
    }
}

fn completion_input_result_status(
    status: ExecutionStatus,
) -> agena_provider::CompletionInputToolResultStatus {
    match status {
        ExecutionStatus::Completed => agena_provider::CompletionInputToolResultStatus::Completed,
        // Policy denial and a user's explicit decline are successful protocol
        // completions: the tool did not run, but the model must receive the
        // structured non-execution result and continue normally.
        ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => {
            agena_provider::CompletionInputToolResultStatus::Completed
        }
        ExecutionStatus::Failed => agena_provider::CompletionInputToolResultStatus::Failed,
        ExecutionStatus::Cancelled | ExecutionStatus::Pending | ExecutionStatus::InProgress => {
            agena_provider::CompletionInputToolResultStatus::Cancelled
        }
    }
}

const MAX_MODEL_WEB_RESULT_SNIPPET_CHARS: usize = 400;
const MAX_MODEL_WEB_CRAWL_DOCUMENTS: usize = 20;
const MAX_MODEL_WEB_CRAWL_FAILURES: usize = 5;

fn structured_operation_output(exec: &OperationPart) -> Option<String> {
    structured_web_search_output(exec).or_else(|| structured_web_crawl_output(exec))
}

fn generic_structured_operation_output(exec: &OperationPart) -> Option<String> {
    let payload = exec
        .result
        .structured
        .clone()
        .or_else(|| exec.details.to_json_payload())?;
    serde_json::to_string(&payload).ok()
}

fn managed_operation_output(exec: &OperationPart) -> Option<String> {
    let mut object = serde_json::Map::new();
    let text = exec.output_text().unwrap_or_default();
    if !text.trim().is_empty() {
        object.insert(
            "text".to_string(),
            serde_json::Value::String(text.to_string()),
        );
    }
    if let Some(payload) = exec
        .result
        .structured
        .clone()
        .or_else(|| exec.details.to_json_payload())
    {
        object.insert("structured".to_string(), payload);
    }
    let managed_outputs = if exec.result.managed_outputs.is_empty() {
        exec.details.managed_outputs.as_slice()
    } else {
        exec.result.managed_outputs.as_slice()
    };
    if !managed_outputs.is_empty() {
        object.insert(
            "managed_outputs".to_string(),
            serde_json::Value::Array(
                managed_outputs
                    .iter()
                    .cloned()
                    .filter_map(|output| serde_json::to_value(output).ok())
                    .collect(),
            ),
        );
    }
    object.insert("truncated".to_string(), serde_json::Value::Bool(true));
    serde_json::to_string(&serde_json::Value::Object(object)).ok()
}

fn structured_web_search_output(exec: &OperationPart) -> Option<String> {
    let agena_runtime_tools::tool::ToolPayloadOutput::WebSearch {
        query,
        backend,
        results,
    } = agena_runtime_tools::tool::ToolPayloadOutput::from_tool_output(
        exec.invocation.name.as_str(),
        &exec.details,
    )?
    else {
        return None;
    };

    let results = results
        .into_iter()
        .map(|result| {
            let snippet = compact_optional_text(result.snippet, MAX_MODEL_WEB_RESULT_SNIPPET_CHARS);
            serde_json::json!({
                "title": result.title,
                "url": result.url,
                "snippet": snippet,
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string(&serde_json::json!({
        "query": query,
        "backend": backend,
        "results": results,
    }))
    .ok()
}

fn structured_web_crawl_output(exec: &OperationPart) -> Option<String> {
    if !matches!(
        exec.invocation.name.as_str(),
        "web.crawl" | "agena_web__crawl" | "crawl"
    ) {
        return None;
    }

    let payload = exec.details.to_json_payload()?;
    let report: ModelWebCrawlReport = serde_json::from_value(payload).ok()?;
    let document_count = report.documents.len();
    let failure_count = report.failures.len();
    let documents = report
        .documents
        .into_iter()
        .take(MAX_MODEL_WEB_CRAWL_DOCUMENTS)
        .map(|document| {
            serde_json::json!({
                "title": document.title,
                "url": document.url,
                "depth": document.depth,
                "chunk_count": document.chunk_count,
            })
        })
        .collect::<Vec<_>>();
    let failures = report
        .failures
        .into_iter()
        .take(MAX_MODEL_WEB_CRAWL_FAILURES)
        .map(|failure| truncate_text(failure.as_str(), MAX_MODEL_WEB_RESULT_SNIPPET_CHARS))
        .collect::<Vec<_>>();

    serde_json::to_string(&serde_json::json!({
        "start_url": report.start_url,
        "engine": report.engine,
        "rendered": report.rendered,
        "stored_count": report.stored_count,
        "cached_count": report.cached_count,
        "duplicate_count": report.duplicate_count,
        "near_duplicate_count": report.near_duplicate_count,
        "failure_count": report.failure_count,
        "total_documents": report.total_documents,
        "documents_truncated": document_count > MAX_MODEL_WEB_CRAWL_DOCUMENTS,
        "documents": documents,
        "failures_truncated": failure_count > MAX_MODEL_WEB_CRAWL_FAILURES,
        "failures": failures,
    }))
    .ok()
}

fn compact_optional_text(value: Option<String>, max_chars: usize) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(|text| truncate_text(text.as_str(), max_chars))
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }

    let mut end = trimmed.len();
    if let Some((idx, _)) = trimmed.char_indices().nth(max_chars) {
        end = idx;
    }
    format!("{}…", trimmed[..end].trim_end())
}

#[derive(Debug, Deserialize)]
struct ModelWebCrawlReport {
    start_url: String,
    engine: String,
    rendered: bool,
    stored_count: usize,
    cached_count: usize,
    duplicate_count: usize,
    near_duplicate_count: usize,
    failure_count: usize,
    total_documents: usize,
    #[serde(default)]
    documents: Vec<ModelWebCrawlDocument>,
    #[serde(default)]
    failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelWebCrawlDocument {
    title: String,
    url: String,
    depth: u32,
    chunk_count: usize,
}

fn attachment_file_content_value(item: &AttachmentItem) -> Option<serde_json::Value> {
    let upload_name = filename(item)
        .map(str::to_owned)
        .unwrap_or_else(|| item.summary_label());
    match &item.source {
        AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
            data_url(item).map(|file_data| {
                serde_json::json!({
                    "type": "file",
                    "file": {
                        "file_data": file_data,
                        "filename": upload_name,
                    }
                })
            })
        }
        AttachmentSource::FileId { file_id } => {
            let file_id = file_id.trim();
            (!file_id.is_empty()).then(|| {
                serde_json::json!({
                    "type": "file",
                    "file": {
                        "file_id": file_id,
                        "filename": upload_name,
                    }
                })
            })
        }
        AttachmentSource::Url { .. } | AttachmentSource::LocalPath { .. } => None,
    }
}

fn parse_data_url(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let payload = trimmed.strip_prefix("data:")?;
    let (metadata, encoded) = payload.split_once(',')?;
    let metadata = metadata.trim();
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return None;
    }
    let mime = metadata
        .strip_suffix(";base64")
        .unwrap_or(metadata)
        .trim()
        .to_owned();
    Some((mime, encoded.to_owned()))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        WirePart, completion_input_provider_state, completion_input_result_status,
        project_completion_input, project_persisted, validate_provider_native_tool_history,
    };
    use agena_domain::ToolInvocation;
    use agena_domain::ToolOutput;
    use agena_domain::{ExecutionStatus, ToolApiFunction};
    use agena_domain::{Role, StructuredObject, TimeRange};
    use agena_runtime_contracts::part::{OperationCompletion, OperationPart};
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
    use serde_json::{Map, Value};

    fn part(kind: &str, role: PartRole, state: PartState, content: Value) -> Part {
        Part {
            part_id: 1,
            kind: kind.to_owned(),
            role,
            state,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
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

    fn run_marker(role: PartRole, provider_state: Option<Value>) -> Part {
        let mut marker = part("run", role, PartState::Completed, Value::Null);
        marker.run_id = None;
        marker.provider_state = provider_state;
        marker
    }

    /// Canonical `tool_call` content for an operation: the invocation identity
    /// as named keys plus the full v1 operation payload under
    /// `operation` (lossless) and `tool_api_call`, mirroring the session
    /// serializer (`tool_call_from_operation`).
    fn tool_call_content(operation: &OperationPart) -> Value {
        let mut object = Map::new();
        object.insert(
            "name".to_owned(),
            Value::String(operation.invocation.name.clone()),
        );
        if let Some(plugin) = &operation.invocation.plugin_name {
            object.insert("plugin".to_owned(), Value::String(plugin.clone()));
        }
        object.insert(
            "input".to_owned(),
            Value::from(operation.invocation.input.clone()),
        );
        object.insert(
            "operation".to_owned(),
            serde_json::to_value(operation).expect("operation is JSON serializable"),
        );
        if let Some(api_call) = &operation.invocation.tool_api_call {
            object.insert(
                "tool_api_call".to_owned(),
                serde_json::to_value(api_call).expect("tool api call is JSON serializable"),
            );
        }
        Value::Object(object)
    }

    fn assistant_operation(invocation: ToolInvocation) -> Part {
        part(
            "tool_call",
            PartRole::Assistant,
            PartState::Completed,
            tool_call_content(&OperationPart::completed(
                0,
                invocation,
                OperationCompletion::new(
                    "Provider tool",
                    "Completed",
                    "ok".to_owned(),
                    Vec::new(),
                    Vec::new(),
                    ToolOutput::default(),
                ),
                TimeRange::default(),
            )),
        )
    }

    #[test]
    fn assistant_launch_notification_projects_as_system_message_never_assistant_text() {
        // An AI-launched background hook rides the assistant turn that launched
        // it. The run keeps Assistant identity for transcript presentation,
        // while the typed body projects as a dedicated SystemMessage part —
        // never as the assistant's own `Text` output.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "proc_test".to_string(),
            operation_kind: "shell".to_string(),
            status: "completed".to_string(),
            summary: "exit 0".to_string(),
            body: "<agena_notification>exit 0</agena_notification>".to_string(),
            ..Default::default()
        };
        let marker = run_marker(PartRole::Assistant, None);
        let mut body_part = part(
            "system_notification",
            PartRole::Assistant,
            PartState::Completed,
            notification.as_value(),
        );
        body_part.run_id = Some(marker.part_id);

        let input = project_completion_input(&[marker, body_part]);

        assert_eq!(
            input.role,
            Role::Assistant,
            "the notification keeps its launching Assistant identity"
        );
        assert!(
            matches!(
                input.parts.as_slice(),
                [agena_provider::CompletionInputPart::SystemMessage { text }]
                    if text == "<agena_notification>exit 0</agena_notification>"
            ),
            "the notification body must project as a typed SystemMessage, never assistant Text"
        );
    }

    #[test]
    fn persisted_notification_projects_as_system_message_wire_part() {
        // `project_persisted` — the projection under `window_items_from_parts`
        // — must emit the notification body as a dedicated SystemMessage wire
        // part (never `WirePart::Text`) so adapters deliver it as a
        // system/notice message rather than the assistant's own reply.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "proc_test".to_string(),
            operation_kind: "shell".to_string(),
            status: "completed".to_string(),
            summary: "exit 0".to_string(),
            body: "<agena_notification>exit 0</agena_notification>".to_string(),
            ..Default::default()
        };
        let body_part = part(
            "system_notification",
            PartRole::Assistant,
            PartState::Completed,
            notification.as_value(),
        );

        let projected = project_persisted(&[body_part]);
        assert_eq!(projected.len(), 1);
        assert!(
            matches!(
                &projected[0],
                WirePart::SystemMessage { text }
                    if text == "<agena_notification>exit 0</agena_notification>"
            ),
            "the persisted notification must project as WirePart::SystemMessage, \
             never WirePart::Text"
        );
    }

    #[test]
    fn completion_input_projection_keeps_role_parts_and_replay_state() {
        let marker = run_marker(
            PartRole::Assistant,
            Some(serde_json::json!({
                "response_id": "resp_123",
                "gemini_thought_signatures": { "part_1": "signature" }
            })),
        );
        let text = part(
            "text",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({ "text": "hello" }),
        );

        let input = project_completion_input(&[marker, text]);

        assert_eq!(input.role, Role::Assistant);
        assert!(matches!(
            input.parts.as_slice(),
            [agena_provider::CompletionInputPart::Text { text }] if text == "hello"
        ));
        assert_eq!(
            input.provider_state.response_id.as_deref(),
            Some("resp_123")
        );
        assert_eq!(
            input.provider_state.gemini_thought_signatures.get("part_1"),
            Some(&"signature".to_owned())
        );
    }

    #[test]
    fn persisted_provider_state_drops_blank_replay_credentials() {
        let state = completion_input_provider_state(&serde_json::json!({
            "response_id": "   ",
            "gemini_thought_signatures": {
                "  call_1  ": "signed",
                "call_2": "   "
            },
            "openai_chat_reasoning_details": [],
            "copilot_reasoning_opaque": "   "
        }))
        .expect("one normalized signature remains");
        assert_eq!(state.response_id, None);
        assert_eq!(
            state.gemini_thought_signatures,
            std::collections::BTreeMap::from([("call_1".to_owned(), "signed".to_owned())])
        );
        assert_eq!(state.openai_chat_reasoning_details, None);
        assert_eq!(state.copilot_reasoning_opaque, None);
    }

    #[test]
    fn every_structured_non_execution_outcome_reaches_the_model_as_completed() {
        for status in [
            ExecutionStatus::PolicyDenied,
            ExecutionStatus::UserDeclined,
            ExecutionStatus::CapabilityUnavailable,
            ExecutionStatus::ToolUnavailable,
        ] {
            assert_eq!(
                completion_input_result_status(status),
                agena_provider::CompletionInputToolResultStatus::Completed,
                "non-execution status {status:?} must not become a provider failure"
            );
        }
    }

    #[test]
    fn selected_skill_reference_enters_provider_history_as_message_scoped_guidance() {
        let skill = part(
            "skill_ref",
            PartRole::User,
            PartState::Completed,
            serde_json::json!({
                "skills": [{
                    "name": "review",
                    "description": "Review the current branch",
                    "instructions": "Inspect the diff and report concrete findings.",
                    "content_hash": "abc123",
                    "source": "bundled",
                    "aliases": []
                }]
            }),
        );
        let text = part(
            "text",
            PartRole::User,
            PartState::Completed,
            serde_json::json!({ "text": "Review my current change." }),
        );

        let projected = project_persisted(&[skill, text]);
        assert_eq!(projected.len(), 2);
        let WirePart::Text { text: skill } = &projected[0] else {
            panic!("expected Skill guidance text")
        };
        assert!(skill.contains("message_scoped_user_selected_skill_reference"));
        assert!(skill.contains("Inspect the diff and report concrete findings."));
        assert!(skill.contains("user explicitly selected"));
        assert!(matches!(
            &projected[1],
            WirePart::Text { text } if text == "Review my current change."
        ));
    }

    #[test]
    fn text_artifact_identity_projects_actual_text_without_editor_placeholders() {
        let text = part(
            "text",
            PartRole::User,
            PartState::Completed,
            serde_json::json!({ "text": "x".repeat(1_000) }),
        );

        let projected = project_persisted(&[text]);
        assert!(matches!(
            projected.as_slice(),
            [WirePart::Text { text }] if text.len() == 1_000 && text.chars().all(|ch| ch == 'x')
        ));
    }

    #[test]
    fn interrupted_tool_call_projects_as_a_closed_call_result_pair() {
        // A tool call whose turn was interrupted before the execution settled
        // must replay as a call/result pair. A provider-visible
        // `function_call` with no matching `function_call_output` is rejected
        // by gateways that translate the Responses input into Chat messages
        // ("No tool output found for tool call N") — the failure seen in
        // session 48, where the provider died mid-turn and left an orphaned
        // call behind.
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Call.function_name(),
            StructuredObject::try_from(serde_json::json!({
                "tool": "fs.read",
                "input": { "path": "README.md" }
            }))
            .expect("structured Tool API payload"),
        );
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: ToolApiFunction::Call,
            arguments: invocation.input.clone(),
        });
        invocation.name = "fs.read".to_owned();
        invocation.input = StructuredObject::try_from(serde_json::json!({ "path": "README.md" }))
            .expect("target input");
        let pending =
            OperationPart::pending(1396, invocation, "Read README.md", TimeRange::default());
        let tool_call = part(
            "tool_call",
            PartRole::Assistant,
            PartState::Completed,
            tool_call_content(&pending),
        );

        let projected = project_persisted(&[tool_call]);
        assert_eq!(projected.len(), 2, "a call must never replay orphaned");
        let WirePart::ToolCall { id, .. } = &projected[0] else {
            panic!("expected ToolCall");
        };
        let WirePart::ToolResult {
            tool_call_id,
            output_json,
            status,
            ..
        } = &projected[1]
        else {
            panic!("expected the paired ToolResult");
        };
        assert_eq!(tool_call_id, id, "call and result must share the call id");
        assert!(
            output_json.contains("interrupted"),
            "the model must learn the call never completed, got {output_json:?}"
        );
        assert_eq!(
            *status,
            agena_provider::CompletionInputToolResultStatus::Cancelled,
            "an interrupted call replays as a cancelled result"
        );
    }

    #[test]
    fn cancelled_tool_uses_numeric_fallback_for_blank_provider_id_and_nonempty_output() {
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Help.function_name(),
            StructuredObject::try_from(serde_json::json!({"tool": "fs.read"}))
                .expect("structured help input"),
        );
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: ToolApiFunction::Help,
            arguments: invocation.input.clone(),
        });
        let mut operation = OperationPart::pending(
            1396,
            invocation,
            "Tool help",
            TimeRange {
                start_ms: 1,
                end_ms: Some(2),
            },
        );
        operation
            .metadata
            .insert("agena.operation_id".to_owned(), serde_json::json!("   "));
        operation.result.state = agena_domain::ToolResultState::Cancelled;
        let tool_call = part(
            "tool_call",
            PartRole::Assistant,
            PartState::Cancelled,
            tool_call_content(&operation),
        );

        let projected = project_persisted(&[tool_call]);
        assert_eq!(projected.len(), 2);
        let WirePart::ToolCall { id, .. } = &projected[0] else {
            panic!("expected ToolCall")
        };
        let WirePart::ToolResult {
            tool_call_id,
            output_json,
            status,
            ..
        } = &projected[1]
        else {
            panic!("expected ToolResult")
        };
        assert_eq!(id, "1396");
        assert_eq!(tool_call_id, id);
        assert!(output_json.contains("cancelled"));
        assert_eq!(
            *status,
            agena_provider::CompletionInputToolResultStatus::Cancelled
        );
    }

    #[test]
    fn execution_tool_name_never_becomes_provider_function_name() {
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Call.function_name(),
            StructuredObject::try_from(serde_json::json!({
                "tool": "agena.session.rename",
                "input": { "title": "renamed" }
            }))
            .expect("structured Tool API payload"),
        );
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: ToolApiFunction::Call,
            arguments: invocation.input.clone(),
        });
        invocation.name = "agena.session.rename".to_owned();
        invocation.input = StructuredObject::try_from(serde_json::json!({ "title": "renamed" }))
            .expect("target input");
        let part = assistant_operation(invocation);

        validate_provider_native_tool_history(std::slice::from_ref(&part))
            .expect("valid Tool API history");
        let projected = project_persisted(&[part]);
        let WirePart::ToolCall {
            function,
            arguments_json,
            ..
        } = &projected[0]
        else {
            panic!("expected provider Tool API call")
        };
        assert_eq!(
            function.function_name(),
            ToolApiFunction::Call.function_name()
        );
        assert_eq!(function.function_name(), "tools_call");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(arguments_json).expect("projected arguments")
                ["tool"],
            "agena.session.rename"
        );
    }

    #[test]
    fn dotted_legacy_tool_api_handler_is_rejected_during_replay() {
        let part = assistant_operation(ToolInvocation::new(
            "agena.tools.help",
            StructuredObject::try_from(serde_json::json!({ "tool": "session.rename" }))
                .expect("structured help payload"),
        ));

        let error = validate_provider_native_tool_history(std::slice::from_ref(&part))
            .expect_err("dotted names must never become protocol identities");
        assert!(
            error
                .to_string()
                .contains("no explicit Tool API function identity")
        );
    }

    #[test]
    fn unadvertised_execution_tool_operation_is_rejected_as_provider_history() {
        let part = assistant_operation(ToolInvocation::new(
            "agena.session.rename",
            StructuredObject::default(),
        ));

        let error = validate_provider_native_tool_history(&[part])
            .expect_err("execution-tool call must fail");
        assert!(
            error
                .to_string()
                .contains("no explicit Tool API function identity")
        );
    }

    #[test]
    fn mismatched_tool_api_identity_is_rejected() {
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Call.function_name(),
            StructuredObject::default(),
        );
        invocation.tool_api_call = Some(agena_domain::ToolApiCall {
            function: ToolApiFunction::Help,
            arguments: StructuredObject::default(),
        });
        let part = assistant_operation(invocation);

        let error = validate_provider_native_tool_history(&[part]).expect_err("mismatch must fail");
        assert!(
            error
                .to_string()
                .contains("must store its exact protocol handler name")
        );
    }

    #[test]
    fn reasoning_parts_survive_projection_for_replay() {
        // A persisted assistant reasoning part must project into a dedicated
        // Reasoning wire part (and CompletionInputPart::Reasoning) so providers
        // that require `reasoning_content` replay can reconstruct it, instead
        // of the reasoning being silently dropped on projection.
        let think = part(
            "think",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({ "summary": ["think step by step"] }),
        );
        let text = part(
            "text",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({ "text": "visible answer" }),
        );

        let wire_parts = project_persisted(&[think.clone(), text.clone()]);
        assert_eq!(wire_parts.len(), 2);
        assert!(matches!(
            &wire_parts[0],
            WirePart::Reasoning { text } if text == "think step by step"
        ));

        let input = project_completion_input(&[think, text]);
        assert!(matches!(
            &input.parts[0],
            agena_provider::CompletionInputPart::Reasoning { text }
                if text == "think step by step"
        ));
    }

    #[test]
    fn hook_message_projects_as_system_message_never_assistant_text() {
        // A stop hook that blocked the run carries its continuation in
        // `message`; projecting the persisted hook run must surface that
        // message as a dedicated system message — never as assistant reply
        // text. The message is System-originated (the agent.stop plugin's
        // `continue_with_message`), so Text would be the same role leak the
        // notification fix closed.
        let hook_with_message = part(
            "hook",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({
                "hook": "agent.stop",
                "summary": "agent.stop hook blocked stop: workflow plan autorun",
                "message": "<plan_context>continue with the next plan step</plan_context>",
            }),
        );
        let wire_parts = project_persisted(&[hook_with_message.clone()]);
        assert_eq!(wire_parts.len(), 1);
        assert!(matches!(
            &wire_parts[0],
            WirePart::SystemMessage { text }
                if text == "<plan_context>continue with the next plan step</plan_context>"
        ));

        // A hook with no message stays human-only activity: nothing projects.
        let hook_without_message = part(
            "hook",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({
                "hook": "chat.params",
                "summary": "chat.params hook ran (no change)",
            }),
        );
        assert!(
            project_persisted(&[hook_without_message]).is_empty(),
            "a message-less hook run must not reach the model prompt"
        );
    }

    #[test]
    fn hook_continuation_on_an_assistant_run_projects_as_system_message_never_assistant_text() {
        // The hook-leak regression, mirroring the notification test: an
        // agent.stop continuation rides an Assistant-role `hook` part on the
        // launching assistant run (no new run). The projection must derive
        // Role::Assistant (the run's role) but emit the continuation as a
        // dedicated SystemMessage part — never as `Text`, which is exactly how
        // hook content would leak into the model's visible output.
        let marker = run_marker(PartRole::Assistant, None);
        let mut hook_part = part(
            "hook",
            PartRole::Assistant,
            PartState::Completed,
            serde_json::json!({
                "hook": "agent.stop",
                "summary": "agent.stop hook blocked stop: workflow plan autorun",
                "message": "<plan_context>continue with the next plan step</plan_context>",
            }),
        );
        hook_part.run_id = Some(marker.part_id);

        let input = project_completion_input(&[marker, hook_part]);

        assert_eq!(
            input.role,
            Role::Assistant,
            "the hook rides the launching assistant run (no new run marker)"
        );
        assert!(
            matches!(
                input.parts.as_slice(),
                [agena_provider::CompletionInputPart::SystemMessage { text }]
                    if text == "<plan_context>continue with the next plan step</plan_context>"
            ),
            "the hook continuation must project as a typed SystemMessage, never as assistant Text"
        );
    }

    #[test]
    fn scheduled_delivery_projects_as_system_message_never_assistant_text() {
        // The cron-identity regression: a scheduled job's prompt is delivered
        // as a Runtime-role `system_notification` part (`operation_kind`
        // "scheduled_delivery") under its own ingress run — never a User-role
        // `user_send` run. Projection maps Runtime to Role::System and emits the
        // prompt as a dedicated SystemMessage part, never assistant `Text`.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "delivery-key-1".to_string(),
            operation_kind: "scheduled_delivery".to_string(),
            status: "submitted".to_string(),
            summary: "Scheduled job job-1 fired".to_string(),
            body: "check the background task list and report".to_string(),
            ..Default::default()
        };
        let marker = run_marker(PartRole::Runtime, None);
        let mut body_part = part(
            "system_notification",
            PartRole::Runtime,
            PartState::Completed,
            notification.as_value(),
        );
        body_part.run_id = Some(marker.part_id);

        let input = project_completion_input(&[marker, body_part]);

        assert_eq!(
            input.role,
            Role::System,
            "the scheduled delivery keeps Runtime/System identity"
        );
        assert!(
            matches!(
                input.parts.as_slice(),
                [agena_provider::CompletionInputPart::SystemMessage { text }]
                    if text == "check the background task list and report"
            ),
            "the scheduled prompt must project as a typed SystemMessage, never as assistant Text"
        );
    }

    #[test]
    fn persisted_scheduled_delivery_projects_as_system_message_wire_part() {
        // `project_persisted` must emit the scheduled prompt as a dedicated
        // SystemMessage wire part (never `WirePart::Text`), so adapters deliver
        // it as a system/notice message rather than the assistant's own reply.
        let notification = agena_runtime_contracts::part_content::SystemNotificationContent {
            operation_id: "delivery-key-1".to_string(),
            operation_kind: "scheduled_delivery".to_string(),
            status: "submitted".to_string(),
            summary: "Scheduled job job-1 fired".to_string(),
            body: "check the background task list and report".to_string(),
            ..Default::default()
        };
        let body_part = part(
            "system_notification",
            PartRole::Runtime,
            PartState::Completed,
            notification.as_value(),
        );

        let projected = project_persisted(&[body_part]);
        assert_eq!(projected.len(), 1);
        assert!(
            matches!(
                &projected[0],
                WirePart::SystemMessage { text }
                    if text == "check the background task list and report"
            ),
            "the persisted scheduled prompt must project as WirePart::SystemMessage, \
             never WirePart::Text"
        );
    }
}
