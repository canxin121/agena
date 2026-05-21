/// Provider-agnostic wire representation of a single chat message.
///
/// [`WirePart`] is the normalised, provider-ready view of an internal
/// [`Message`].  Callers obtain it via [`project`] and then map it to
/// whatever payload format their provider expects.
///
/// The projection step handles all concerns that are shared across every
/// provider:
///   - stripping UI-only content (file changes, permission requests, …)
///   - resolving the tool-call ID from `part.operation_id` with fallback
///   - carrying operation outputs through one provider-neutral projection path
///   - emitting an empty output for still-pending / in-progress tool executions
use base64::Engine as _;

use crate::message::{
    AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, Message, OperationPart,
    PartContent, ToolInvocation,
};

// ─── Core type ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WirePart {
    Text {
        text: String,
    },
    Attachment {
        item: AttachmentItem,
    },
    ToolCall {
        id: String,
        name: String,
        arguments_json: String,
    },
    ToolResult {
        tool_call_id: String,
        /// Name of the tool that produced this result. Required by some
        /// providers (Gemini's `functionResponse`) and ignored by others
        /// (OpenAI/Anthropic only need the call id). Empty string if
        /// unknown — callers that require it must fall back gracefully.
        tool_name: String,
        output_json: String,
    },
}

impl WirePart {
    pub fn as_text_lossy(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Attachment { item } => hint_text(item),
            Self::ToolCall { id, name, .. } => format!("[tool_call:{name}:{id}]"),
            Self::ToolResult { tool_call_id, .. } => format!("[tool_result:{tool_call_id}]"),
        }
    }
}

// ─── Projection ───────────────────────────────────────────────────────────────

/// Normalise a [`Message`] into a flat list of provider-ready [`WirePart`]s.
pub fn project(message: &Message) -> Vec<WirePart> {
    let mut parts: Vec<WirePart> = Vec::new();

    for part in &message.parts {
        let Some(content) = part.content.as_ref() else {
            continue;
        };

        match content {
            PartContent::Text(text) => {
                if !text.text.is_empty() {
                    parts.push(WirePart::Text {
                        text: text.text.clone(),
                    });
                }
            }
            PartContent::Attachment(attachment) => {
                for item in &attachment.attachments {
                    parts.push(WirePart::Attachment { item: item.clone() });
                }
            }
            PartContent::Operation(exec) => {
                let call_id = part
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| exec.call_id().to_string());

                let (name, arguments_json) = project_tool_invocation(exec, message);
                parts.push(WirePart::ToolCall {
                    id: call_id.clone(),
                    name: name.clone(),
                    arguments_json,
                });

                if matches!(
                    part.status,
                    ExecutionStatus::Completed | ExecutionStatus::Failed
                ) {
                    parts.push(WirePart::ToolResult {
                        tool_call_id: call_id,
                        tool_name: name,
                        output_json: project_operation_output(part.status, exec),
                    });
                }
            }
            _ => {}
        }
    }

    parts
}

/// Like [`project`] but returns a single lossy string — used when the provider
/// only needs plain text (e.g. system messages for non-multimodal endpoints).
pub(crate) fn project_text_lossy(message: &Message) -> String {
    let parts = project(message);
    if parts.is_empty() {
        message.as_text_lossy()
    } else {
        parts_text_lossy(parts.as_slice())
    }
}

// ─── Part helpers ─────────────────────────────────────────────────────────────

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
            WirePart::Attachment { item } => attachment_to_openai_content_value(item),
            WirePart::ToolCall { name, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_call:{name}]") })
            }
            WirePart::ToolResult { tool_call_id, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
            }
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(items)
}

// ─── Private helpers ──────────────────────────────────────────────────────────

fn project_tool_invocation(exec: &OperationPart, _message: &Message) -> (String, String) {
    invocation_name_and_args(exec.invocation())
}

fn invocation_name_and_args(invocation: &ToolInvocation) -> (String, String) {
    let ToolInvocation { name, input, .. } = invocation;
    let json_value: serde_json::Value = input.clone().into();
    (
        name.clone(),
        serde_json::to_string(&json_value).unwrap_or_else(|_| "{}".to_owned()),
    )
}

fn project_operation_output(status: ExecutionStatus, exec: &OperationPart) -> String {
    match status {
        ExecutionStatus::Pending | ExecutionStatus::InProgress | ExecutionStatus::Cancelled => {
            String::new()
        }
        ExecutionStatus::Completed => exec.model_output.text.clone(),
        ExecutionStatus::Failed => exec
            .output_text()
            .or_else(|| exec.error_message())
            .unwrap_or_default()
            .to_string(),
    }
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
    use chrono::Utc;

    use super::*;
    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, MessageMetadata,
        MessagePart, MessageStatus, ToolOutput,
    };
    use crate::role::Role;

    #[test]
    fn project_keeps_tool_attachment_images() {
        let message = crate::message::Message {
            id: 9,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    9,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::Operation(crate::message::OperationPart::completed(
                        4,
                        crate::message::ToolInvocation {
                            name: "resource_tool".to_string(),
                            plugin_name: Some("resource_plugin".to_string()),
                            input: crate::message::StructuredObject::default(),
                        },
                        "resource ready",
                        Vec::new(),
                        Vec::new(),
                        ToolOutput::default(),
                        crate::message::TimeRange {
                            start_ms: 0,
                            end_ms: Some(1),
                        },
                    )),
                ),
                MessagePart::with_content(
                    2,
                    9,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::attachments(vec![AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_string(),
                        source: AttachmentSource::Url {
                            url: "https://example.com/image.png".to_string(),
                        },
                        filename: Some("image.png".to_string()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    }]),
                ),
            ],
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        let parts = project(&message);
        assert_eq!(parts.len(), 3);
        assert!(matches!(parts[0], WirePart::ToolCall { .. }));
        assert!(matches!(parts[1], WirePart::ToolResult { .. }));
        assert_eq!(
            parts[2],
            WirePart::Attachment {
                item: AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::Url {
                        url: "https://example.com/image.png".to_string(),
                    },
                    filename: Some("image.png".to_string()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                },
            }
        );
    }

    #[test]
    fn project_ignores_legacy_pruned_tool_result_tag() {
        let mut message = crate::message::Message {
            id: 11,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    11,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::Operation(crate::message::OperationPart::completed(
                        5,
                        crate::message::ToolInvocation {
                            name: "resource_tool".to_string(),
                            plugin_name: Some("resource_plugin".to_string()),
                            input: crate::message::StructuredObject::default(),
                        },
                        "very long original output",
                        Vec::new(),
                        Vec::new(),
                        ToolOutput::default(),
                        crate::message::TimeRange {
                            start_ms: 0,
                            end_ms: Some(1),
                        },
                    )),
                ),
                MessagePart::with_content(
                    2,
                    11,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::attachments(vec![AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_string(),
                        source: AttachmentSource::Url {
                            url: "https://example.com/image.png".to_string(),
                        },
                        filename: Some("image.png".to_string()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    }]),
                ),
            ],
            created_at: Utc::now(),
            metadata: {
                let mut metadata = MessageMetadata::default();
                metadata.add_tag("tool_result_pruned");
                metadata
            },
            usage: None,
            finish: None,
        };
        message.parts[0].operation_id = Some("call_5".to_string());

        let parts = project(&message);
        assert_eq!(
            parts,
            vec![
                WirePart::ToolCall {
                    id: "call_5".to_string(),
                    name: "resource_tool".to_string(),
                    arguments_json: "{}".to_string(),
                },
                WirePart::ToolResult {
                    tool_call_id: "call_5".to_string(),
                    tool_name: "resource_tool".to_string(),
                    output_json: "very long original output".to_string(),
                },
                WirePart::Attachment {
                    item: AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_string(),
                        source: AttachmentSource::Url {
                            url: "https://example.com/image.png".to_string(),
                        },
                        filename: Some("image.png".to_string()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    },
                },
            ]
        );
    }

    #[test]
    fn project_ignores_legacy_attachment_stripped_tag() {
        let message = crate::message::Message {
            id: 12,
            role: Role::User,
            state: MessageStatus::Completed,
            parts: vec![MessagePart::with_content(
                1,
                12,
                Utc::now(),
                ExecutionStatus::Completed,
                PartContent::attachments(vec![AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::DataUrl {
                        url: format!("data:image/png;base64,{}", "A".repeat(1024)),
                    },
                    filename: Some("screenshot.png".to_string()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }]),
            )],
            created_at: Utc::now(),
            metadata: {
                let mut metadata = MessageMetadata::default();
                metadata.add_tag("attachment_payload_stripped");
                metadata
            },
            usage: None,
            finish: None,
        };

        let parts = project(&message);
        assert_eq!(
            parts,
            vec![WirePart::Attachment {
                item: AttachmentItem {
                    kind: AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::DataUrl {
                        url: format!("data:image/png;base64,{}", "A".repeat(1024)),
                    },
                    filename: Some("screenshot.png".to_string()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                },
            }]
        );
    }
}
