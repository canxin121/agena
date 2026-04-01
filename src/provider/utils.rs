use std::collections::HashMap;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{AppError, ProviderErrorKind};
use crate::message::{
    AttachmentSource, Message, PartContent, TodoListPart, TodoPriority, TodoStatus,
    ToolExecutionPart, ToolInvocation,
};
use crate::role::Role;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedSessionPart {
    Text {
        text: String,
    },
    ImageUrl {
        url: String,
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
            Self::ImageUrl { url } => format!("[image:{url}]"),
            Self::ToolCall { id, name, .. } => format!("[tool_call:{name}:{id}]"),
            Self::ToolResult { tool_call_id, .. } => format!("[tool_result:{tool_call_id}]"),
        }
    }
}

pub fn project_session_parts(message: &Message) -> Vec<ProjectedSessionPart> {
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
                for item in &attachment.attachments {
                    if let AttachmentSource::Url { url } = &item.source {
                        parts.push(ProjectedSessionPart::ImageUrl { url: url.clone() });
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
                    let output_json = match exec {
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

pub fn project_session_text_lossy(message: &Message) -> String {
    let parts = project_session_parts(message);
    if parts.is_empty() {
        message.as_text_lossy()
    } else {
        projected_parts_text_lossy(parts.as_slice())
    }
}

fn project_tool_invocation(invocation: &ToolInvocation) -> (String, String) {
    match invocation {
        ToolInvocation::Builtin { input } => (
            input.to_string(),
            serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned()),
        ),
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

pub fn apply_extra_headers(
    mut req: reqwest::RequestBuilder,
    headers: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    for (key, value) in headers {
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
        ExecutionStatus, MessageMetadata, MessagePart, MessageStatus, TodoItem, ToolOutput,
    };

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
                        invocation: ToolInvocation::Builtin {
                            input: crate::message::BuiltinToolInput::ToolSearch(
                                crate::message::ToolSearchToolInput {
                                    query: "edit".to_string(),
                                    load: vec!["edit".to_string()],
                                    limit: None,
                                },
                            ),
                        },
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

#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamChunk {
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
        .cloned()
        .or_else(|| event.get("usage").cloned())
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
}
