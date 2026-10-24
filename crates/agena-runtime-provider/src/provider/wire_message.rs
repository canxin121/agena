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
    ThinkContent, TypedContent, attachment_from_file_ref, command_reference_from_command_ref,
    decode, operation_from_tool_call,
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
        attachments: Vec<AttachmentItem>,
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
/// missing, the operation's numeric call id is the deterministic fallback.
fn project_operation_call_id(exec: &OperationPart) -> String {
    exec.metadata
        .get("agena.operation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| exec.call_id().to_string())
}

// ─── Typed-content projections ───────────────────────────────────────────────
//
// Provider projection uses the runtime-facing values recovered through the
// public extractor helpers in `agena_runtime_contracts::part_content`
// (`operation_from_tool_call`, `attachment_from_file_ref`,
// `command_reference_from_command_ref`, …).

/// Derive the reasoning text providers prefer from the canonical `think`
/// shape (summary wins, raw content otherwise).
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
/// typed [`TypedContent`] and projected into the current provider wire
/// contract. Run markers and
/// provider-only operations are skipped; the coarse `state` column drives
/// result emission with the fine-grained denial outcomes recovered from the
/// rich content.
pub fn project_persisted(parts: &[Part]) -> Vec<WirePart> {
    let mut wire: Vec<WirePart> = Vec::new();

    for part in parts {
        if !part.visibility.visible_to_ai() {
            continue;
        }
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
            // Paste and compaction are textual provider context; emit their
            // text directly when non-empty.
            TypedContent::PasteRef(paste) => {
                if !paste.text.is_empty() {
                    wire.push(WirePart::Text {
                        text: paste.text.clone(),
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
                    if file_ref
                        .extra
                        .get("delivery")
                        .and_then(serde_json::Value::as_str)
                        == Some("reference")
                    {
                        let label = item
                            .filename
                            .as_deref()
                            .or(item.title.as_deref())
                            .unwrap_or("attachment");
                        let text=local_resource_reference_text(item).unwrap_or_else(||format!("[Reference-only resource: {label}; media type {}. No media bytes or remote-file access were provided to this model. Use a separately authorized cloud media tool or explicitly attach contents to analyze it.]",item.mime));
                        wire.push(WirePart::Text { text });
                    } else if let Some(text) = local_resource_reference_text(item) {
                        wire.push(WirePart::Text { text });
                    } else {
                        wire.push(WirePart::Attachment { item: item.clone() });
                    }
                }
            }
            TypedContent::CommandRef(command_ref) => {
                let command_reference = command_reference_from_command_ref(&command_ref);
                if !command_reference.commands.is_empty() {
                    wire.push(WirePart::Text {
                        text: command_reference.model_context_text(),
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
                            attachments: exec
                                .raw_output()
                                .map(|output| output.attachments.clone())
                                .unwrap_or_default(),
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
                    attachments: exec
                        .raw_output()
                        .map(|output| output.attachments.clone())
                        .unwrap_or_default(),
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
            TypedContent::Notice(_) | TypedContent::Error(_) => {}
        }
    }

    wire
}

fn wire_part_from_completion_input(part: CompletionInputPart) -> WirePart {
    match part {
        CompletionInputPart::Text { text } => WirePart::Text { text },
        CompletionInputPart::Reasoning { text } => WirePart::Reasoning { text },
        CompletionInputPart::SystemMessage { text } => WirePart::SystemMessage { text },
        CompletionInputPart::Attachment { attachment } => {
            let item = attachment_item_from_completion_input(attachment);
            match local_resource_reference_text(&item) {
                Some(text) => WirePart::Text { text },
                None => WirePart::Attachment { item },
            }
        }
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
            mut output_json,
            attachments,
        } => {
            let mut media = Vec::new();
            for attachment in attachments {
                let item = attachment_item_from_completion_input(attachment);
                if let Some(reference) = local_resource_reference_text(&item) {
                    output_json.push('\n');
                    output_json.push_str(&reference);
                } else if !matches!(item.source, AttachmentSource::LocalPath { .. }) {
                    media.push(item);
                }
            }
            WirePart::ToolResult {
                tool_call_id,
                function,
                arguments_json,
                status,
                output_json,
                attachments: media,
            }
        }
    }
}

/// The domain view of a projected attachment, for callers that must judge a
/// history attachment against the selected route's rules.
pub(super) fn attachment_item_for_validation(
    attachment: &CompletionInputAttachment,
) -> AttachmentItem {
    attachment_item_from_completion_input(attachment.clone())
}

/// Rewrite one history attachment whose bytes this route cannot deliver into an
/// explicit unavailable notice. The attachment keeps its kind, MIME and display
/// metadata; the projection turns it into text instead of media.
pub(super) fn degrade_completion_input_attachment(
    attachment: &mut CompletionInputAttachment,
    reason: &str,
) {
    if matches!(
        attachment.source,
        CompletionInputAttachmentSource::Unavailable { .. }
    ) {
        return;
    }
    attachment.source = CompletionInputAttachmentSource::Unavailable {
        reason: reason.to_owned(),
    };
    attachment.sha256 = None;
    attachment.size_bytes = None;
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
            CompletionInputAttachmentSource::ProviderData { route, data } => {
                AttachmentSource::ProviderData { route, data }
            }
            CompletionInputAttachmentSource::FileId { id } => {
                AttachmentSource::FileId { file_id: id }
            }
            CompletionInputAttachmentSource::LocalPath { path } => {
                AttachmentSource::LocalPath { path }
            }
            CompletionInputAttachmentSource::Unavailable { reason } => {
                AttachmentSource::Unavailable { reason }
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
            .and_then(|part| {
                part.content
                    .get("rounds")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|rounds| rounds.last())
                    .and_then(|round| round.get("provider_state"))
                    .filter(|state| !state.is_null())
                    .or(part.provider_state.as_ref())
            })
            .and_then(|state| completion_input_provider_state_from_parts(state, parts))
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
            attachments,
        } => CompletionInputPart::ToolResult {
            tool_call_id,
            function,
            arguments_json,
            status,
            output_json,
            attachments: attachments
                .into_iter()
                .map(completion_input_attachment)
                .collect(),
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
            AttachmentSource::ProviderData { route, data } => {
                CompletionInputAttachmentSource::ProviderData { route, data }
            }
            AttachmentSource::FileId { file_id } => {
                CompletionInputAttachmentSource::FileId { id: file_id }
            }
            AttachmentSource::LocalPath { path } => {
                CompletionInputAttachmentSource::LocalPath { path }
            }
            AttachmentSource::Unavailable { reason } => {
                CompletionInputAttachmentSource::Unavailable { reason }
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

pub fn completion_input_provider_state_from_parts(
    value: &serde_json::Value,
    parts: &[Part],
) -> Option<CompletionInputProviderState> {
    let sources = parts
        .iter()
        .filter(|part| part.kind == "think")
        .map(|part| {
            agena_runtime_contracts::provider_state::ReasoningTextSource::from_content(
                part.part_id,
                &part.content,
            )
        })
        .collect::<Vec<_>>();
    match agena_runtime_contracts::provider_state::restore_reasoning_text(value, &sources) {
        Ok(state) => completion_input_provider_state(&state),
        Err(error) => {
            tracing::error!(diagnostic = %error, "provider replay state contains an invalid part-content reference");
            None
        }
    }
}

pub fn completion_input_provider_state(
    value: &serde_json::Value,
) -> Option<CompletionInputProviderState> {
    let mut state: CompletionInputProviderState =
        match serde_json::from_value::<PartProviderState>(value.clone()) {
            Ok(state) => state.into(),
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode persisted provider state for prompt replay",
                        &error,
                    ),
                    "malformed persisted provider state was omitted from prompt replay"
                );
                return None;
            }
        };
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

// ─── Classifier transcript projection ─────────────────────────────────────────

/// Per-part character cap for the classifier transcript. One runaway tool
/// output must not consume the whole budget: the classifier needs breadth
/// (which actions ran, in what order, with what arguments) far more than it
/// needs the tail of a 200k-character build log.
const CLASSIFIER_PART_CHARS: usize = 4_000;

/// Render an already-resolved semantic run, including checkpoint text and
/// rendered tool outputs that do not have a standalone stored part.
pub fn project_classifier_input_run_text(run: &CompletionInputRun) -> String {
    let role = classifier_role_label(run.role);
    let mut lines: Vec<String> = Vec::new();
    for part in &run.parts {
        match part {
            CompletionInputPart::Text { text } => push_classifier_line(&mut lines, role, text),
            // The agent's own narration. Labelled distinctly because it is
            // evidence of intent, never evidence about the action itself.
            CompletionInputPart::Reasoning { text } => {
                push_classifier_line(&mut lines, "assistant (reasoning)", text)
            }
            CompletionInputPart::SystemMessage { text } => {
                push_classifier_line(&mut lines, "system", text)
            }
            CompletionInputPart::Attachment { attachment } => {
                let label = attachment
                    .filename
                    .as_deref()
                    .or(attachment.title.as_deref())
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or("attachment");
                lines.push(format!("{role}: [attachment: {label}]"));
            }
            CompletionInputPart::ToolCall {
                function,
                arguments_json,
                ..
            } => lines.push(format!(
                "tool {} call: {}",
                classifier_tool_label(function, arguments_json),
                truncate_classifier_part(arguments_json)
            )),
            CompletionInputPart::ToolResult {
                function,
                arguments_json,
                status,
                output_json,
                ..
            } => lines.push(format!(
                "tool {} result ({}): {}",
                classifier_tool_label(function, arguments_json),
                classifier_result_status_label(*status),
                truncate_classifier_part(output_json)
            )),
        }
    }
    lines.join("\n")
}

/// Human-readable target of a tool call.
///
/// Every gateway call projects under the single protocol name `tools_call`, so
/// a transcript of `tool tools_call call: {"tool":"shell.exec",…}` buries the
/// one fact the reviewer judges — *which* tool ran. The `tool` argument inside
/// the call payload names it, and names the matching result too (that result
/// carries no arguments of its own), which keeps a call and its result legible
/// as a pair.
fn classifier_tool_label(function: &ModelToolFunction, arguments_json: &str) -> String {
    if function.function_name() != agena_domain::ToolApiFunction::Call.function_name() {
        return function.function_name().to_owned();
    }
    serde_json::from_str::<serde_json::Value>(arguments_json.trim())
        .ok()
        .and_then(|value| {
            value
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| function.function_name().to_owned())
}

fn classifier_role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn classifier_result_status_label(
    status: agena_provider::CompletionInputToolResultStatus,
) -> &'static str {
    use agena_provider::CompletionInputToolResultStatus as Status;
    match status {
        Status::Completed => "completed",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

fn push_classifier_line(lines: &mut Vec<String>, label: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    lines.push(format!("{label}: {}", truncate_classifier_part(text)));
}

fn truncate_classifier_part(text: &str) -> String {
    if text.len() <= CLASSIFIER_PART_CHARS {
        return text.to_owned();
    }
    let marker = "…[truncated]…";
    let remaining = CLASSIFIER_PART_CHARS.saturating_sub(marker.len());
    let mut head = remaining * 2 / 3;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - (remaining - remaining * 2 / 3);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}{marker}{}", &text[..head], &text[tail..])
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

pub fn parts_text_lossy(parts: &[WirePart]) -> String {
    parts
        .iter()
        .map(WirePart::as_text_lossy)
        .collect::<Vec<_>>()
        .join("")
}

// ─── Attachment helpers ───────────────────────────────────────────────────────

/// Render a workspace-local resource as a lazy, model-visible reference.
///
/// Local paths belong to Agena's workspace and are intentionally never sent to
/// a provider as multimodal bytes. Text can be read by fs.read; media contents
/// can be delivered by fs.read_media or explicit composer delivery.
/// Do not instruct the model to loop through metadata-only media reads.
fn local_resource_reference_text(item: &AttachmentItem) -> Option<String> {
    if let AttachmentSource::Unavailable { reason } = &item.source {
        return Some(AttachmentSource::unavailable_notice(
            &item.summary_label(),
            reason,
        ));
    }
    let AttachmentSource::LocalPath { path } = &item.source else {
        return None;
    };
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    let resource = serde_json::json!({
        "kind": item.kind.as_ref(),
        "path": path,
        "name": item.filename.as_deref().unwrap_or_default(),
        "media_type": item.mime,
        "size_bytes": item.size_bytes,
        "sha256": item.sha256,
    });
    let inspect = match item.kind {
        AttachmentKind::Image => {
            "Call tools_help for fs.read_media, then tools_call with this exact local path to see the image with the current conversation model. fs.read returns a reference only and does not let you see the picture."
        }
        AttachmentKind::Pdf => {
            "Call tools_help for fs.read_media, then tools_call with this exact local path to inspect PDF text and page visuals with the current model. Use fs.document for text extraction. fs.read attachment mode returns a reference only."
        }
        AttachmentKind::Audio | AttachmentKind::Video => {
            "Call fs.read_media to provide the actual audio/video bytes to the current model. This requires confirmed model and protocol support; unsupported inputs return an explicit error. Do not infer playback or analysis from metadata."
        }
        AttachmentKind::File => {
            "For UTF-8 text, use fs.read with file_path set exactly to the resource path. Use fs.read_media for supported binary media, or fs.document to extract PDF/Office text. Binary attachment mode returns a local reference only."
        }
    };
    let payload = serde_json::json!({
        "semantics": "message_scoped_user_selected_resource_reference",
        "guidance": [
            "The user attached this workspace resource by reference; its contents are not embedded in the message.",
            inspect,
            "Do not infer the resource contents from its filename or metadata alone."
        ],
        "resource": resource,
    });
    let encoded = serde_json::to_string_pretty(&payload)
        .expect("workspace resource reference is always JSON serializable")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Some(format!(
        "<agena_resource_reference>\n{encoded}\n</agena_resource_reference>"
    ))
}

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
        AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
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
        | AttachmentSource::LocalPath { .. }
        | AttachmentSource::Unavailable { .. } => None,
    }
}

pub fn media_url(item: &AttachmentItem) -> Option<String> {
    match &item.source {
        AttachmentSource::Url { url } | AttachmentSource::DataUrl { url } => {
            Some(url.trim().to_owned())
        }
        AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
            if item.mime.trim().is_empty() || data.trim().is_empty() {
                None
            } else {
                Some(format!("data:{};base64,{}", item.mime.trim(), data.trim()))
            }
        }
        AttachmentSource::FileId { .. }
        | AttachmentSource::LocalPath { .. }
        | AttachmentSource::Unavailable { .. } => None,
    }
}

pub fn base64_with_mime(item: &AttachmentItem) -> Option<(String, String)> {
    match &item.source {
        AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
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
        | AttachmentSource::LocalPath { .. }
        | AttachmentSource::Unavailable { .. } => None,
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
        AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
            match base64::engine::general_purpose::STANDARD.decode(data.trim()) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(
                        attachment_name = ?item.filename,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode a text attachment from base64",
                            &error,
                        ),
                        "attachment text projection was omitted"
                    );
                    return None;
                }
            }
        }
        AttachmentSource::DataUrl { url } => {
            let Some((_, encoded)) = url.split_once(',') else {
                tracing::warn!(
                    attachment_name = ?item.filename,
                    "attachment text projection was omitted because its data URL has no payload separator"
                );
                return None;
            };
            match base64::engine::general_purpose::STANDARD.decode(encoded.trim()) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(
                        attachment_name = ?item.filename,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode a text attachment data URL",
                            &error,
                        ),
                        "attachment text projection was omitted"
                    );
                    return None;
                }
            }
        }
        AttachmentSource::Url { .. }
        | AttachmentSource::FileId { .. }
        | AttachmentSource::LocalPath { .. }
        | AttachmentSource::Unavailable { .. } => return None,
    };

    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) => {
            tracing::warn!(
                attachment_name = ?item.filename,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a text attachment as UTF-8",
                    &error,
                ),
                "attachment text projection was omitted"
            );
            None
        }
    }
}

/// Serialize one [`AttachmentItem`] to an OpenAI Chat content-part JSON value.
///
/// Chat Completions supports PDF `file` parts and native WAV/MP3 `input_audio`.
/// A user-selected text
/// resource (for example a long clipboard paste prepared as a bounded UTF-8 text
/// file) must reach the model as text: gateways that only implement Chat
/// Completions drop the Responses-only `file` part silently, which is how a
/// submitted message arrived "empty" while its attachment looked as if it had
/// only been delivered on a later turn. Text-like files are therefore inlined
/// verbatim, matching the Anthropic and Bedrock adapters; binary documents keep
/// the `file` shape and fall back to a stable hint when no bytes are attached.
pub fn attachment_to_openai_content_value(item: &AttachmentItem) -> serde_json::Value {
    match item.kind {
        AttachmentKind::Image => media_url(item)
            .map(|url| serde_json::json!({ "type": "image_url", "image_url": { "url": url } }))
            .unwrap_or_else(|| serde_json::json!({ "type": "text", "text": hint_text(item) })),
        AttachmentKind::File => attachment_text(item)
            .map(|text| serde_json::json!({ "type": "text", "text": text }))
            .unwrap_or_else(|| serde_json::json!({ "type": "text", "text": hint_text(item) })),
        AttachmentKind::Audio => base64_with_mime(item)
            .filter(|(mime, _)| matches!(mime.as_str(), "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"))
            .map(|(mime, data)| serde_json::json!({"type":"input_audio", "input_audio":{"data":data,"format":if matches!(mime.as_str(), "audio/wav" | "audio/x-wav") {"wav"} else {"mp3"}}}))
            .unwrap_or_else(|| serde_json::json!({"type":"text", "text":hint_text(item)})),
        AttachmentKind::Video => serde_json::json!({"type":"text", "text":hint_text(item)}),
        AttachmentKind::Pdf => {
            attachment_file_content_value(item)
                .unwrap_or_else(|| serde_json::json!({ "type": "text", "text": hint_text(item) }))
        }
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
    let function = match model_tool_function_for_invocation(invocation) {
        Ok(function) => function,
        Err(error) => {
            tracing::error!(
                tool = %invocation.name,
                diagnostic = %error,
                "tool invocation was omitted from provider wire projection because its function identity is invalid"
            );
            return None;
        }
    };
    // A direct tool call (any function other than the opaque `tools_call`
    // envelope) carries its payload in `invocation.input`; only the envelope
    // stores the payload inside its own arguments. Reading `arguments`
    // unconditionally sent an empty object to the provider for every non-
    // envelope call, so the model replayed a call with no arguments at all.
    let source = match invocation.tool_api_call.as_ref() {
        Some(call) if call.function == ToolApiFunction::Call => call.arguments.clone(),
        _ => invocation.input.clone(),
    };
    let json_value: serde_json::Value = source.into();
    let arguments = match serde_json::to_string(&json_value) {
        Ok(arguments) => arguments,
        Err(error) => {
            tracing::error!(
                tool = %invocation.name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize tool invocation arguments for provider wire projection",
                    &error,
                ),
                "tool invocation was omitted from provider wire projection"
            );
            return None;
        }
    };
    Some((function, arguments))
}

pub fn tool_api_function_for_invocation(
    invocation: &ToolInvocation,
) -> Result<ToolApiFunction, String> {
    let stored_name = invocation.name.as_str();
    if let Some(call) = invocation.tool_api_call.as_ref() {
        if call.function == ToolApiFunction::Call {
            // A result records the envelope function but repeats no arguments;
            // its own name is the target, exactly as it is on the request that
            // produced it. Only a *call* must name its target in `arguments`.
            if stored_name == call.function.function_name() {
                return Ok(call.function);
            }
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
            let raw = exec.raw_output();
            if raw.is_none_or(|raw| raw.managed_outputs.is_empty() && !raw.truncated) {
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

fn serialize_structured_operation_value(
    value: &serde_json::Value,
    context: &str,
) -> Option<String> {
    match serde_json::to_string(value) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::error!(
                operation = context,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "structured operation output could not be serialized for the model"
            );
            None
        }
    }
}

fn generic_structured_operation_output(exec: &OperationPart) -> Option<String> {
    let payload = exec.raw_output()?.payload.clone()?;
    serialize_structured_operation_value(&payload, "serialize generic structured tool output")
}

fn managed_operation_output(exec: &OperationPart) -> Option<String> {
    let raw = exec.raw_output()?;
    let mut object = serde_json::Map::new();
    let text = exec.output_text().unwrap_or_default();
    if !text.trim().is_empty() {
        object.insert(
            "text".to_string(),
            serde_json::Value::String(text.to_string()),
        );
    }
    if let Some(payload) = raw.payload.clone() {
        object.insert("structured".to_string(), payload);
    }
    if !raw.managed_outputs.is_empty() {
        let mut managed_outputs = Vec::with_capacity(raw.managed_outputs.len());
        for output in &raw.managed_outputs {
            match serde_json::to_value(output) {
                Ok(output) => managed_outputs.push(output),
                Err(error) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "serialize managed tool output for model projection",
                            &error,
                        ),
                        "managed tool output was omitted from the model projection"
                    );
                }
            }
        }
        object.insert(
            "managed_outputs".to_string(),
            serde_json::Value::Array(managed_outputs),
        );
    }
    object.insert("truncated".to_string(), serde_json::Value::Bool(true));
    serialize_structured_operation_value(
        &serde_json::Value::Object(object),
        "serialize managed tool output",
    )
}

fn structured_web_search_output(exec: &OperationPart) -> Option<String> {
    let output =
        match agena_domain::ToolOutput::from_json_payload(exec.raw_output()?.payload.as_ref()) {
            Ok(output) => output,
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "tool output is not a typed web-search payload; using generic projection"
                );
                return None;
            }
        };
    let agena_runtime_tools::tool::ToolPayloadOutput::WebSearch {
        query,
        backend,
        results,
    } = agena_runtime_tools::tool::ToolPayloadOutput::from_tool_output(
        exec.invocation.name.as_str(),
        &output,
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

    serialize_structured_operation_value(
        &serde_json::json!({
            "query": query,
            "backend": backend,
            "results": results,
        }),
        "serialize web-search output for model projection",
    )
}

fn structured_web_crawl_output(exec: &OperationPart) -> Option<String> {
    if !matches!(
        exec.invocation.name.as_str(),
        "web.crawl" | "agena_web__crawl" | "crawl"
    ) {
        return None;
    }

    let payload = exec.raw_output()?.payload.clone()?;
    let report: ModelWebCrawlReport = match serde_json::from_value(payload) {
        Ok(report) => report,
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode typed web-crawl output for model projection",
                    &error,
                ),
                "tool output is not a typed web-crawl report; using generic projection"
            );
            return None;
        }
    };
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

    serialize_structured_operation_value(
        &serde_json::json!({
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
        }),
        "serialize web-crawl output for model projection",
    )
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
        AttachmentSource::Base64 { .. }
        | AttachmentSource::ProviderData { .. }
        | AttachmentSource::DataUrl { .. } => data_url(item).map(|file_data| {
            serde_json::json!({
                "type": "file",
                "file": {
                    "file_data": file_data,
                    "filename": upload_name,
                }
            })
        }),
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
        AttachmentSource::Url { .. }
        | AttachmentSource::LocalPath { .. }
        | AttachmentSource::Unavailable { .. } => None,
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
