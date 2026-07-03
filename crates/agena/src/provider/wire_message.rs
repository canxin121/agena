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
                if exec.is_provider_native_only() {
                    continue;
                }
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

pub(crate) fn project_operation_output(status: ExecutionStatus, exec: &OperationPart) -> String {
    match status {
        ExecutionStatus::Pending | ExecutionStatus::InProgress | ExecutionStatus::Cancelled => {
            String::new()
        }
        ExecutionStatus::Completed => {
            structured_operation_output(exec).unwrap_or_else(|| exec.model_output.text.clone())
        }
        ExecutionStatus::Failed => exec
            .output_text()
            .or_else(|| exec.error_message())
            .unwrap_or_default()
            .to_string(),
    }
}

const MAX_MODEL_WEB_RESULT_SNIPPET_CHARS: usize = 400;
const MAX_MODEL_WEB_CRAWL_DOCUMENTS: usize = 20;
const MAX_MODEL_WEB_CRAWL_FAILURES: usize = 5;

fn structured_operation_output(exec: &OperationPart) -> Option<String> {
    structured_web_search_output(exec).or_else(|| structured_web_crawl_output(exec))
}

fn structured_web_search_output(exec: &OperationPart) -> Option<String> {
    let crate::tool::ToolPayloadOutput::WebSearch {
        query,
        backend,
        results,
    } = crate::tool::ToolPayloadOutput::from_tool_output(
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
    use chrono::Utc;
    use serde_json::json;

    use super::{WirePart, project};
    use crate::message::{
        ExecutionStatus, Message, MessagePart, OperationPart, PartContent, StructuredObject,
        TimeRange, ToolInvocation,
    };
    use crate::role::Role;
    use crate::tool::{ToolPayloadOutput, WebSearchHit};

    #[test]
    fn project_structures_local_web_search_results_for_model() {
        let created_at = Utc::now();
        let invocation = ToolInvocation::new(
            "web.search",
            StructuredObject::try_from(json!({ "query": "OpenAI Responses API" }))
                .expect("tool input"),
        );
        let details = ToolPayloadOutput::WebSearch {
            query: "OpenAI Responses API".to_string(),
            backend: "bing".to_string(),
            results: vec![WebSearchHit {
                title: "Responses API reference".to_string(),
                url: "https://platform.openai.com/docs/api-reference/responses".to_string(),
                snippet: Some(
                    "Build stateful interactions with text, image, and tool outputs.".to_string(),
                ),
            }],
        }
        .into_tool_output();
        let mut tool_part = MessagePart::with_content(
            1,
            0,
            created_at,
            ExecutionStatus::Completed,
            PartContent::Operation(OperationPart::completed(
                1,
                invocation,
                "Found 1 web search result. These are candidate links, not final evidence."
                    .to_string(),
                Vec::new(),
                Vec::new(),
                details,
                TimeRange::default(),
            )),
        );
        tool_part.operation_id = Some("call_1".to_string());

        let assistant = Message {
            id: 2,
            role: Role::Assistant,
            state: ExecutionStatus::Completed,
            parts: vec![tool_part],
            created_at,
            metadata: Default::default(),
            provider_state: None,
            usage: None,
        };

        let projected = project(&assistant);
        let Some(WirePart::ToolResult { output_json, .. }) = projected
            .iter()
            .find(|part| matches!(part, WirePart::ToolResult { .. }))
        else {
            panic!("expected projected tool result");
        };

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(output_json).expect("valid json"),
            json!({
                "query": "OpenAI Responses API",
                "backend": "bing",
                "results": [{
                    "title": "Responses API reference",
                    "url": "https://platform.openai.com/docs/api-reference/responses",
                    "snippet": "Build stateful interactions with text, image, and tool outputs."
                }]
            })
        );
        assert!(!output_json.contains("candidate links"));
    }

    #[test]
    fn project_structures_local_web_crawl_results_for_model() {
        let created_at = Utc::now();
        let invocation = ToolInvocation::new(
            "web.crawl",
            StructuredObject::try_from(json!({ "start_url": "https://example.com/docs" }))
                .expect("tool input"),
        );
        let details = crate::message::ToolOutput::from_json_payload(Some(&json!({
            "start_url": "https://example.com/docs",
            "engine": "spider",
            "rendered": false,
            "stored_count": 2,
            "cached_count": 1,
            "duplicate_count": 0,
            "near_duplicate_count": 0,
            "failure_count": 1,
            "total_documents": 3,
            "documents": [
                {
                    "title": "Docs Home",
                    "url": "https://example.com/docs",
                    "depth": 0,
                    "chunk_count": 2
                },
                {
                    "title": "Install",
                    "url": "https://example.com/docs/install",
                    "depth": 1,
                    "chunk_count": 1
                }
            ],
            "failures": ["https://example.com/docs/broken: http 500"]
        })))
        .expect("tool output");
        let mut tool_part = MessagePart::with_content(
            1,
            0,
            created_at,
            ExecutionStatus::Completed,
            PartContent::Operation(OperationPart::completed(
                1,
                invocation,
                "Crawled and indexed 2 pages.".to_string(),
                Vec::new(),
                Vec::new(),
                details,
                TimeRange::default(),
            )),
        );
        tool_part.operation_id = Some("call_2".to_string());

        let assistant = Message {
            id: 2,
            role: Role::Assistant,
            state: ExecutionStatus::Completed,
            parts: vec![tool_part],
            created_at,
            metadata: Default::default(),
            provider_state: None,
            usage: None,
        };

        let projected = project(&assistant);
        let Some(WirePart::ToolResult { output_json, .. }) = projected
            .iter()
            .find(|part| matches!(part, WirePart::ToolResult { .. }))
        else {
            panic!("expected projected tool result");
        };

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(output_json).expect("valid json"),
            json!({
                "start_url": "https://example.com/docs",
                "engine": "spider",
                "rendered": false,
                "stored_count": 2,
                "cached_count": 1,
                "duplicate_count": 0,
                "near_duplicate_count": 0,
                "failure_count": 1,
                "total_documents": 3,
                "documents_truncated": false,
                "documents": [
                    {
                        "title": "Docs Home",
                        "url": "https://example.com/docs",
                        "depth": 0,
                        "chunk_count": 2
                    },
                    {
                        "title": "Install",
                        "url": "https://example.com/docs/install",
                        "depth": 1,
                        "chunk_count": 1
                    }
                ],
                "failures_truncated": false,
                "failures": ["https://example.com/docs/broken: http 500"]
            })
        );
    }
}
