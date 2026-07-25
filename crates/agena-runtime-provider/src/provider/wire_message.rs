use agena_domain::{ExecutionStatus, Role};
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
use serde::Deserialize;

use crate::ProviderError;
use agena_domain::{ToolApiFunction, ToolInvocation};
use agena_provider::{
    CompletionInputAttachment, CompletionInputAttachmentKind, CompletionInputAttachmentSource,
    CompletionInputMessage, CompletionInputPart, CompletionInputProviderState, ModelToolFunction,
};
use agena_runtime_contracts::message::{
    AttachmentItem, AttachmentKind, AttachmentSource, Message, OperationPart, PartContent,
};

// ─── Runtime-private type ─────────────────────────────────────────────────────

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
            Self::Attachment { item } => hint_text(item),
            Self::ToolCall { id, function, .. } => {
                format!("[tool_call:{}:{id}]", function.function_name())
            }
            Self::ToolResult { tool_call_id, .. } => format!("[tool_result:{tool_call_id}]"),
        }
    }
}

// ─── Projection ───────────────────────────────────────────────────────────────

/// Normalise a [`Message`] into a flat list of provider-ready [`WirePart`]s.
/// Project provider-owned input into the adapter-neutral wire-part view.
pub fn project(message: &CompletionInputMessage) -> Vec<WirePart> {
    message
        .parts
        .iter()
        .cloned()
        .map(wire_part_from_completion_input)
        .collect()
}

/// Project a persisted core message at the session/core boundary.
pub fn project_persisted(message: &Message) -> Vec<WirePart> {
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
                if exec.is_provider_only() || exec.is_ui_only() {
                    continue;
                }
                let call_id = part
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| exec.call_id().to_string());

                let Some((function, arguments_json)) = project_tool_invocation(exec, message)
                else {
                    // Execution-tool/internal operations are handled through a
                    // Tool API function. They are never independent provider
                    // calls.
                    continue;
                };
                if matches!(message.role, Role::Tool) {
                    if matches!(
                        part.status,
                        ExecutionStatus::Completed
                            | ExecutionStatus::Failed
                            | ExecutionStatus::Cancelled
                    ) {
                        parts.push(WirePart::ToolResult {
                            tool_call_id: call_id,
                            function,
                            arguments_json,
                            status: completion_input_result_status(part.status),
                            output_json: project_operation_output(part.status, exec),
                        });
                    }
                    continue;
                }
                parts.push(WirePart::ToolCall {
                    id: call_id.clone(),
                    function: function.clone(),
                    arguments_json: arguments_json.clone(),
                });

                if matches!(
                    part.status,
                    ExecutionStatus::Completed
                        | ExecutionStatus::Failed
                        | ExecutionStatus::Cancelled
                ) {
                    parts.push(WirePart::ToolResult {
                        tool_call_id: call_id,
                        function,
                        arguments_json,
                        status: completion_input_result_status(part.status),
                        output_json: project_operation_output(part.status, exec),
                    });
                }
            }
            _ => {}
        }
    }

    parts
}

fn wire_part_from_completion_input(part: CompletionInputPart) -> WirePart {
    match part {
        CompletionInputPart::Text { text } => WirePart::Text { text },
        CompletionInputPart::Reasoning { text } => WirePart::Text { text },
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

/// Project a persisted core message into the provider-owned completion input
/// contract. Runtime-private presentation and lifecycle fields are intentionally
/// excluded; all provider-visible parts and replay state are retained.
pub fn project_completion_input(message: &Message) -> CompletionInputMessage {
    CompletionInputMessage {
        role: message.role,
        parts: project_persisted(message)
            .into_iter()
            .map(completion_input_part_from_wire)
            .collect(),
        provider_state: message
            .provider_state
            .as_ref()
            .map(completion_input_provider_state)
            .unwrap_or_default(),
    }
}

fn completion_input_part_from_wire(part: WirePart) -> CompletionInputPart {
    match part {
        WirePart::Text { text } => CompletionInputPart::Text { text },
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

fn completion_input_provider_state(
    state: &agena_runtime_contracts::message::MessageProviderState,
) -> CompletionInputProviderState {
    state.clone().into()
}

/// Enforce the boundary between Tool API functions and execution tools before
/// any adapter serializes a request. Every replayed operation must be a known
/// Tool API function; execution-tool names and internal keys are never
/// provider function calls.
pub fn validate_provider_native_tool_input_history(
    _messages: &[CompletionInputMessage],
) -> Result<(), ProviderError> {
    // A `CompletionInputPart::ToolCall`/`ToolResult` already carries the
    // closed `ToolApiFunction` identity. Runtime validates dynamic operation
    // invocations before it projects persisted history into this contract.
    Ok(())
}

#[cfg(test)]
pub fn validate_provider_native_tool_history(messages: &[Message]) -> Result<(), ProviderError> {
    for (message_index, message) in messages.iter().enumerate() {
        for (part_index, part) in message.parts.iter().enumerate() {
            let Some(PartContent::Operation(operation)) = part.content.as_ref() else {
                continue;
            };
            if operation.is_provider_only() || operation.is_ui_only() {
                continue;
            }
            model_tool_function_for_invocation(operation.invocation()).map_err(|reason| ProviderError::Internal(format!(
                "invalid provider tool history at messages[{message_index}].parts[{part_index}]: {reason}"
            )))?;
        }
    }
    Ok(())
}

/// Like [`project`] but returns a single lossy string — used when the provider
/// only needs plain text (e.g. system messages for non-multimodal endpoints).
pub fn project_text_lossy(message: &CompletionInputMessage) -> String {
    let parts = project(message);
    if parts.is_empty() {
        message.as_text_lossy()
    } else {
        parts_text_lossy(parts.as_slice())
    }
}

/// Runtime-private lossy projection used while preparing the persisted prompt window.
pub fn project_persisted_text_lossy(message: &Message) -> String {
    let parts = project_persisted(message);
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

fn project_tool_invocation(
    exec: &OperationPart,
    _message: &Message,
) -> Option<(ModelToolFunction, String)> {
    invocation_name_and_args(exec.invocation())
}

fn invocation_name_and_args(invocation: &ToolInvocation) -> Option<(ModelToolFunction, String)> {
    let function = model_tool_function_for_invocation(invocation).ok()?;
    let json_value: serde_json::Value = invocation.input.clone().into();
    Some((
        function,
        serde_json::to_string(&json_value).unwrap_or_else(|_| "{}".to_owned()),
    ))
}

pub fn tool_api_function_for_invocation(
    invocation: &ToolInvocation,
) -> Result<ToolApiFunction, String> {
    let stored_name = invocation.name.as_str();
    if let Some(function) = invocation.tool_api_function {
        if stored_name == function.function_name() && invocation.plugin_name.is_none() {
            return Ok(function);
        }
        return Err(format!(
            "Tool API function `{}` must store only its exact protocol name and no plugin identity, but the operation stores name `{stored_name}` with plugin {:?}",
            function.function_name(),
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
    if let Some(function) = invocation.tool_api_function {
        tool_api_function_for_invocation(invocation)?;
        return Ok(function.into());
    }
    let provider_name = invocation
        .provider_function_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "invocation `{}` has no provider function identity",
                invocation.name
            )
        })?;
    Ok(ModelToolFunction::new(provider_name))
}

pub fn project_operation_output(status: ExecutionStatus, exec: &OperationPart) -> String {
    match status {
        ExecutionStatus::Pending | ExecutionStatus::InProgress | ExecutionStatus::Cancelled => {
            String::new()
        }
        ExecutionStatus::Completed => {
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
        WirePart, project_completion_input, project_persisted,
        validate_provider_native_tool_history,
    };
    use agena_domain::ToolApiFunction;
    use agena_domain::ToolInvocation;
    use agena_domain::ToolOutput;
    use agena_domain::{Role, StructuredObject, TimeRange};
    use agena_runtime_contracts::message::{
        Message, MessageProviderState, OperationPart, PartContent,
    };

    fn assistant_operation(invocation: ToolInvocation) -> Message {
        Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::Operation(OperationPart::completed(
                0,
                invocation,
                "ok".to_owned(),
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange::default(),
            ))],
        )
    }

    #[test]
    fn completion_input_projection_keeps_role_parts_and_replay_state() {
        let mut message = Message::prompt_text(Role::Assistant, "hello");
        message.provider_state = Some(MessageProviderState {
            response_id: Some("resp_123".to_owned()),
            gemini_thought_signatures: [("part_1".to_owned(), "signature".to_owned())]
                .into_iter()
                .collect(),
            ..Default::default()
        });

        let input = project_completion_input(&message);

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
    fn ui_only_activity_never_enters_provider_history() {
        let mut message = assistant_operation(ToolInvocation::new(
            "session.compact",
            StructuredObject::default(),
        ));
        let Some(PartContent::Operation(operation)) = message.parts[0].content.as_mut() else {
            panic!("expected operation")
        };
        operation.set_ui_only(true);

        assert!(message.is_ui_only());
        assert!(message.as_text_lossy().is_empty());
        assert!(project_persisted(&message).is_empty());
        validate_provider_native_tool_history(std::slice::from_ref(&message))
            .expect("UI-only activity is outside provider history");
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
        invocation.tool_api_function = Some(ToolApiFunction::Call);
        let message = assistant_operation(invocation);

        validate_provider_native_tool_history(std::slice::from_ref(&message))
            .expect("valid Tool API history");
        let projected = project_persisted(&message);
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
        let message = assistant_operation(ToolInvocation::new(
            "agena.tools.help",
            StructuredObject::try_from(serde_json::json!({ "tool": "session.rename" }))
                .expect("structured help payload"),
        ));

        let error = validate_provider_native_tool_history(std::slice::from_ref(&message))
            .expect_err("dotted names must never become protocol identities");
        assert!(error.to_string().contains("no provider function identity"));
    }

    #[test]
    fn direct_execution_tool_replays_its_provider_function_name() {
        let mut invocation =
            ToolInvocation::new("agena.session.rename", StructuredObject::default());
        invocation.provider_function_name = Some("session_rename".to_string());
        let message = assistant_operation(invocation);

        validate_provider_native_tool_history(std::slice::from_ref(&message))
            .expect("direct execution tool has provider identity");
        let projected = project_persisted(&message);
        let WirePart::ToolCall { function, .. } = &projected[0] else {
            panic!("expected direct provider tool call")
        };
        assert_eq!(function.function_name(), "session_rename");
    }

    #[test]
    fn unadvertised_execution_tool_operation_is_rejected_as_provider_history() {
        let message = assistant_operation(ToolInvocation::new(
            "agena.session.rename",
            StructuredObject::default(),
        ));

        let error = validate_provider_native_tool_history(&[message])
            .expect_err("execution-tool call must fail");
        assert!(error.to_string().contains("no provider function identity"));
    }

    #[test]
    fn mismatched_tool_api_identity_is_rejected() {
        let mut invocation = ToolInvocation::new(
            ToolApiFunction::Help.function_name(),
            StructuredObject::default(),
        );
        invocation.tool_api_function = Some(ToolApiFunction::Call);
        let message = assistant_operation(invocation);

        let error =
            validate_provider_native_tool_history(&[message]).expect_err("mismatch must fail");
        assert!(
            error
                .to_string()
                .contains("must store only its exact protocol name")
        );
    }
}
