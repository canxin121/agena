/// Provider-agnostic wire representation of a single chat message.
///
/// [`WirePart`] is the normalised, provider-ready view of an internal
/// [`Message`].  Callers obtain it via [`project`] and then map it to
/// whatever payload format their provider expects.
///
/// The projection step handles all concerns that are shared across every
/// provider:
///   - stripping UI-only content (file changes, permission requests, …)
///   - respecting `MESSAGE_TAG_TOOL_RESULT_PRUNED` and
///     `MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED` metadata tags
///   - resolving the tool-call ID from `part.operation_id` with fallback
///   - rendering `TodoList` parts as plain text (skipped inside tool messages)
///   - emitting an empty output for still-pending / in-progress tool executions
use base64::Engine as _;

use crate::message::{
    AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent, TodoListPart,
    TodoPriority, TodoStatus, ToolExecutionPart, ToolInvocation,
};
use crate::role::Role;
use crate::session::{MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_TOOL_RESULT_PRUNED};

pub(crate) const PRUNED_TOOL_RESULT_PLACEHOLDER: &str = "[Old tool result content cleared]";

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
    let pruned_tool_result =
        message.role == Role::Tool && message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED);
    let stripped_attachments = message
        .metadata
        .has_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED);

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
                if pruned_tool_result {
                    continue;
                }
                for item in &attachment.attachments {
                    if stripped_attachments {
                        parts.push(WirePart::Text {
                            text: hint_text(item),
                        });
                    } else {
                        parts.push(WirePart::Attachment { item: item.clone() });
                    }
                }
            }
            PartContent::TodoList(todo) if message.role != Role::Tool => {
                parts.push(WirePart::Text {
                    text: render_todo_list(todo),
                });
            }
            PartContent::TodoList(_) => {}
            PartContent::ToolExecution(exec) => {
                let call_id = part
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| exec.call_id().to_string());

                if message.role == Role::Tool {
                    let output_json = if pruned_tool_result {
                        PRUNED_TOOL_RESULT_PLACEHOLDER.to_owned()
                    } else {
                        match exec {
                            ToolExecutionPart::Pending { .. }
                            | ToolExecutionPart::InProgress { .. } => String::new(),
                            ToolExecutionPart::Completed { output_text, .. } => output_text.clone(),
                            ToolExecutionPart::Failed {
                                output_text,
                                error_message,
                                ..
                            } => {
                                if output_text.is_empty() {
                                    error_message.clone()
                                } else {
                                    output_text.clone()
                                }
                            }
                        }
                    };
                    parts.push(WirePart::ToolResult {
                        tool_call_id: call_id,
                        output_json,
                    });
                } else {
                    let (name, arguments_json) = project_tool_invocation(exec, message);
                    parts.push(WirePart::ToolCall {
                        id: call_id,
                        name,
                        arguments_json,
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
        if message.role == Role::Tool && message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
            return PRUNED_TOOL_RESULT_PLACEHOLDER.to_owned();
        }
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

pub fn tool_results(parts: &[WirePart]) -> Vec<(String, String)> {
    parts
        .iter()
        .filter_map(|part| match part {
            WirePart::ToolResult {
                tool_call_id,
                output_json,
            } => Some((tool_call_id.clone(), output_json.clone())),
            _ => None,
        })
        .collect()
}

pub fn non_tool_result_parts(parts: &[WirePart]) -> Vec<WirePart> {
    parts
        .iter()
        .filter(|part| !matches!(part, WirePart::ToolResult { .. }))
        .cloned()
        .collect()
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

fn project_tool_invocation(exec: &ToolExecutionPart, _message: &Message) -> (String, String) {
    invocation_name_and_args(exec.invocation())
}

fn invocation_name_and_args(invocation: &ToolInvocation) -> (String, String) {
    let ToolInvocation { name, input } = invocation;
    (
        name.clone(),
        serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
    )
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

fn render_todo_list(todo: &TodoListPart) -> String {
    if todo.items.is_empty() {
        return "Todo list is empty.".to_string();
    }
    let mut lines = vec!["Todo list:".to_string()];
    for item in &todo.items {
        lines.push(format!(
            "- [{}][{}] {}",
            todo_status_label(item.status),
            todo_priority_label(item.priority),
            item.content
        ));
    }
    lines.join("\n")
}

fn todo_status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "pending",
        TodoStatus::InProgress => "in_progress",
        TodoStatus::Completed => "completed",
        TodoStatus::Cancelled => "cancelled",
    }
}

fn todo_priority_label(priority: TodoPriority) -> &'static str {
    match priority {
        TodoPriority::High => "high",
        TodoPriority::Medium => "medium",
        TodoPriority::Low => "low",
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, MessageMetadata,
        MessagePart, MessageStatus, TodoItem, ToolOutput,
    };
    use crate::session::{MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_TOOL_RESULT_PRUNED};

    #[test]
    fn project_renders_non_tool_todo_lists_as_text() {
        let message = crate::message::Message {
            id: 7,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts: vec![MessagePart::with_content(
                1,
                7,
                Utc::now(),
                ExecutionStatus::Completed,
                PartContent::TodoList(crate::message::TodoListPart {
                    items: vec![TodoItem {
                        content: "Inspect tool behavior".to_string(),
                        status: TodoStatus::InProgress,
                        priority: TodoPriority::High,
                    }],
                }),
            )],
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        let parts = project(&message);
        assert_eq!(
            parts,
            vec![WirePart::Text {
                text: "Todo list:\n- [in_progress][high] Inspect tool behavior".to_string()
            }]
        );
    }

    #[test]
    fn project_skips_todo_list_inside_tool_message() {
        let message = crate::message::Message {
            id: 8,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    8,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(crate::message::ToolExecutionPart::Completed {
                        call_id: 3,
                        invocation: crate::message::BuiltinToolInput::ToolSearch(
                            crate::message::ToolSearchToolInput {
                                query: "patch".to_string(),
                                load: vec!["apply_patch".to_string()],
                                limit: None,
                            },
                        )
                        .into_invocation(),
                        output_text: "Loaded deferred tools.".to_string(),
                        blocks: Vec::new(),
                        attachments: Vec::new(),
                        details: ToolOutput::default(),
                        lifecycle: crate::message::TimeRange::default(),
                    }),
                ),
                MessagePart::with_content(
                    2,
                    8,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::TodoList(crate::message::TodoListPart { items: Vec::new() }),
                ),
            ],
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        let parts = project(&message);
        assert_eq!(parts.len(), 1);
        assert!(matches!(parts[0], WirePart::ToolResult { .. }));
    }

    #[test]
    fn project_keeps_tool_attachment_images() {
        let message = crate::message::Message {
            id: 9,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    9,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(crate::message::ToolExecutionPart::Completed {
                        call_id: 4,
                        invocation: crate::message::ToolInvocation {
                            name: "resource_tool".to_string(),
                            input: crate::message::StructuredObject::default(),
                        },
                        output_text: "resource ready".to_string(),
                        blocks: Vec::new(),
                        attachments: Vec::new(),
                        details: ToolOutput::default(),
                        lifecycle: crate::message::TimeRange::default(),
                    }),
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
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[0], WirePart::ToolResult { .. }));
        assert_eq!(
            parts[1],
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
    fn project_replaces_pruned_tool_result_with_placeholder() {
        let mut message = crate::message::Message {
            id: 11,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    11,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(crate::message::ToolExecutionPart::Completed {
                        call_id: 5,
                        invocation: crate::message::ToolInvocation {
                            name: "resource_tool".to_string(),
                            input: crate::message::StructuredObject::default(),
                        },
                        output_text: "very long original output".to_string(),
                        blocks: Vec::new(),
                        attachments: Vec::new(),
                        details: ToolOutput::default(),
                        lifecycle: crate::message::TimeRange::default(),
                    }),
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
                metadata.add_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED);
                metadata
            },
            usage: None,
            finish: None,
        };
        message.parts[0].operation_id = Some("call_5".to_string());

        let parts = project(&message);
        assert_eq!(
            parts,
            vec![WirePart::ToolResult {
                tool_call_id: "call_5".to_string(),
                output_json: PRUNED_TOOL_RESULT_PLACEHOLDER.to_string(),
            }]
        );
    }

    #[test]
    fn project_replaces_stripped_attachments_with_hint_text() {
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
                metadata.add_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED);
                metadata
            },
            usage: None,
            finish: None,
        };

        let parts = project(&message);
        assert_eq!(
            parts,
            vec![WirePart::Text {
                text: "[image:screenshot.png]".to_string(),
            }]
        );
    }
}
