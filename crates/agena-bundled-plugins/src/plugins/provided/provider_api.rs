//! Resource and inference APIs share the active provider connection. Paths are
//! constructed from an allowlist; callers cannot supply endpoints or credentials.
mod accounting;
mod multipart;
pub(crate) mod realtime;
mod routes;
#[cfg(test)]
mod tests;
mod uploads;

use super::cloud_media::Service;
use super::official_service;
use agena_macros::ToolInput;
use agena_plugin_sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_sdk::{PluginError, Result, ToolInvokeOutput};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

const MAX_UPLOAD: u64 = 512 * 1024 * 1024;
const MAX_DOWNLOAD: usize = 512 * 1024 * 1024;
const MAX_INLINE_BINARY: usize = 96 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    #[default]
    List,
    Create,
    Get,
    Update,
    Delete,
    Download,
    Upload,
    Cancel,
    Wait,
    Search,
    AddFile,
    ListFiles,
    GetFile,
    UpdateFile,
    DeleteFile,
    DownloadFile,
    ImportFile,
    CreateBatch,
    GetBatch,
    CancelBatch,
    ListBatchFiles,
    Results,
    CreateVersion,
    ListVersions,
    GetVersion,
    DeleteVersion,
    Events,
    Items,
    Archive,
    AddPart,
    Complete,
    AddItems,
    GetItem,
    DeleteItem,
    CreateRun,
    ListRuns,
    GetRun,
    CancelRun,
    CreateEvent,
    Pause,
    Unpause,
    Run,
    Compact,
    StreamEvents,
    DeleteRun,
    Validate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub struct ResourceInput {
    /// Operation. Each tool documents its supported actions. No automatic retries.
    #[serde(default)]
    pub action: Action,
    /// Vendor resource ID, or its documented resource name (not an HTTP URL).
    #[serde(default)]
    pub id: Option<String>,
    /// Nested resource ID, such as a vector-store file, skill version or event.
    #[serde(default)]
    pub item_id: Option<String>,
    /// Third-level resource ID, e.g. an eval output item or a subagent turn.
    #[serde(default)]
    pub sub_item_id: Option<String>,
    /// Optional collection for platform/managed-agent operations; see tool help.
    #[serde(default)]
    pub resource: Option<String>,
    /// Defaults to the current session model for model-based requests. Use a
    /// model from this tool's vendor with the required capability; otherwise
    /// explicitly pass model. Resource list/get/delete operations need no model.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Opt in to a bounded inference event stream. Returns complete received
    /// events and a cursor; does not keep a background connection open.
    #[serde(default)]
    pub stream: bool,
    /// Official API request fields. model/stream are controlled by Agena.
    /// This is request JSON, never local commands or connection configuration.
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    /// Pagination, filters and other official query parameters.
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    /// Explicit local input file for multipart uploads/transcription.
    #[serde(default)]
    pub path: Option<String>,
    /// Additional local paths, e.g. the files of a custom Claude Skill.
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Optional workspace output path. Existing files are never overwritten.
    /// Omit to save downloads as managed attachments.
    #[serde(default)]
    pub output_path: Option<String>,
    /// Bound a wait call to 1..50 seconds. An unfinished task is returned as
    /// pending with its ID; call wait again rather than creating another task.
    #[serde(default = "default_wait")]
    pub wait_seconds: u32,
    /// Official beta feature names only; cannot override authentication headers.
    #[serde(default)]
    pub beta_headers: Vec<String>,
    /// Resume an event stream from the provider's last SSE event ID.
    #[serde(default)]
    pub last_event_id: Option<String>,
}
fn default_wait() -> u32 {
    20
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Files,
    Knowledge,
    Containers,
    Tasks,
    Batches,
    Skills,
    Caches,
    Operations,
    Inference,
    Images,
    Transcription,
    Speech,
    Video,
    Music,
    Embeddings,
    Moderation,
    Tokens,
    Agents,
    Platform,
}

pub(super) struct Route {
    method: reqwest::Method,
    path: String,
    model: bool,
    multipart: bool,
    binary: bool,
}

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::invalid_params(message.into())
}

fn ambiguous(message: impl std::fmt::Display) -> PluginError {
    let mut error = PluginError::internal(message);
    error.failure.retry = agena_failure::RetryDirective::Never;
    error.failure.recovery = agena_failure::RecoveryDirective::None;
    error
}

/// Reject path traversal, URL syntax and nested names in individual segments.
fn segment(value: &str, prefix: &str) -> Result<String> {
    let value = value.strip_prefix(prefix).unwrap_or(value);
    if value.is_empty()
        || value.len() > 256
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(invalid(
            "invalid vendor resource ID; supply an ID, not a URL or path",
        ));
    }
    Ok(value.to_owned())
}

pub async fn invoke(
    service: Service<'_>,
    family: Family,
    mut input: ResourceInput,
) -> Result<ToolInvokeOutput> {
    if input.action == Action::List
        && matches!(
            family,
            Family::Inference
                | Family::Images
                | Family::Transcription
                | Family::Speech
                | Family::Video
                | Family::Music
                | Family::Embeddings
                | Family::Moderation
                | Family::Tokens
        )
    {
        input.action = Action::Create;
    }
    let route = routes::resolve(service.provider, family, &input)?;
    validate_input(&input)?;
    if input.stream
        && (!matches!(family, Family::Inference | Family::Tasks) || input.action != Action::Create)
    {
        return Err(invalid(
            "stream is supported only for inference/response creation",
        ));
    }
    if input.stream && !(1..=50).contains(&input.wait_seconds) {
        return Err(invalid("wait_seconds must be 1..50 for event streams"));
    }
    if input.action == Action::Wait {
        if !(1..=50).contains(&input.wait_seconds) {
            return Err(invalid("wait_seconds must be 1..50"));
        }
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(input.wait_seconds as u64);
        let mut last: Option<Value> = None;
        let response = loop {
            let response =
                match tokio::time::timeout_at(deadline, request(&service, family, &input, &route))
                    .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => {
                        if let Some(mut response) = last {
                            response["poll_warning"] = json!(error.failure.user.fallback);
                            break response;
                        }
                        return Err(error);
                    }
                    Err(_) => {
                        let mut response = last.unwrap_or_else(
                            || json!({"response":{"id":input.id,"status":"pending"}}),
                        );
                        response["poll_warning"] =
                            json!("Polling deadline reached; no task was resubmitted.");
                        break response;
                    }
                };
            if !pending(&response) || tokio::time::Instant::now() >= deadline {
                break response;
            }
            last = Some(response);
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::time::sleep(remaining.min(std::time::Duration::from_secs(2))).await;
        };
        return output(&service, family, &input, response).await;
    }
    let response = request(&service, family, &input, &route).await?;
    output(&service, family, &input, response).await
}

fn validate_input(input: &ResourceInput) -> Result<()> {
    for key in [
        "model",
        "stream",
        "api_key",
        "base_url",
        "headers",
        "authorization",
    ] {
        if input.parameters.contains_key(key) {
            return Err(invalid(format!(
                "parameters.{key} is controlled by Agena; use the top-level model override"
            )));
        }
    }
    if input
        .last_event_id
        .as_ref()
        .is_some_and(|s| s.len() > 1024 || s.chars().any(char::is_control))
    {
        return Err(invalid("invalid last_event_id"));
    }
    if matches!(input.action, Action::Events | Action::StreamEvents)
        && !(1..=50).contains(&input.wait_seconds)
    {
        return Err(invalid("wait_seconds must be 1..50 for event streams"));
    }
    if input.paths.len() > 64 || input.beta_headers.len() > 16 || input.query.len() > 32 {
        return Err(invalid(
            "too many file paths, beta headers or query parameters",
        ));
    }
    for beta in &input.beta_headers {
        if beta.is_empty()
            || beta.len() > 128
            || !beta.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(invalid("invalid beta feature name"));
        }
    }
    for key in input.query.keys() {
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "key" | "api_key" | "access_token" | "authorization"
        ) {
            return Err(invalid("credentials must come from the current provider"));
        }
    }
    Ok(())
}

async fn request(
    service: &Service<'_>,
    family: Family,
    input: &ResourceInput,
    route: &Route,
) -> Result<Value> {
    let path = if route.path.contains("{model}") {
        let model = official_service::configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "provider API",
        )?;
        let model = model.strip_prefix("models/").unwrap_or(&model);
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err(invalid("invalid model"));
        }
        let encoded: String = url::form_urlencoded::byte_serialize(model.as_bytes()).collect();
        route.path.replace("{model}", &encoded)
    } else {
        route.path.clone()
    };
    let mut url = url::Url::parse(&official_service::endpoint(
        &service.connection.base_url,
        &path,
    )?)
    .map_err(|_| invalid("invalid provider endpoint"))?;
    if family == Family::Inference && service.provider == "claude" {
        if let Some(messages_url) = &service.connection.messages_url {
            url =
                url::Url::parse(messages_url).map_err(|_| invalid("invalid messages endpoint"))?;
        }
    }
    for (key, value) in &input.query {
        url.query_pairs_mut().append_pair(key, value);
    }
    if service.provider == "gemini" && route.binary {
        url.query_pairs_mut().append_pair("alt", "media");
    }
    official_service::validate_provider_endpoint(service.host, url.as_str()).await?;
    if family == Family::Knowledge && service.provider == "gemini" && input.action == Action::Upload
    {
        return uploads::file_search(service, input, &url).await;
    }
    let mut body = Value::Object(input.parameters.clone().into_iter().collect());
    if family == Family::Batches && service.provider == "claude" && input.action == Action::Create {
        let requests = body
            .get_mut("requests")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid("parameters.requests must contain the batch requests"))?;
        let model = official_service::configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "batch",
        )?;
        for request in requests {
            let params = request
                .get_mut("params")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| invalid("each batch request requires a params object"))?;
            params.entry("model").or_insert_with(|| json!(model));
        }
    }
    if family == Family::Agents && input.action == Action::Create {
        if service.provider == "chatgpt"
            && input.resource.as_deref().is_none_or(|r| r == "sessions")
        {
            body.as_object_mut()
                .unwrap()
                .entry("agent")
                .or_insert_with(|| json!({}));
            if input.model.is_some() && !body["agent"].is_object() {
                return Err(invalid(
                    "a stored agent reference retains its model; use an inline agent object to override model",
                ));
            }
        }
        if service.provider == "gemini"
            && (input.model.is_some() || !body.as_object().unwrap().contains_key("base_agent"))
        {
            body.as_object_mut()
                .unwrap()
                .entry("agent_config")
                .or_insert_with(|| json!({"type":"antigravity"}));
        }
        let nested = match service.provider {
            "chatgpt"
                if input.resource.as_deref().is_none_or(|r| r == "sessions")
                    && body.get("agent").is_some_and(Value::is_object) =>
            {
                Some("agent")
            }
            "gemini" if body.get("agent_config").is_some_and(Value::is_object) => {
                Some("agent_config")
            }
            _ => None,
        };
        if let Some(key) = nested {
            let model = official_service::configured_model(
                input.model.clone(),
                service.connection.model.as_deref(),
                "agent",
            )?;
            let config = body[key].as_object_mut().unwrap();
            if input.model.is_some() || !config.contains_key("model") {
                config.insert("model".into(), json!(model));
            }
        }
    }
    if route.model && !route.path.contains("{model}") {
        if service.provider == "gemini" && body.get("agent").is_some() {
            if input.model.is_some() {
                return Err(invalid("agent and model are mutually exclusive"));
            }
        } else {
            let model = official_service::configured_model(
                input.model.clone(),
                service.connection.model.as_deref(),
                "provider API",
            )?;
            body["model"] = json!(if service.provider == "gemini"
                && matches!(
                    family,
                    Family::Embeddings | Family::Caches | Family::Batches
                ) {
                format!("models/{}", model.strip_prefix("models/").unwrap_or(&model))
            } else {
                model
            });
        }
    }
    if service.provider == "gemini"
        && family == Family::Embeddings
        && input.action == Action::CreateBatch
    {
        let model = official_service::configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "batch embeddings",
        )?;
        let name = format!("models/{}", model.strip_prefix("models/").unwrap_or(&model));
        let requests = body
            .get_mut("requests")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid("parameters.requests must contain the embedding requests"))?;
        for request in requests {
            let request = request
                .as_object_mut()
                .ok_or_else(|| invalid("each embedding request must be an object"))?;
            if input.model.is_some() || !request.contains_key("model") {
                request.insert("model".into(), json!(name));
            }
        }
    }
    if let Some(prompt) = &input.prompt {
        match (service.provider, family) {
            ("chatgpt", Family::Images | Family::Transcription) => body["prompt"] = json!(prompt),
            ("chatgpt", Family::Speech) => body["input"] = json!(prompt),
            ("chatgpt", Family::Tasks) if input.action == Action::Compact => {
                body["input"] = json!([{"role":"user","content":prompt}])
            }
            ("claude", Family::Inference | Family::Tokens) => {
                let messages = body
                    .as_object_mut()
                    .unwrap()
                    .entry("messages")
                    .or_insert_with(|| json!([]));
                messages
                    .as_array_mut()
                    .ok_or_else(|| invalid("parameters.messages must be an array"))?
                    .push(json!({"role":"user","content":prompt}));
            }
            ("gemini", Family::Embeddings) => body["content"] = json!({"parts":[{"text":prompt}]}),
            ("gemini", Family::Tokens) => body["contents"] = json!([{"parts":[{"text":prompt}]}]),
            _ => body["input"] = json!(prompt),
        }
    }
    if service.provider == "gemini"
        && matches!(family, Family::Speech | Family::Video | Family::Music)
    {
        let format = body
            .as_object_mut()
            .unwrap()
            .entry("response_format")
            .or_insert_with(|| json!({}));
        let format = format
            .as_object_mut()
            .ok_or_else(|| invalid("parameters.response_format must be an object"))?;
        format.insert(
            "type".into(),
            json!(if family == Family::Video {
                "video"
            } else {
                "audio"
            }),
        );
    }
    if matches!(
        family,
        Family::Inference | Family::Tasks | Family::Speech | Family::Video | Family::Music
    ) && route.method == reqwest::Method::POST
        && input.action == Action::Create
    {
        body["stream"] = json!(input.stream);
        if service.provider == "claude" {
            body.as_object_mut()
                .unwrap()
                .entry("max_tokens")
                .or_insert(json!(4096));
        }
    }
    let mut req = service
        .client
        .request(route.method.clone(), url)
        .headers(official_service::provider_headers(&service.connection)?)
        .header("x-agena-provider-api", service.provider);
    if let Some(last_event_id) = &input.last_event_id {
        req = req.header("last-event-id", last_event_id);
    }
    if family == Family::Agents && service.provider == "chatgpt" {
        req = req.header("openai-beta", "agents=v1");
    }
    if service.provider == "claude" {
        req = req.header("anthropic-version", service.anthropic_version);
        let mut betas = input.beta_headers.clone();
        if family == Family::Files {
            betas.push("files-api-2025-04-14".into());
        }
        if family == Family::Skills {
            betas.push("skills-2025-10-02".into());
        }
        if !betas.is_empty() {
            req = req.header("anthropic-beta", betas.join(","));
        }
    }
    if route.multipart {
        let mut form = reqwest::multipart::Form::new();
        for (key, value) in multipart::fields(body.as_object().unwrap())? {
            form = form.text(key, value);
        }
        let paths: Vec<_> = input.path.iter().chain(&input.paths).collect();
        if paths.is_empty() {
            return Err(invalid("this operation requires path or paths"));
        }
        let voice_recording = family == Family::Platform
            && matches!(
                input.resource.as_deref(),
                Some("audio/voices" | "audio/voice_consents")
            );
        if paths.len() != 1 && (voice_recording || family == Family::Transcription) {
            return Err(invalid("provide exactly one recording for this operation"));
        }
        let upload_limit = if voice_recording {
            10 * 1024 * 1024
        } else {
            MAX_UPLOAD
        };
        let mut total = 0u64;
        for path in paths {
            let local = authorized_input(
                service,
                path,
                input.expected_sha256.as_deref(),
                upload_limit,
            )
            .await?;
            total = total.saturating_add(local.len() as u64);
            if total > MAX_UPLOAD {
                return Err(invalid("upload inputs exceed 512 MiB"));
            }
            let skill_path = std::path::Path::new(path)
                .strip_prefix(service.root)
                .unwrap_or(std::path::Path::new(path));
            let filename = if family == Family::Skills
                && !skill_path.is_absolute()
                && skill_path
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
            {
                skill_path.to_string_lossy().replace('\\', "/")
            } else {
                PathBuf::from(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("input")
                    .to_owned()
            };
            let field = if family == Family::Skills {
                "files[]"
            } else if family == Family::Platform && input.resource.as_deref() == Some("uploads") {
                "data"
            } else if family == Family::Platform
                && input.resource.as_deref() == Some("audio/voices")
            {
                "audio_sample"
            } else if family == Family::Platform
                && input.resource.as_deref() == Some("audio/voice_consents")
            {
                "recording"
            } else {
                "file"
            };
            if matches!(field, "recording" | "audio_sample") && local.len() > 10 * 1024 * 1024 {
                return Err(invalid("voice recordings must not exceed 10 MiB"));
            }
            let mime = multipart::file_mime(&filename);
            let part = reqwest::multipart::Part::bytes(local)
                .file_name(filename)
                .mime_str(mime)
                .map_err(|e| PluginError::internal_error(&e))?;
            form = form.part(field, part);
        }
        req = req.multipart(form);
    } else if route.method != reqwest::Method::GET && route.method != reqwest::Method::DELETE {
        req = req.json(&body);
    }
    let response = req.send().await.map_err(|e| ambiguous(format!(
        "provider request failed; remote acceptance may be unknown, do not automatically repeat: {}", e.without_url())))?;
    if response.status().is_success()
        && response
            .headers()
            .get("content-type")
            .and_then(|s| s.to_str().ok())
            .is_some_and(|s| s.contains("text/event-stream"))
    {
        if !(1..=50).contains(&input.wait_seconds) {
            return Err(invalid("wait_seconds must be 1..50 for event streams"));
        }
        return read_events(response, input.wait_seconds).await;
    }
    let textual_transcription = family == Family::Transcription
        && response.status().is_success()
        && !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("json");
    if (route.binary || textual_transcription) && response.status().is_success() {
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|s| s.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let bytes = response_bytes(response, MAX_DOWNLOAD).await?;
        let attachment = save_binary(service, input.output_path.as_deref(), &mime, bytes).await?;
        return Ok(json!({"download":attachment}));
    }
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(json!({"deleted":true}));
    }
    // Batch results are often JSONL, not a JSON object.
    if input.action == Action::Results && response.status().is_success() {
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|s| s.to_str().ok())
            .unwrap_or("application/x-ndjson")
            .to_owned();
        let bytes = response_bytes(response, MAX_DOWNLOAD).await?;
        let usage_entries = if service.provider == "claude" {
            accounting::batch_results(&bytes)?
        } else {
            Vec::new()
        };
        let attachment = save_binary(service, input.output_path.as_deref(), &mime, bytes).await?;
        return Ok(json!({"download":attachment,"usage_entries":usage_entries}));
    }
    let (status, request_id, value) =
        official_service::read_json_response_bounded(response, service.provider, "resource API")
            .await
            .map_err(|mut error| {
                if route.method != reqwest::Method::GET {
                    error.failure.retry = agena_failure::RetryDirective::Never;
                }
                error
            })?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("provider rejected the operation");
        return Err(PluginError::internal(format!(
            "{} API HTTP {status}: {message}",
            service.provider
        )));
    }
    Ok(json!({"request_id":request_id,"response":value}))
}

// A tool invocation cannot keep a live event stream open indefinitely. Retain
// complete events at the deadline; continuation uses the provider resource ID.
async fn read_events(response: reqwest::Response, seconds: u32) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds as u64);
    let mut stream = response.bytes_stream();
    let mut data = Vec::new();
    let mut ended = false;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if data.len().saturating_add(chunk.len()) > 128 * 1024 {
                    break;
                }
                data.extend(chunk);
            }
            Ok(None) => {
                ended = true;
                break;
            }
            _ => break,
        }
    }
    Ok(decode_events(&data, ended))
}

fn decode_events(data: &[u8], ended: bool) -> Value {
    let text = String::from_utf8_lossy(data).replace("\r\n", "\n");
    let mut events = Vec::new();
    let mut envelopes = Vec::new();
    let mut cursor = None;
    let mut truncated = false;
    for frame in text
        .split_inclusive("\n\n")
        .filter(|frame| frame.ends_with("\n\n") || ended)
    {
        if events.len() == 256 {
            truncated = true;
            break;
        }
        let mut lines = Vec::new();
        let mut metadata = serde_json::Map::new();
        for line in frame.lines() {
            if line.starts_with(':') {
                continue;
            }
            let (key, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match key {
                "data" => lines.push(value),
                "id" if !value.contains('\0') => {
                    cursor = Some(value.to_owned());
                    metadata.insert("id".into(), json!(value));
                }
                "event" => {
                    metadata.insert("event".into(), json!(value));
                }
                "retry" => {
                    if let Ok(ms) = value.parse::<u64>() {
                        metadata.insert("retry".into(), json!(ms));
                    }
                }
                _ => {}
            }
        }
        if lines.is_empty() {
            continue;
        }
        let data = lines.join("\n");
        if data == "[DONE]" {
            continue;
        }
        let value = serde_json::from_str(&data).unwrap_or_else(|_| json!({"data":data}));
        metadata.insert("event_index".into(), json!(events.len()));
        events.push(value);
        envelopes.push(Value::Object(metadata));
    }
    json!({"response":{"events":events,"sse_events":envelopes,"last_event_id":cursor,
        "stream_ended":ended,"partial":!ended || truncated}})
}

pub(super) async fn authorized_input(
    service: &Service<'_>,
    path: &str,
    expected: Option<&str>,
    limit: u64,
) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let path = service.root.join(path);
    crate::permissions::require_path(
        service.host.as_ref(),
        agena_domain::PathKind::Read,
        &path.to_string_lossy(),
    )
    .await?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| PluginError::internal_error(&e))?;
    let before = file
        .metadata()
        .await
        .map_err(|e| PluginError::internal_error(&e))?;
    if !before.is_file() || before.len() == 0 || before.len() > limit {
        return Err(invalid(format!(
            "input must be a nonempty regular file, at most {limit} bytes"
        )));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| PluginError::internal_error(&e))?;
    let after = file
        .metadata()
        .await
        .map_err(|e| PluginError::internal_error(&e))?;
    if bytes.len() as u64 > limit
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(invalid(
            "input changed during reading or exceeded its size limit",
        ));
    }
    if expected.is_some_and(|e| !e.eq_ignore_ascii_case(&hex::encode(Sha256::digest(&bytes)))) {
        return Err(invalid("input SHA256 mismatch; nothing was sent"));
    }
    Ok(bytes)
}

async fn response_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(invalid("provider file exceeds download limit"));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| PluginError::internal_error(&e.without_url()))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(invalid("provider file exceeds download limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) async fn save_binary(
    service: &Service<'_>,
    output: Option<&str>,
    mime: &str,
    bytes: Vec<u8>,
) -> Result<AttachmentItem> {
    let raw_pcm = mime.to_ascii_lowercase().starts_with("audio/pcm")
        || mime.to_ascii_lowercase().starts_with("audio/l16");
    let (mime, bytes) = if raw_pcm {
        let rate = mime
            .split(';')
            .find_map(|v| {
                v.trim()
                    .strip_prefix("rate=")
                    .and_then(|v| v.parse::<u32>().ok())
            })
            .unwrap_or(24000);
        if !(8000..=192000).contains(&rate) || bytes.len() % 2 != 0 {
            return Err(invalid("invalid provider PCM format"));
        }
        ("audio/wav", realtime::wav(bytes, rate))
    } else if mime.starts_with("image/") && bytes.starts_with(b"\xff\xd8\xff") {
        ("image/jpeg", bytes)
    } else if mime.starts_with("image/")
        && bytes.starts_with(b"RIFF")
        && bytes.get(8..12) == Some(b"WEBP")
    {
        ("image/webp", bytes)
    } else if mime.starts_with("audio/")
        && (bytes.starts_with(b"ID3")
            || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0))
    {
        ("audio/mpeg", bytes)
    } else {
        (mime, bytes)
    };
    let kind = if mime.starts_with("image/") {
        AttachmentKind::Image
    } else if mime.starts_with("audio/") {
        AttachmentKind::Audio
    } else if mime.starts_with("video/") {
        AttachmentKind::Video
    } else if mime.starts_with("application/pdf") {
        AttachmentKind::Pdf
    } else {
        AttachmentKind::File
    };
    let extension = match mime.split(';').next().unwrap_or(mime).trim() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "application/pdf" => "pdf",
        "application/json" => "json",
        "application/x-ndjson" | "application/jsonl" => "jsonl",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/vtt" => "vtt",
        "application/x-subrip" => "srt",
        "audio/opus" => "opus",
        "audio/aac" => "aac",
        _ => "bin",
    };
    let path = if let Some(output) = output {
        let path = service.root.join(output);
        crate::permissions::require_path(
            service.host.as_ref(),
            agena_domain::PathKind::Write,
            &path.to_string_lossy(),
        )
        .await?;
        path
    } else {
        agena_storage_layout::StorageLayout::current()
            .artifacts_dir(service.root)
            .join("provider-tools")
            .join(service.provider)
            .join(format!("{}.{}", uuid::Uuid::new_v4().simple(), extension))
    };
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let size_bytes = bytes.len() as u64;
    crate::artifact_file::persist_new(path.clone(), bytes, "provider output").await?;
    Ok(AttachmentItem {
        kind,
        mime: mime.into(),
        source: AttachmentSource::LocalPath {
            path: path.to_string_lossy().into_owned(),
        },
        filename: path.file_name().and_then(|n| n.to_str()).map(str::to_owned),
        title: Some("Provider output".into()),
        size_bytes: Some(size_bytes),
        sha256: Some(sha256),
        width: None,
        height: None,
        duration_ms: None,
        page_count: None,
    })
}

fn pending(value: &Value) -> bool {
    let value = value.get("response").unwrap_or(value);
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return false;
    }
    if let Some(done) = value.get("done").and_then(Value::as_bool) {
        return !done;
    }
    let state = value
        .get("status")
        .or_else(|| value.get("processing_status"))
        .or_else(|| value.get("state"))
        .or_else(|| value.pointer("/metadata/state"))
        .and_then(Value::as_str);
    if matches!(
        state,
        Some(
            "completed"
                | "succeeded"
                | "failed"
                | "cancelled"
                | "expired"
                | "JOB_STATE_SUCCEEDED"
                | "JOB_STATE_FAILED"
                | "JOB_STATE_CANCELLED"
                | "JOB_STATE_EXPIRED"
                | "BATCH_STATE_SUCCEEDED"
                | "BATCH_STATE_FAILED"
                | "BATCH_STATE_CANCELLED"
                | "BATCH_STATE_EXPIRED"
        )
    ) {
        return false;
    }
    if value.get("done").is_none()
        && value.get("name").and_then(Value::as_str).is_some_and(|n| {
            n.starts_with("operations/") || n.contains("/operations/") || n.starts_with("batches/")
        })
        && value.get("response").is_none()
    {
        return true;
    }
    matches!(
        state,
        Some(
            "uploaded"
                | "queued"
                | "in_progress"
                | "running"
                | "pending"
                | "cancelling"
                | "validating"
                | "validating_files"
                | "finalizing"
                | "pausing"
                | "PROCESSING"
                | "VALIDATING"
                | "JOB_STATE_PENDING"
                | "JOB_STATE_RUNNING"
                | "BATCH_STATE_PENDING"
                | "BATCH_STATE_RUNNING"
        )
    )
}

async fn output(
    service: &Service<'_>,
    family: Family,
    input: &ResourceInput,
    mut response: Value,
) -> Result<ToolInvokeOutput> {
    let mut warnings = Vec::new();
    let usage = accounting::metadata(service, family, input, &response, &mut warnings).await?;
    if let Some(object) = response.as_object_mut() {
        object.remove("usage_entries");
    }
    let mut attachments = Vec::new();
    if let Some(download) = response.get("download") {
        attachments.push(
            serde_json::from_value(download.clone())
                .map_err(|e| PluginError::internal_error(&e))?,
        );
    }
    let mut candidates = Vec::new();
    collect_binary(&mut response, &mut candidates, 0);
    for (mime, encoded) in candidates {
        let result = if encoded.len() > MAX_INLINE_BINARY * 4 / 3 + 4 {
            Err(invalid("inline output exceeds 96 MiB"))
        } else {
            STANDARD
                .decode(encoded)
                .map_err(|_| invalid("provider returned invalid base64"))
        };
        match result {
            Ok(bytes) => match save_binary(
                service,
                if attachments.is_empty() {
                    input.output_path.as_deref()
                } else {
                    None
                },
                &mime,
                bytes,
            )
            .await
            {
                Ok(attachment) => attachments.push(attachment),
                Err(error) => warnings.push(error.failure.user.fallback),
            },
            Err(error) => warnings.push(error.failure.user.fallback),
        }
    }
    let pending = pending(&response);
    let vendor_response = response.get("response").unwrap_or(&response);
    let resource_id = vendor_response
        .get("id")
        .or_else(|| vendor_response.get("name"))
        .cloned()
        .or_else(|| input.id.as_ref().map(|id| json!(id)));
    let provider_status = vendor_response
        .get("status")
        .or_else(|| vendor_response.get("stop_reason"))
        .cloned();
    let receipt =
        serde_json::to_vec_pretty(&response).map_err(|e| PluginError::internal_error(&e))?;
    let receipt_path = agena_storage_layout::StorageLayout::current()
        .artifacts_dir(service.root)
        .join("provider-tools")
        .join(service.provider)
        .join(format!("{}.json", uuid::Uuid::new_v4().simple()));
    let persisted =
        crate::artifact_file::persist_new(receipt_path.clone(), receipt, "provider API receipt")
            .await;
    if let Err(error) = &persisted {
        warnings.push(error.failure.user.fallback.clone());
    }
    let text =
        serde_json::to_string_pretty(&response).map_err(|e| PluginError::internal_error(&e))?;
    let preview = official_service::truncate_text(&text, 32 * 1024);
    let small_response = if text.len() <= 128 * 1024 {
        response
    } else {
        json!({"truncated":true,"preview":preview})
    };
    Ok(ToolInvokeOutput::from_parts(
        format!("{} {family:?}", service.provider),
        if pending {
            "Provider task pending; retain the resource ID"
        } else {
            "Provider response received"
        },
        preview,
        Some(
            json!({"provider_id":service.connection.provider_id,"provider":service.provider,"response":small_response,
            "execution_location":"vendor_cloud","local_workspace_automatically_available":false,
            "pending":pending,"resource_id":resource_id,"provider_status":provider_status,
            "receipt_path":persisted.is_ok().then(||receipt_path.to_string_lossy().into_owned()),
            "persistence_warnings":warnings,"safe_to_reexecute_automatically":false,"local_actions_executed":false}),
        ),
        usage,
        attachments,
    ))
}

// Persist inline images, audio and video, and remove base64 from receipts/model
// context. Provider file URIs remain references; no authenticated cross-origin
// fetch or implicit fallback to an official endpoint is performed.
fn collect_binary(value: &mut Value, output: &mut Vec<(String, String)>, depth: usize) {
    if depth > 64 {
        return;
    }
    match value {
        Value::Object(object) => {
            let mime = object
                .get("mime_type")
                .or_else(|| object.get("mimeType"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    object
                        .get("type")
                        .and_then(Value::as_str)
                        .and_then(|t| match t {
                            "audio" => Some("audio/wav".into()),
                            "video" => Some("video/mp4".into()),
                            "image" => Some("image/png".into()),
                            _ => None,
                        })
                });
            if let Some(Value::String(encoded)) = object.get_mut("b64_json") {
                output.push((
                    "image/png".into(),
                    std::mem::replace(encoded, "[saved as attachment]".into()),
                ));
            }
            if let Some(mime) = mime.filter(|m| {
                m.starts_with("image/") || m.starts_with("audio/") || m.starts_with("video/")
            }) {
                for key in ["data", "result"] {
                    if let Some(Value::String(encoded)) = object.get_mut(key) {
                        output.push((
                            mime.clone(),
                            std::mem::replace(encoded, "[saved as attachment]".into()),
                        ));
                    }
                }
            }
            if object.get("type").and_then(Value::as_str) == Some("image_generation_call") {
                if let Some(Value::String(encoded)) = object.get_mut("result") {
                    output.push((
                        "image/png".into(),
                        std::mem::replace(encoded, "[saved as attachment]".into()),
                    ));
                }
            }
            for value in object.values_mut() {
                collect_binary(value, output, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_binary(item, output, depth + 1);
            }
        }
        _ => {}
    }
}
