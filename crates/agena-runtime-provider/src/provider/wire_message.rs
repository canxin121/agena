//! Provider-agnostic wire representation of a completion input run.
//!
//! [`project`] converts the provider-owned [`CompletionInputRun`] contract into
//! [`WirePart`]s consumed by concrete adapters. The session kernel supplies the
//! run's role, ordered parts, tool outcomes, and provider replay state.
//! Attachment projection keeps local and unavailable resources as explicit
//! references while adapters encode deliverable media for their protocols.

use base64::Engine as _;

use crate::ProviderError;
use agena_domain::{AttachmentItem, AttachmentKind, AttachmentSource, Role};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentKind, CompletionInputAttachmentSource,
    CompletionInputPart, CompletionInputRun, ModelToolFunction,
};
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

/// Enforce the boundary between Tool API functions and execution tools before
/// any adapter serializes a request. Every replayed operation must be a known
/// Tool API function; execution-tool names and internal keys are never
/// provider function calls.
pub fn validate_provider_native_tool_input_history(
    runs: &[CompletionInputRun],
) -> Result<(), ProviderError> {
    for part in runs.iter().flat_map(|run| &run.parts) {
        if let CompletionInputPart::ToolCall { function, .. }
        | CompletionInputPart::ToolResult { function, .. } = part
            && function.gateway_function().is_none()
        {
            return Err(ProviderError::Config(format!(
                "model history contains non-protocol function {:?}; execution tools must be replayed through tools_call",
                function.function_name(),
            )));
        }
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

/// Whether this attachment's bytes are text that every protocol can carry as
/// ordinary message text.
///
/// One rule for every adapter: a text-like file must never be handed to a
/// document channel (`input_file`, PDF parts), because those channels parse
/// the bytes as a document and reject plain text.
pub fn attachment_is_text_like(item: &AttachmentItem) -> bool {
    let mime = item.mime.trim().to_ascii_lowercase();
    mime.starts_with("text/")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/javascript"
        )
}

pub fn attachment_text(item: &AttachmentItem) -> Option<String> {
    if !attachment_is_text_like(item) {
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

#[cfg(test)]
mod protocol_history_tests {
    use super::*;

    #[test]
    fn current_input_preserves_order_tool_identity_and_media_delivery_boundaries() {
        let image = CompletionInputAttachment {
            kind: CompletionInputAttachmentKind::Image,
            mime: "image/png".into(),
            source: CompletionInputAttachmentSource::Base64 {
                data: "aGk=".into(),
            },
            filename: Some("image.png".into()),
            title: Some("Preview".into()),
            size_bytes: Some(2),
            sha256: Some("fixture-sha".into()),
            width: Some(1),
            height: Some(1),
            duration_ms: None,
            page_count: None,
        };
        let mut local = image.clone();
        local.source = CompletionInputAttachmentSource::LocalPath {
            path: "workspace.png".into(),
        };
        let mut unavailable = image.clone();
        degrade_completion_input_attachment(&mut unavailable, "its provider route changed");
        assert_eq!(unavailable.sha256, None);
        assert_eq!(unavailable.size_bytes, None);
        let function: ModelToolFunction = agena_domain::ToolApiFunction::Call.into();
        let arguments_json =
            r#"{"tool":"fs.read_media","input":{"file_path":"image.png"}}"#.to_owned();
        let run = CompletionInputRun {
            role: Role::Assistant,
            parts: vec![
                CompletionInputPart::Text {
                    text: "before".into(),
                },
                CompletionInputPart::Reasoning {
                    text: "reasoning".into(),
                },
                CompletionInputPart::SystemMessage {
                    text: "system notice".into(),
                },
                CompletionInputPart::ToolCall {
                    id: "call-42".into(),
                    function: function.clone(),
                    arguments_json: arguments_json.clone(),
                },
                CompletionInputPart::ToolResult {
                    tool_call_id: "call-42".into(),
                    function: function.clone(),
                    arguments_json: arguments_json.clone(),
                    status: agena_provider::CompletionInputToolResultStatus::Failed,
                    output_json: r#"{"error":"failed"}"#.into(),
                    attachments: vec![image, local, unavailable],
                },
            ],
            provider_state: Default::default(),
        };
        let projected = project(&run);
        let [
            WirePart::Text { text },
            WirePart::Reasoning { text: reasoning },
            WirePart::SystemMessage { text: notice },
            WirePart::ToolCall {
                id,
                function: call_function,
                arguments_json: call_arguments,
            },
            WirePart::ToolResult {
                tool_call_id,
                function: result_function,
                arguments_json: result_arguments,
                status,
                output_json,
                attachments,
            },
        ] = projected.as_slice()
        else {
            panic!("the current completion contract must retain part ordering and roles");
        };
        assert_eq!(text, "before");
        assert_eq!(reasoning, "reasoning");
        assert_eq!(notice, "system notice");
        assert_eq!(id, "call-42");
        assert_eq!(tool_call_id, id);
        assert_eq!(call_function, &function);
        assert_eq!(result_function, &function);
        assert_eq!(call_arguments, &arguments_json);
        assert_eq!(result_arguments, &arguments_json);
        assert_eq!(
            *status,
            agena_provider::CompletionInputToolResultStatus::Failed
        );
        assert!(output_json.starts_with(r#"{"error":"failed"}"#));
        assert!(output_json.contains("workspace.png"));
        assert!(output_json.contains("its provider route changed"));
        assert!(!output_json.contains("aGk="));
        assert_eq!(attachments.len(), 1);
        assert!(
            matches!(&attachments[0].source, AttachmentSource::Base64 { data } if data == "aGk=")
        );
        assert_eq!(attachments[0].sha256.as_deref(), Some("fixture-sha"));
        assert_eq!(attachments[0].size_bytes, Some(2));
        assert_eq!(attachments[0].title.as_deref(), Some("Preview"));
    }

    #[test]
    fn history_replays_explicit_protocol_calls_and_rejects_control_decisions() {
        let run = |function| CompletionInputRun {
            role: Role::Assistant,
            parts: vec![CompletionInputPart::ToolCall {
                id: "call-1".into(),
                function,
                arguments_json: r#"{"tool":"fs.read","input":{"file_path":"a.txt"}}"#.into(),
            }],
            provider_state: Default::default(),
        };
        assert!(
            validate_provider_native_tool_input_history(&[run(
                agena_domain::ToolApiFunction::Call.into()
            ),])
            .is_ok()
        );
        assert!(
            validate_provider_native_tool_input_history(&[run(ModelToolFunction::ApproveAction),])
                .is_err()
        );
    }
}
