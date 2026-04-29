use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use futures_core::Stream;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::error::{AppError, ProviderErrorKind};
use crate::message::{
    AttachmentItem, AttachmentKind, AttachmentSource, Message, PartContent, TodoListPart,
    TodoPriority, TodoStatus, ToolExecutionPart, ToolInvocation,
};
use crate::model::{ModelId, ProviderId};
use crate::provider::{
    CompletionFinishReason, CompletionResponse, CompletionStreamEvent, CompletionToolCall,
    CompletionUsage, ManagedCredential, should_retry_credential,
};
use crate::role::Role;
use crate::session::{MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_TOOL_RESULT_PRUNED};

pub(crate) const PRUNED_TOOL_RESULT_PLACEHOLDER: &str = "[Old tool result content cleared]";

pub(crate) fn prompt_cache_header_entries(
    headers: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut entries = headers
        .iter()
        .filter_map(|(key, value)| {
            let normalized_key = key.trim().to_ascii_lowercase();
            let normalized_value = value.trim();
            if normalized_key.is_empty()
                || normalized_value.is_empty()
                || prompt_cache_ignores_header(normalized_key.as_str())
            {
                return None;
            }

            Some((normalized_key, normalized_value.to_owned()))
        })
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries
}

pub(crate) fn prompt_cache_ignores_header(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "authorization"
        || key == "proxy-authorization"
        || key == "cookie"
        || key == "set-cookie"
        || key == "x-api-key"
        || key.contains("request-id")
        || key.contains("correlation-id")
        || key.contains("trace")
        || key.contains("span")
        || key.contains("baggage")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
}

pub(crate) fn request_shape_fingerprint<T>(value: &T) -> String
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_slice());
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedSessionPart {
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

impl ProjectedSessionPart {
    pub fn as_text_lossy(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Attachment { item } => attachment_hint(item),
            Self::ToolCall { id, name, .. } => format!("[tool_call:{name}:{id}]"),
            Self::ToolResult { tool_call_id, .. } => format!("[tool_result:{tool_call_id}]"),
        }
    }
}

pub fn project_session_parts(message: &Message) -> Vec<ProjectedSessionPart> {
    let pruned_tool_result =
        message.role == Role::Tool && message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED);
    let stripped_attachment_payloads = message
        .metadata
        .has_tag(MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED);
    let mut parts: Vec<ProjectedSessionPart> = Vec::new();
    for part in &message.parts {
        let Some(content) = part.content.as_ref() else {
            continue;
        };

        match content {
            PartContent::Text(text) => {
                if !text.text.is_empty() {
                    parts.push(ProjectedSessionPart::Text {
                        text: text.text.clone(),
                    });
                }
            }
            PartContent::Attachment(attachment) => {
                if pruned_tool_result {
                    continue;
                }
                for item in &attachment.attachments {
                    if stripped_attachment_payloads {
                        parts.push(ProjectedSessionPart::Text {
                            text: attachment_hint(item),
                        });
                    } else {
                        parts.push(ProjectedSessionPart::Attachment { item: item.clone() });
                    }
                }
            }
            PartContent::TodoList(todo) if message.role != Role::Tool => {
                parts.push(ProjectedSessionPart::Text {
                    text: render_todo_list(todo),
                });
            }
            PartContent::TodoList(_) => {}
            PartContent::ToolExecution(exec) => {
                let call_id = part.operation_id.clone().unwrap_or_else(|| match exec {
                    ToolExecutionPart::Pending { call_id, .. }
                    | ToolExecutionPart::InProgress { call_id, .. }
                    | ToolExecutionPart::Completed { call_id, .. }
                    | ToolExecutionPart::Failed { call_id, .. } => call_id.to_string(),
                });

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
                    parts.push(ProjectedSessionPart::ToolResult {
                        tool_call_id: call_id,
                        output_json,
                    });
                } else {
                    let (name, arguments_json) = match exec {
                        ToolExecutionPart::Pending { invocation, .. }
                        | ToolExecutionPart::InProgress { invocation, .. }
                        | ToolExecutionPart::Completed { invocation, .. }
                        | ToolExecutionPart::Failed { invocation, .. } => {
                            project_tool_invocation(invocation)
                        }
                    };
                    parts.push(ProjectedSessionPart::ToolCall {
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

pub fn projected_parts_text_lossy(parts: &[ProjectedSessionPart]) -> String {
    parts
        .iter()
        .map(ProjectedSessionPart::as_text_lossy)
        .collect::<Vec<_>>()
        .join("")
}

pub fn tool_results(parts: &[ProjectedSessionPart]) -> Vec<(String, String)> {
    parts
        .iter()
        .filter_map(|part| match part {
            ProjectedSessionPart::ToolResult {
                tool_call_id,
                output_json,
            } => Some((tool_call_id.clone(), output_json.clone())),
            _ => None,
        })
        .collect()
}

pub fn non_tool_result_parts(parts: &[ProjectedSessionPart]) -> Vec<ProjectedSessionPart> {
    parts
        .iter()
        .filter(|part| !matches!(part, ProjectedSessionPart::ToolResult { .. }))
        .cloned()
        .collect()
}

pub(crate) fn project_session_text_lossy(message: &Message) -> String {
    let parts = project_session_parts(message);
    if parts.is_empty() {
        if message.role == Role::Tool && message.metadata.has_tag(MESSAGE_TAG_TOOL_RESULT_PRUNED) {
            return PRUNED_TOOL_RESULT_PLACEHOLDER.to_owned();
        }
        message.as_text_lossy()
    } else {
        projected_parts_text_lossy(parts.as_slice())
    }
}

fn project_tool_invocation(invocation: &ToolInvocation) -> (String, String) {
    match invocation {
        ToolInvocation::Mcp {
            server,
            tool,
            input,
        } => (
            format!("{server}:{tool}"),
            serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
        ),
        ToolInvocation::Custom { name, input } => (
            name.clone(),
            serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
        ),
    }
}

fn attachment_hint(item: &AttachmentItem) -> String {
    let label = item.summary_label();
    match item.kind {
        AttachmentKind::Image => format!("[image:{label}]"),
        AttachmentKind::Audio => format!("[audio:{label}]"),
        AttachmentKind::Video => format!("[video:{label}]"),
        AttachmentKind::Pdf => format!("[document:{label}]"),
        AttachmentKind::File => format!("[file:{label}]"),
    }
}

pub fn attachment_hint_text(item: &AttachmentItem) -> String {
    attachment_hint(item)
}

pub fn attachment_filename(item: &AttachmentItem) -> Option<&str> {
    item.filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn attachment_data_url(item: &AttachmentItem) -> Option<String> {
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

pub fn attachment_media_url(item: &AttachmentItem) -> Option<String> {
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

pub fn attachment_base64_with_mime(item: &AttachmentItem) -> Option<(String, String)> {
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

pub fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

pub fn auth_header_value(scheme: Option<&str>, token: &str) -> String {
    let token = token.trim();
    match scheme.map(str::trim).filter(|s| !s.is_empty()) {
        Some(scheme) => format!("{scheme} {token}"),
        None => token.to_owned(),
    }
}

/// Apply provider-configured `extra_headers` AND consult the loaded
/// plugin host's `chat.headers` hook chain. Plugins can add or remove
/// headers per-provider on every request.
///
/// `provider_id` is used as the `provider` field passed to the hook so
/// plugins can scope their patches.
pub fn apply_request_headers(
    provider_id: &str,
    mut req: reqwest::RequestBuilder,
    extra_headers: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    let mut combined: std::collections::BTreeMap<String, String> = extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if let Some(host) = crate::runtime::plugin_slot::current()
        && !host.is_empty()
    {
        let input = crate::plugin::ChatHeadersInput {
            provider: provider_id.to_string(),
            headers: combined.clone(),
        };
        match host.dispatch_chat_headers_blocking(input) {
            Ok(updated) => combined = updated.headers,
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::chat_headers",
                    provider = provider_id,
                    "chat.headers hook failed (using base headers): {err}"
                );
            }
        }
    }

    for (key, value) in &combined {
        req = req.header(key.as_str(), value.as_str());
    }
    req
}

pub async fn parse_json_response<T>(
    provider_id: &str,
    response: reqwest::Response,
) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    if response.status().is_success() {
        return Ok(response.json::<T>().await?);
    }

    Err(http_status_error_from_response(provider_id, response).await)
}

pub fn parse_json_value<T>(
    provider_id: &str,
    context: &str,
    value: serde_json::Value,
) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|err| {
        AppError::Provider(format!(
            "{provider_id} returned invalid {context} payload: {err}"
        ))
    })
}

pub async fn http_status_error_from_response(
    provider_id: &str,
    response: reqwest::Response,
) -> AppError {
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<empty>".to_owned());

    let body = serde_json::from_str::<ProviderErrorEnvelope>(&body)
        .map(|parsed| {
            let mut message = parsed.error.message;
            if let Some(kind) = parsed.error.kind {
                message.push_str(format!(" (type={kind})").as_str());
            }
            if let Some(code) = parsed.error.code {
                message.push_str(format!(" (code={code})").as_str());
            }
            message
        })
        .unwrap_or(body);

    let classified = classify_http_error(provider_id, status, body.as_str());

    AppError::HttpStatus {
        provider: provider_id.to_owned(),
        status,
        body,
        kind: classified.kind,
        retryable: classified.retryable,
    }
}

#[derive(Debug, Clone, Copy)]
struct ProviderErrorClassification {
    kind: ProviderErrorKind,
    retryable: bool,
}

const CONTEXT_OVERFLOW_PATTERNS: &[&str] = &[
    "exceeds the context window",
    "maximum context length",
    "context length",
    "too many tokens",
    "prompt is too long",
    "request too large",
    "request entity too large",
    "input is too long",
];

fn is_context_overflow_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    CONTEXT_OVERFLOW_PATTERNS
        .iter()
        .any(|pattern| normalized.contains(pattern))
        || ((normalized.starts_with("400") || normalized.starts_with("413"))
            && normalized.contains("(no body)"))
}

#[cfg(test)]
mod projection_tests {
    use chrono::Utc;

    use super::*;
    use crate::message::{
        AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, MessageMetadata,
        MessagePart, MessageStatus, TodoItem, ToolOutput,
    };
    use crate::session::{MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_TOOL_RESULT_PRUNED};

    #[test]
    fn project_session_parts_renders_non_tool_todo_lists_as_text() {
        let message = Message {
            id: 7,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts: vec![MessagePart::with_content(
                1,
                7,
                Utc::now(),
                ExecutionStatus::Completed,
                PartContent::TodoList(TodoListPart {
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

        let parts = project_session_parts(&message);
        assert_eq!(
            parts,
            vec![ProjectedSessionPart::Text {
                text: "Todo list:\n- [in_progress][high] Inspect tool behavior".to_string()
            }]
        );
    }

    #[test]
    fn project_session_parts_keeps_tool_results_without_duplicate_todo_text() {
        let message = Message {
            id: 8,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    8,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(ToolExecutionPart::Completed {
                        call_id: 3,
                        invocation: crate::message::BuiltinToolInput::ToolSearch(
                            crate::message::ToolSearchToolInput {
                                    query: "patch".to_string(),
                                    load: vec!["apply_patch".to_string()],
                                    limit: None,
                                },
                            ).into_invocation(),
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
                    PartContent::TodoList(TodoListPart { items: Vec::new() }),
                ),
            ],
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        let parts = project_session_parts(&message);
        assert_eq!(parts.len(), 1);
        assert!(matches!(parts[0], ProjectedSessionPart::ToolResult { .. }));
    }

    #[test]
    fn project_session_parts_keeps_tool_attachment_images() {
        let message = Message {
            id: 9,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    9,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(ToolExecutionPart::Completed {
                        call_id: 4,
                        invocation: ToolInvocation::Custom {
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

        let parts = project_session_parts(&message);
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[0], ProjectedSessionPart::ToolResult { .. }));
        assert_eq!(
            parts[1],
            ProjectedSessionPart::Attachment {
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
    fn project_session_parts_keeps_all_attachment_kinds() {
        let message = Message {
            id: 10,
            role: Role::User,
            state: MessageStatus::Completed,
            parts: vec![MessagePart::with_content(
                1,
                10,
                Utc::now(),
                ExecutionStatus::Completed,
                PartContent::attachments(vec![
                    AttachmentItem {
                        kind: AttachmentKind::Audio,
                        mime: "audio/mpeg".to_string(),
                        source: AttachmentSource::Url {
                            url: "https://example.com/voice.mp3".to_string(),
                        },
                        filename: Some("voice.mp3".to_string()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    },
                    AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_string(),
                        source: AttachmentSource::DataUrl {
                            url: "data:image/png;base64,AAA".to_string(),
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
                ]),
            )],
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        };

        let parts = project_session_parts(&message);
        assert_eq!(
            parts,
            vec![
                ProjectedSessionPart::Attachment {
                    item: AttachmentItem {
                        kind: AttachmentKind::Audio,
                        mime: "audio/mpeg".to_string(),
                        source: AttachmentSource::Url {
                            url: "https://example.com/voice.mp3".to_string(),
                        },
                        filename: Some("voice.mp3".to_string()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    },
                },
                ProjectedSessionPart::Attachment {
                    item: AttachmentItem {
                        kind: AttachmentKind::Image,
                        mime: "image/png".to_string(),
                        source: AttachmentSource::DataUrl {
                            url: "data:image/png;base64,AAA".to_string(),
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
    fn project_session_parts_replaces_pruned_tool_result_with_placeholder() {
        let mut message = Message {
            id: 11,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![
                MessagePart::with_content(
                    1,
                    11,
                    Utc::now(),
                    ExecutionStatus::Completed,
                    PartContent::ToolExecution(ToolExecutionPart::Completed {
                        call_id: 5,
                        invocation: ToolInvocation::Custom {
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

        let parts = project_session_parts(&message);
        assert_eq!(
            parts,
            vec![ProjectedSessionPart::ToolResult {
                tool_call_id: "call_5".to_string(),
                output_json: PRUNED_TOOL_RESULT_PLACEHOLDER.to_string(),
            }]
        );
        assert_eq!(
            project_session_text_lossy(&message),
            "[tool_result:call_5]".to_string()
        );
        assert!(
            message
                .as_text_lossy()
                .contains("very long original output")
        );
        assert!(message.as_text_lossy().contains("image.png"));
    }

    #[test]
    fn project_session_parts_replaces_stripped_attachments_with_hint_text() {
        let message = Message {
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

        let parts = project_session_parts(&message);
        assert_eq!(
            parts,
            vec![ProjectedSessionPart::Text {
                text: "[image:screenshot.png]".to_string(),
            }]
        );
        assert!(message.as_text_lossy().contains("screenshot.png"));
    }
}

fn classify_http_error(
    provider_id: &str,
    status: reqwest::StatusCode,
    message: &str,
) -> ProviderErrorClassification {
    if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE || is_context_overflow_message(message) {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::ContextOverflow,
            retryable: false,
        };
    }

    let provider = provider_id.trim().to_ascii_lowercase();
    if provider == "openai" && status == reqwest::StatusCode::NOT_FOUND {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::ApiError,
            retryable: true,
        };
    }

    let retryable = matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::CONFLICT
            | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error();

    ProviderErrorClassification {
        kind: ProviderErrorKind::ApiError,
        retryable,
    }
}

fn classify_stream_error(provider_id: &str, code: Option<&str>, message: &str) -> AppError {
    let normalized_code = code.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized_code == "context_length_exceeded" {
        return AppError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: "Input exceeds context window. Try shortening your prompt.".to_owned(),
            kind: ProviderErrorKind::ContextOverflow,
            retryable: false,
        };
    }

    if normalized_code == "insufficient_quota" {
        return AppError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: "Quota exceeded. Please check your plan and billing details.".to_owned(),
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        };
    }

    if normalized_code == "usage_not_included" {
        return AppError::ProviderClassified {
            provider: provider_id.to_owned(),
            message:
                "To use Codex models and OpenAI reasoning summaries, upgrade to Plus plan first."
                    .to_owned(),
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        };
    }

    if normalized_code == "invalid_prompt" {
        return AppError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: if message.trim().is_empty() {
                "Invalid prompt.".to_owned()
            } else {
                message.to_owned()
            },
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        };
    }

    let kind = if is_context_overflow_message(message) {
        ProviderErrorKind::ContextOverflow
    } else {
        ProviderErrorKind::ApiError
    };

    AppError::ProviderClassified {
        provider: provider_id.to_owned(),
        message: if message.trim().is_empty() {
            "provider stream error".to_owned()
        } else {
            message.to_owned()
        },
        retryable: false,
        kind,
    }
}

/// Credential-refresh loop shared by all HTTP providers.
///
/// `build` is called with the current API key each attempt; on 401/403 the
/// credential is force-refreshed once before giving up.
pub async fn send_with_credential_refresh<F>(
    api_key: &ManagedCredential,
    mut build: F,
) -> Result<reqwest::Response, AppError>
where
    F: FnMut(&str) -> reqwest::RequestBuilder,
{
    let mut force_refresh = false;
    loop {
        let key = if force_refresh {
            api_key.force_refresh().await?
        } else {
            api_key.resolve().await?
        };

        let response = build(key.as_str()).send().await?;
        if !force_refresh && should_retry_credential(response.status()) {
            force_refresh = true;
            continue;
        }
        return Ok(response);
    }
}

/// State accumulated while draining a streaming response.
#[derive(Default)]
struct AggregatedToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Drain a `CompletionStreamEvent` stream and assemble a `CompletionResponse`.
///
/// Handles all four event kinds: `TextDelta`, `ThinkingDelta`, `ToolCallDelta`,
/// and `Completed`. The `provider_id` and `fallback_model` are used only when
/// the stream closes without a `Completed` event.
pub async fn aggregate_stream<S>(
    provider_id: &str,
    fallback_model: ModelId,
    stream: S,
) -> Result<CompletionResponse, AppError>
where
    S: Stream<Item = Result<CompletionStreamEvent, AppError>> + Unpin,
{
    let mut stream = stream;
    let mut text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls: BTreeMap<String, AggregatedToolCallState> = BTreeMap::new();
    let mut completed: Option<(
        ProviderId,
        ModelId,
        Option<CompletionFinishReason>,
        Option<CompletionUsage>,
        Option<serde_json::Value>,
    )> = None;

    while let Some(item) = stream.next().await {
        match item? {
            CompletionStreamEvent::TextDelta { delta, .. } => {
                text.push_str(delta.as_str());
            }
            CompletionStreamEvent::ThinkingDelta { delta, .. } => {
                reasoning_text.push_str(delta.as_str());
            }
            CompletionStreamEvent::ToolCallDelta {
                stream_key,
                id,
                name,
                arguments_delta,
                ..
            } => {
                let state = tool_calls.entry(stream_key).or_default();
                if let Some(id) = id {
                    state.id = Some(id);
                }
                if let Some(name) = name {
                    state.name = Some(name);
                }
                state.arguments.push_str(arguments_delta.as_str());
            }
            CompletionStreamEvent::Completed {
                provider_id: pid,
                model,
                finish_reason,
                usage,
                provider_metadata,
            } => {
                completed = Some((pid, model, finish_reason, usage, provider_metadata));
            }
        }
    }

    let (pid, model, finish_reason, usage, provider_metadata) =
        completed.unwrap_or_else(|| {
            (
                ProviderId::new(provider_id),
                fallback_model,
                None,
                None,
                None,
            )
        });

    let calls = tool_calls
        .into_iter()
        .map(|(stream_key, state)| {
            let id = normalize_optional_text(state.id).unwrap_or(stream_key);
            let name = normalize_optional_text(state.name).ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} stream ended with tool call without name"
                ))
            })?;
            Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: state.arguments,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    if text.is_empty() && calls.is_empty() && finish_reason.is_none() {
        return Err(AppError::Provider(format!(
            "{provider_id} stream aggregation produced empty completion"
        )));
    }

    Ok(CompletionResponse {
        provider_id: pid,
        model,
        text,
        reasoning_text: (!reasoning_text.is_empty()).then_some(reasoning_text),
        finish_reason,
        tool_calls: calls,
        usage,
        provider_metadata,
    })
}

/// Wrap an optional response_id as provider_metadata JSON.
pub fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    response_id.map(|id| serde_json::json!({ "response_id": id }))
}

// ─── OpenAI-style content value helpers ──────────────────────────────────────
// Used by both openai_compatible.rs and chat_wire.rs for building message
// content values in the Chat Completions JSON format.

fn attachment_upload_name(item: &AttachmentItem) -> String {
    attachment_filename(item)
        .map(str::to_owned)
        .unwrap_or_else(|| item.summary_label())
}

fn attachment_file_content_value(item: &AttachmentItem) -> Option<serde_json::Value> {
    let filename = attachment_upload_name(item);
    match &item.source {
        AttachmentSource::Base64 { .. } | AttachmentSource::DataUrl { .. } => {
            attachment_data_url(item).map(|file_data| {
                serde_json::json!({
                    "type": "file",
                    "file": {
                        "file_data": file_data,
                        "filename": filename,
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
                        "filename": filename,
                    }
                })
            })
        }
        AttachmentSource::Url { .. } | AttachmentSource::LocalPath { .. } => None,
    }
}

/// Serialize one `AttachmentItem` to an OpenAI Chat content-part JSON value.
pub fn attachment_content_value(item: &AttachmentItem) -> serde_json::Value {
    match item.kind {
        AttachmentKind::Image => attachment_media_url(item)
            .map(|url| serde_json::json!({ "type": "image_url", "image_url": { "url": url } }))
            .unwrap_or_else(|| {
                serde_json::json!({ "type": "text", "text": attachment_hint_text(item) })
            }),
        AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf
        | AttachmentKind::File => attachment_file_content_value(item).unwrap_or_else(|| {
            serde_json::json!({ "type": "text", "text": attachment_hint_text(item) })
        }),
    }
}

/// Serialize a slice of `ProjectedSessionPart`s to an OpenAI Chat `content`
/// array value (i.e. `[{"type":"text","text":"..."},{"type":"image_url",...}]`).
pub fn projected_parts_to_openai_chat_value(parts: &[ProjectedSessionPart]) -> serde_json::Value {
    let items = parts
        .iter()
        .map(|part| match part {
            ProjectedSessionPart::Text { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            ProjectedSessionPart::Attachment { item } => attachment_content_value(item),
            ProjectedSessionPart::ToolCall { name, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_call:{name}]") })
            }
            ProjectedSessionPart::ToolResult { tool_call_id, .. } => {
                serde_json::json!({ "type": "text", "text": format!("[tool_result:{tool_call_id}]") })
            }
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(items)
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChatStreamChoice>,
    #[serde(default)]
    pub usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamChoice {
    #[serde(default)]
    pub delta: Option<ChatStreamDelta>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamDelta {
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesToolEventKind {
    Added,
    Delta,
    Done,
}

#[derive(Debug, Clone)]
pub struct ResponsesToolEvent {
    pub kind: ResponsesToolEventKind,
    pub output_index: Option<usize>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

impl ResponsesToolEvent {
    pub fn stream_key(&self, provider_id: &str) -> Result<String, AppError> {
        if let Some(idx) = self.output_index {
            return Ok(format!("idx:{idx}"));
        }

        if let Some(id) = self.id.as_ref() {
            return Ok(format!("id:{id}"));
        }

        Err(AppError::Provider(format!(
            "{provider_id} returned tool event without output_index/call_id"
        )))
    }
}

pub fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

pub fn optional_non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|raw| if raw.is_empty() { None } else { Some(raw) })
}

pub fn responses_is_completed(event: &serde_json::Value) -> bool {
    matches!(
        responses_event_type(event),
        Some("response.completed" | "response.incomplete" | "response.done")
    )
}

pub fn responses_text_delta(event: &serde_json::Value) -> Option<String> {
    let event_type = responses_event_type(event);
    if matches!(event_type, Some("response.function_call_arguments.delta")) {
        return None;
    }

    if matches!(event_type, Some("response.output_text.delta")) {
        return event
            .get("delta")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .filter(|s| !s.is_empty());
    }

    if matches!(event_type, Some("response.text.delta")) {
        return event
            .get("delta")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .filter(|s| !s.is_empty());
    }

    event
        .get("output_text")
        .and_then(|v| v.as_str())
        .or_else(|| {
            event
                .get("delta")
                .and_then(|v| v.as_str())
                .filter(|_| event_type.is_none())
        })
        .map(ToOwned::to_owned)
        .filter(|s| !s.is_empty())
}

pub fn responses_tool_event(
    provider_id: &str,
    event: &serde_json::Value,
) -> Result<Option<ResponsesToolEvent>, AppError> {
    let Some(event_type) = responses_event_type(event) else {
        return Ok(None);
    };

    if event_type == "response.function_call_arguments.delta" {
        let parsed: ResponsesFunctionArgumentsDeltaPayload = parse_json_value(
            provider_id,
            "responses function_call_arguments.delta",
            event.clone(),
        )?;
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Delta,
            output_index: parsed.output_index,
            id: normalize_optional_text(parsed.call_id).or(normalize_optional_text(parsed.item_id)),
            name: normalize_optional_text(parsed.name),
            arguments: optional_non_empty(Some(parsed.delta)),
        }));
    }

    if event_type == "response.function_call_arguments.done" {
        let parsed: ResponsesFunctionArgumentsDonePayload = parse_json_value(
            provider_id,
            "responses function_call_arguments.done",
            event.clone(),
        )?;
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Done,
            output_index: parsed.output_index,
            id: normalize_optional_text(parsed.call_id).or(normalize_optional_text(parsed.item_id)),
            name: normalize_optional_text(parsed.name),
            arguments: optional_non_empty(Some(parsed.arguments)),
        }));
    }

    if event_type == "response.output_item.added" || event_type == "response.output_item.done" {
        let parsed: ResponsesOutputItemPayload =
            parse_json_value(provider_id, "responses output_item payload", event.clone())?;

        if parsed.item.kind != "function_call" {
            return Ok(None);
        }

        return Ok(Some(ResponsesToolEvent {
            kind: if event_type == "response.output_item.added" {
                ResponsesToolEventKind::Added
            } else {
                ResponsesToolEventKind::Done
            },
            output_index: parsed.output_index,
            id: normalize_optional_text(parsed.item.call_id)
                .or(normalize_optional_text(parsed.item.id)),
            name: normalize_optional_text(parsed.item.name),
            arguments: optional_non_empty(parsed.item.arguments),
        }));
    }

    Ok(None)
}

pub fn responses_finish_reason(event: &serde_json::Value) -> Option<String> {
    event
        .get("response")
        .and_then(|r| r.get("stop_reason"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .get("response")
                .and_then(|r| r.get("incomplete_details"))
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            event
                .get("response")
                .and_then(|r| r.get("status_details"))
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            event
                .get("response")
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            event
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
}

pub fn responses_usage_value(event: &serde_json::Value) -> Option<serde_json::Value> {
    event
        .get("response")
        .and_then(|r| r.get("usage"))
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| event.get("usage").filter(|value| !value.is_null()).cloned())
}

pub fn responses_response_id(event: &serde_json::Value) -> Option<String> {
    event
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .get("id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}

pub fn responses_stream_error(
    provider_id: &str,
    event: &serde_json::Value,
) -> Result<Option<AppError>, AppError> {
    let Some(payload) = event.get("error") else {
        return Ok(None);
    };

    let parsed = parse_json_value::<ResponsesStreamErrorPayload>(
        provider_id,
        "responses stream error",
        payload.clone(),
    )?;

    Ok(Some(classify_stream_error(
        provider_id,
        parsed.code.as_deref(),
        parsed.message.as_deref().unwrap_or_default(),
    )))
}

fn responses_event_type(event: &serde_json::Value) -> Option<&str> {
    event.get("type").and_then(|v| v.as_str())
}

#[derive(Debug, Deserialize)]
struct ResponsesFunctionArgumentsDeltaPayload {
    delta: String,
    #[serde(default)]
    output_index: Option<usize>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesFunctionArgumentsDonePayload {
    arguments: String,
    #[serde(default)]
    output_index: Option<usize>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutputItemPayload {
    #[serde(default)]
    output_index: Option<usize>,
    item: ResponsesOutputItem,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutputItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamErrorPayload {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ProviderErrorEnvelope {
    error: ProviderErrorBody,
}

#[derive(Debug, serde::Deserialize)]
struct ProviderErrorBody {
    message: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    code: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CapabilitySupport, ModelCapabilities, ModelInputModality};

    #[test]
    fn responses_is_completed_only_accepts_terminal_event_types() {
        assert!(responses_is_completed(
            &serde_json::json!({ "type": "response.completed" })
        ));
        assert!(responses_is_completed(
            &serde_json::json!({ "type": "response.incomplete" })
        ));
        assert!(responses_is_completed(
            &serde_json::json!({ "type": "response.done" })
        ));
        assert!(!responses_is_completed(
            &serde_json::json!({ "type": "response.output_item.completed" })
        ));
        assert!(!responses_is_completed(
            &serde_json::json!({ "type": "response.output_text.delta" })
        ));
    }

    #[test]
    fn responses_text_delta_supports_legacy_realtime_delta_type() {
        assert_eq!(
            responses_text_delta(&serde_json::json!({
                "type": "response.text.delta",
                "delta": "hi"
            }))
            .as_deref(),
            Some("hi")
        );
    }

    #[test]
    fn responses_tool_event_supports_function_arguments_done() {
        let event = responses_tool_event(
            "openai",
            &serde_json::json!({
                "type": "response.function_call_arguments.done",
                "output_index": 1,
                "call_id": "call_1",
                "name": "search",
                "arguments": "{\"q\":\"rust\"}"
            }),
        )
        .expect("tool event should parse")
        .expect("tool event should exist");

        assert!(matches!(event.kind, ResponsesToolEventKind::Done));
        assert_eq!(event.output_index, Some(1));
        assert_eq!(event.id.as_deref(), Some("call_1"));
        assert_eq!(event.name.as_deref(), Some("search"));
        assert_eq!(event.arguments.as_deref(), Some("{\"q\":\"rust\"}"));
    }

    #[test]
    fn responses_finish_reason_supports_realtime_status_and_reason() {
        assert_eq!(
            responses_finish_reason(&serde_json::json!({
                "type": "response.done",
                "response": {
                    "status": "completed"
                }
            }))
            .as_deref(),
            Some("completed")
        );

        assert_eq!(
            responses_finish_reason(&serde_json::json!({
                "type": "response.done",
                "response": {
                    "status": "incomplete",
                    "status_details": {
                        "reason": "max_output_tokens"
                    }
                }
            }))
            .as_deref(),
            Some("max_output_tokens")
        );
    }

    #[test]
    fn responses_usage_value_ignores_null_payloads() {
        assert_eq!(
            responses_usage_value(&serde_json::json!({
                "type": "response.done",
                "response": {
                    "usage": null
                }
            })),
            None
        );
        assert_eq!(
            responses_usage_value(&serde_json::json!({
                "type": "response.done",
                "usage": null
            })),
            None
        );
    }

    #[test]
    fn tool_event_stream_key_rejects_missing_identity() {
        let event = ResponsesToolEvent {
            kind: ResponsesToolEventKind::Delta,
            output_index: None,
            id: None,
            name: Some("search".to_owned()),
            arguments: Some("{}".to_owned()),
        };

        let err = event
            .stream_key("openai")
            .expect_err("missing key should be rejected");
        assert!(matches!(err, AppError::Provider(_)));
    }

    #[test]
    fn normalize_optional_text_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_optional_text(Some("  hello  ".to_owned())).as_deref(),
            Some("hello")
        );
        assert_eq!(normalize_optional_text(Some("   ".to_owned())), None);
        assert_eq!(normalize_optional_text(None), None);
    }

    #[test]
    fn prompt_cache_header_entries_filters_volatile_and_secret_headers() {
        let headers = HashMap::from([
            ("X-Request-Id".to_owned(), "req-1".to_owned()),
            ("traceparent".to_owned(), "trace-1".to_owned()),
            ("authorization".to_owned(), "Bearer secret".to_owned()),
            ("x-route".to_owned(), "stable".to_owned()),
        ]);

        let entries = prompt_cache_header_entries(&headers);

        assert_eq!(entries, vec![("x-route".to_owned(), "stable".to_owned())]);
    }

    #[test]
    fn prompt_cache_header_entries_normalizes_and_sorts_headers() {
        let headers = HashMap::from([
            (" X-Zeta ".to_owned(), " z ".to_owned()),
            ("x-alpha".to_owned(), " a ".to_owned()),
        ]);

        let entries = prompt_cache_header_entries(&headers);

        assert_eq!(
            entries,
            vec![
                ("x-alpha".to_owned(), "a".to_owned()),
                ("x-zeta".to_owned(), "z".to_owned()),
            ]
        );
    }

    #[test]
    fn optional_non_empty_keeps_whitespace_content() {
        assert_eq!(optional_non_empty(Some("".to_owned())), None);
        assert_eq!(
            optional_non_empty(Some("  ".to_owned())).as_deref(),
            Some("  ")
        );
        assert_eq!(optional_non_empty(None), None);
    }

    #[test]
    fn classify_http_error_marks_openai_404_retryable() {
        let classified = classify_http_error("openai", reqwest::StatusCode::NOT_FOUND, "missing");
        assert_eq!(classified.kind, ProviderErrorKind::ApiError);
        assert!(classified.retryable);
    }

    #[test]
    fn classify_http_error_marks_context_overflow_non_retryable() {
        let classified = classify_http_error(
            "openai",
            reqwest::StatusCode::BAD_REQUEST,
            "maximum context length is exceeded",
        );
        assert_eq!(classified.kind, ProviderErrorKind::ContextOverflow);
        assert!(!classified.retryable);
    }

    #[test]
    fn responses_stream_error_maps_known_codes() {
        let err = responses_stream_error(
            "openai",
            &serde_json::json!({
                "error": {
                    "code": "context_length_exceeded",
                    "message": "too long"
                }
            }),
        )
        .expect("stream error payload should deserialize")
        .expect("stream error should parse");

        assert!(matches!(
            err,
            AppError::ProviderClassified {
                kind: ProviderErrorKind::ContextOverflow,
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn model_capabilities_reject_non_text_generic_files_only_when_explicitly_unsupported() {
        let capabilities =
            ModelCapabilities::default().with_file_input(CapabilitySupport::Unsupported);
        let attachment = AttachmentItem {
            kind: AttachmentKind::File,
            mime: "application/octet-stream".to_owned(),
            source: AttachmentSource::Base64 {
                data: "QUJD".to_owned(),
            },
            filename: Some("blob.bin".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };

        assert_eq!(
            capabilities.unsupported_attachment_modality(&attachment),
            Some(ModelInputModality::File)
        );

        let text_attachment = AttachmentItem {
            mime: "text/plain".to_owned(),
            filename: Some("notes.txt".to_owned()),
            ..attachment
        };
        assert_eq!(
            capabilities.unsupported_attachment_modality(&text_attachment),
            None
        );
    }
}
