use std::collections::{BTreeMap, HashMap};

use futures_core::Stream;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::error::{AppError, ProviderErrorKind};
use crate::model::{ModelId, ProviderId};
use crate::provider::{
    CompletionFinishReason, CompletionResponse, CompletionStreamEvent, CompletionToolCall,
    CompletionUsage, ManagedCredential, should_retry_credential,
};

pub(crate) const ADAPTER_LOG_TARGET: &str = "agena::adapter";
const ADAPTER_LOG_STRING_LIMIT: usize = 2_048;

// ─── Header / fingerprint helpers ────────────────────────────────────────────

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

pub(crate) fn request_shape_fingerprint<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_slice());
    hex::encode(hasher.finalize())
}

// ─── URL / auth helpers ───────────────────────────────────────────────────────

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

/// Apply provider-configured `extra_headers` and consult the plugin host's
/// `chat.headers` hook chain.
pub fn resolved_request_headers(
    provider_id: &str,
    extra_headers: &HashMap<String, String>,
) -> BTreeMap<String, String> {
    let mut combined: BTreeMap<String, String> = extra_headers
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

    combined
}

pub fn apply_resolved_request_headers(
    mut req: reqwest::RequestBuilder,
    headers: &BTreeMap<String, String>,
) -> reqwest::RequestBuilder {
    for (key, value) in headers {
        req = req.header(key.as_str(), value.as_str());
    }
    req
}

pub fn merged_request_headers(
    base_headers: &HashMap<String, String>,
    request_headers: &BTreeMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = base_headers.clone();
    for (key, value) in request_headers {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

pub fn serialize_request_body_with_patch(
    body: &impl serde::Serialize,
    patch: &BTreeMap<String, serde_json::Value>,
) -> Result<serde_json::Value, AppError> {
    let mut value = serde_json::to_value(body).map_err(AppError::from)?;
    if patch.is_empty() {
        return Ok(value);
    }

    let serde_json::Value::Object(target) = &mut value else {
        return Err(AppError::Config(
            "request body patch can only be applied to JSON object bodies".to_owned(),
        ));
    };

    merge_json_object_patch_map(target, patch);
    Ok(value)
}

pub fn merge_json_object_patch_map(
    target: &mut serde_json::Map<String, serde_json::Value>,
    patch: &BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in patch {
        match target.get_mut(key) {
            Some(current) => merge_json_value(current, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_json_value(current: &mut serde_json::Value, patch: &serde_json::Value) {
    match (current, patch) {
        (serde_json::Value::Object(current), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match current.get_mut(key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        current.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (current, patch) => *current = patch.clone(),
    }
}

// ─── HTTP response helpers ────────────────────────────────────────────────────

pub async fn parse_json_response_logged<T: DeserializeOwned>(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    response: reqwest::Response,
) -> Result<T, AppError> {
    if response.status().is_success() {
        ensure_response_content_type(provider_id, &response, "application/json")?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await?;
        adapter_log_http_response_text(
            provider_id,
            adapter_kind,
            operation,
            status,
            &headers,
            body.as_str(),
        );
        return serde_json::from_str::<T>(body.as_str()).map_err(AppError::from);
    }
    Err(
        http_status_error_from_response_logged(provider_id, adapter_kind, operation, response)
            .await,
    )
}

pub fn ensure_response_content_type(
    provider_id: &str,
    response: &reqwest::Response,
    expected_prefix: &str,
) -> Result<(), AppError> {
    let actual = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<missing>");
    if actual
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected_prefix))
    {
        return Ok(());
    }

    Err(AppError::Provider(format!(
        "{provider_id} returned unexpected content-type `{actual}` (expected {expected_prefix})"
    )))
}

pub fn parse_json_value<T: DeserializeOwned>(
    provider_id: &str,
    context: &str,
    value: serde_json::Value,
) -> Result<T, AppError> {
    serde_json::from_value(value).map_err(|err| {
        AppError::Provider(format!(
            "{provider_id} returned invalid {context} payload: {err}"
        ))
    })
}

pub async fn http_status_error_from_response_logged(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    response: reqwest::Response,
) -> AppError {
    let status = response.status();
    let headers = response.headers().clone();
    let raw_body = response
        .text()
        .await
        .unwrap_or_else(|_| "<empty>".to_owned());

    adapter_log_http_response_text(
        provider_id,
        adapter_kind,
        operation,
        status,
        &headers,
        raw_body.as_str(),
    );

    let mut body = serde_json::from_str::<ProviderErrorEnvelope>(&raw_body)
        .map(|parsed| {
            let mut message = parsed.error.message;
            if let Some(kind) = parsed.error.kind {
                message.push_str(&format!(" (type={kind})"));
            }
            if let Some(code) = parsed.error.code {
                message.push_str(&format!(" (code={code})"));
            }
            message
        })
        .unwrap_or(raw_body);

    let upstream_refs = [
        response_header_value(&headers, "x-request-id")
            .map(|value| format!("x-request-id={value}")),
        response_header_value(&headers, "cf-ray").map(|value| format!("cf-ray={value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !upstream_refs.is_empty() {
        body.push_str(" [");
        body.push_str(upstream_refs.join(", ").as_str());
        body.push(']');
    }

    let classified = classify_http_error(provider_id, status, body.as_str());
    AppError::HttpStatus {
        provider: provider_id.to_owned(),
        status,
        body,
        kind: classified.kind,
        retryable: classified.retryable,
    }
}

fn response_header_value(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn adapter_log_http_request_json<I, K, V>(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    method: &str,
    url: &str,
    headers: I,
    body: Option<&serde_json::Value>,
) where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    if !tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::DEBUG) {
        return;
    }

    let sanitized_url = sanitize_url(url);
    let sanitized_headers = sanitize_headers(headers);
    let body_fingerprint = body.map(request_shape_fingerprint);
    let header_count = sanitized_headers
        .as_object()
        .map(|object| object.len())
        .unwrap_or_default();

    tracing::debug!(
        target: ADAPTER_LOG_TARGET,
        provider = provider_id,
        adapter = adapter_kind,
        operation,
        method,
        url = sanitized_url.as_str(),
        header_count,
        has_body = body.is_some(),
        body_fingerprint = body_fingerprint.as_deref().unwrap_or(""),
        "adapter http request"
    );

    if tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::TRACE) {
        let headers_json = serde_json::to_string_pretty(&sanitized_headers).unwrap_or_default();
        let body_json = body.map(sanitize_json_value);
        let body_text = body_json
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_default();
        tracing::trace!(
            target: ADAPTER_LOG_TARGET,
            provider = provider_id,
            adapter = adapter_kind,
            operation,
            request_headers = headers_json.as_str(),
            request_body = body_text.as_str(),
            "adapter http request payload"
        );
    }
}

pub fn adapter_log_http_response_open(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) {
    if !tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::DEBUG) {
        return;
    }

    let sanitized_headers = sanitize_header_map(headers);
    let header_count = sanitized_headers
        .as_object()
        .map(|object| object.len())
        .unwrap_or_default();
    let content_type = response_header_value(headers, reqwest::header::CONTENT_TYPE.as_str())
        .unwrap_or_else(|| "<missing>".to_owned());

    tracing::debug!(
        target: ADAPTER_LOG_TARGET,
        provider = provider_id,
        adapter = adapter_kind,
        operation,
        status = status.as_u16(),
        content_type = content_type.as_str(),
        header_count,
        "adapter http response opened"
    );

    if tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::TRACE) {
        let headers_json = serde_json::to_string_pretty(&sanitized_headers).unwrap_or_default();
        tracing::trace!(
            target: ADAPTER_LOG_TARGET,
            provider = provider_id,
            adapter = adapter_kind,
            operation,
            response_headers = headers_json.as_str(),
            "adapter http response headers"
        );
    }
}

pub fn adapter_log_stream_event(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    event: &serde_json::Value,
) {
    if !tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::TRACE) {
        return;
    }

    let sanitized = sanitize_json_value(event);
    let event_text = serde_json::to_string_pretty(&sanitized).unwrap_or_default();
    tracing::trace!(
        target: ADAPTER_LOG_TARGET,
        provider = provider_id,
        adapter = adapter_kind,
        operation,
        stream_event = event_text.as_str(),
        "adapter stream event"
    );
}

fn adapter_log_http_response_text(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) {
    if !tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::DEBUG) {
        return;
    }

    let sanitized_headers = sanitize_header_map(headers);
    let header_count = sanitized_headers
        .as_object()
        .map(|object| object.len())
        .unwrap_or_default();
    let content_type = response_header_value(headers, reqwest::header::CONTENT_TYPE.as_str())
        .unwrap_or_else(|| "<missing>".to_owned());
    let body_chars = body.chars().count();

    tracing::debug!(
        target: ADAPTER_LOG_TARGET,
        provider = provider_id,
        adapter = adapter_kind,
        operation,
        status = status.as_u16(),
        content_type = content_type.as_str(),
        header_count,
        body_chars,
        "adapter http response"
    );

    if tracing::enabled!(target: ADAPTER_LOG_TARGET, tracing::Level::TRACE) {
        let headers_json = serde_json::to_string_pretty(&sanitized_headers).unwrap_or_default();
        let body_text = sanitize_response_body_text(body);
        tracing::trace!(
            target: ADAPTER_LOG_TARGET,
            provider = provider_id,
            adapter = adapter_kind,
            operation,
            response_headers = headers_json.as_str(),
            response_body = body_text.as_str(),
            "adapter http response payload"
        );
    }
}

fn sanitize_response_body_text(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        return serde_json::to_string_pretty(&sanitize_json_value(&value))
            .unwrap_or_else(|_| truncate_for_log(body));
    }
    truncate_for_log(body)
}

fn sanitize_header_map(headers: &reqwest::header::HeaderMap) -> serde_json::Value {
    sanitize_headers(headers.iter().filter_map(|(key, value)| {
        value
            .to_str()
            .ok()
            .map(|text| (key.as_str().to_owned(), text.to_owned()))
    }))
}

fn sanitize_headers<I, K, V>(headers: I) -> serde_json::Value
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut map = serde_json::Map::new();
    for (key, value) in headers {
        let key = key.as_ref().to_owned();
        map.insert(
            key.clone(),
            serde_json::Value::String(sanitize_named_string(Some(key.as_str()), value.as_ref())),
        );
    }
    serde_json::Value::Object(map)
}

fn sanitize_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return truncate_for_log(url);
    };

    let pairs = parsed
        .query_pairs()
        .map(|(key, value)| {
            let key = key.to_string();
            let value = if is_sensitive_key(key.as_str()) {
                redacted_marker(value.len())
            } else {
                sanitize_named_string(Some(key.as_str()), value.as_ref())
            };
            (key, value)
        })
        .collect::<Vec<_>>();

    if !pairs.is_empty() {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in pairs {
            serializer.append_pair(key.as_str(), value.as_str());
        }
        parsed.set_query(Some(serializer.finish().as_str()));
    }

    truncate_for_log(parsed.as_str())
}

fn sanitize_json_value(value: &serde_json::Value) -> serde_json::Value {
    sanitize_json_value_with_key(None, value)
}

fn sanitize_json_value_with_key(
    key_hint: Option<&str>,
    value: &serde_json::Value,
) -> serde_json::Value {
    if key_hint.is_some_and(is_sensitive_key) {
        return serde_json::Value::String(redacted_marker(serialized_len(value)));
    }

    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        sanitize_json_value_with_key(Some(key.as_str()), value),
                    )
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| sanitize_json_value_with_key(key_hint, item))
                .collect(),
        ),
        serde_json::Value::String(text) => {
            serde_json::Value::String(sanitize_named_string(key_hint, text))
        }
        _ => value.clone(),
    }
}

fn sanitize_named_string(key_hint: Option<&str>, value: &str) -> String {
    if key_hint.is_some_and(is_sensitive_key) {
        return redacted_marker(value.chars().count());
    }
    truncate_for_log(value)
}

fn truncate_for_log(value: &str) -> String {
    let char_count = value.chars().count();
    if char_count <= ADAPTER_LOG_STRING_LIMIT {
        return value.to_owned();
    }

    let preview = value
        .chars()
        .take(ADAPTER_LOG_STRING_LIMIT)
        .collect::<String>();
    format!(
        "{preview}<truncated {} chars>",
        char_count - ADAPTER_LOG_STRING_LIMIT
    )
}

fn redacted_marker(original_len: usize) -> String {
    format!("<redacted:{original_len}>")
}

fn serialized_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value)
        .map(|text| text.chars().count())
        .unwrap_or_default()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "authorization"
        || key == "proxy-authorization"
        || key == "x-api-key"
        || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
        || key.contains("cookie")
        || key.contains("credential")
        || key.contains("password")
        || key.contains("auth")
        || key.ends_with("key")
        || key.contains("api_key")
}

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

// ─── Stream aggregation ───────────────────────────────────────────────────────

#[derive(Default)]
struct AggregatedToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[allow(clippy::type_complexity)]
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

    let (pid, model, finish_reason, usage, provider_metadata) = completed.unwrap_or_else(|| {
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

pub fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    response_id.map(|id| serde_json::json!({ "response_id": id }))
}

pub fn provider_metadata_with_assistant_reasoning_field(
    provider_metadata: Option<serde_json::Value>,
    assistant_reasoning_field: Option<&str>,
) -> Option<serde_json::Value> {
    let assistant_reasoning_field = assistant_reasoning_field
        .map(str::trim)
        .filter(|value| matches!(*value, "reasoning_content" | "reasoning_details"));

    match (provider_metadata, assistant_reasoning_field) {
        (None, None) => None,
        (Some(metadata), None) => Some(metadata),
        (None, Some(field)) => Some(serde_json::json!({
            "assistant_reasoning_field": field
        })),
        (Some(serde_json::Value::Object(mut metadata)), Some(field)) => {
            metadata.insert(
                "assistant_reasoning_field".to_owned(),
                serde_json::Value::String(field.to_owned()),
            );
            Some(serde_json::Value::Object(metadata))
        }
        (Some(metadata), Some(field)) => Some(serde_json::json!({
            "provider_metadata": metadata,
            "assistant_reasoning_field": field,
        })),
    }
}

// ─── Text normalization ───────────────────────────────────────────────────────

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

// ─── OpenAI Responses API event helpers ──────────────────────────────────────

const RESPONSES_CALL_ID_MAX_CHARS: usize = 64;

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
    pub reasoning_content: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_details: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesToolEventKind {
    Added,
    Delta,
    Done,
}

#[derive(Debug, Clone)]
pub struct ResponsesToolEvent {
    pub kind: ResponsesToolEventKind,
    pub output_index: Option<usize>,
    pub item_id: Option<String>,
    pub call_id: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

pub fn responses_protocol_call_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| value.chars().count() <= RESPONSES_CALL_ID_MAX_CHARS)
        .map(ToOwned::to_owned)
}

impl ResponsesToolEvent {
    pub fn stream_key_candidates(&self, provider_id: &str) -> Result<Vec<String>, AppError> {
        let mut keys = Vec::new();
        if let Some(item_id) = self.item_id.as_ref() {
            keys.push(format!("item:{item_id}"));
        }
        if let Some(idx) = self.output_index {
            keys.push(format!("idx:{idx}"));
        }
        if let Some(call_id) = self.call_id.as_ref() {
            keys.push(format!("call:{call_id}"));
        }
        if keys.is_empty() {
            return Err(AppError::Provider(format!(
                "{provider_id} returned tool event without item_id/output_index/call_id"
            )));
        }
        Ok(keys)
    }
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
    if matches!(
        event_type,
        Some("response.output_text.delta" | "response.text.delta")
    ) {
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
        let item_id = normalize_optional_text(parsed.item_id);
        let call_id = normalize_optional_text(parsed.call_id);
        let id = responses_protocol_call_id(call_id.as_deref())
            .or_else(|| responses_protocol_call_id(item_id.as_deref()));
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Delta,
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            id,
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
        let item_id = normalize_optional_text(parsed.item_id);
        let call_id = normalize_optional_text(parsed.call_id);
        let id = responses_protocol_call_id(call_id.as_deref())
            .or_else(|| responses_protocol_call_id(item_id.as_deref()));
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Done,
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            id,
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
        let item_id = normalize_optional_text(parsed.item.id);
        let call_id = normalize_optional_text(parsed.item.call_id);
        let id = responses_protocol_call_id(call_id.as_deref())
            .or_else(|| responses_protocol_call_id(item_id.as_deref()));
        return Ok(Some(ResponsesToolEvent {
            kind: if event_type == "response.output_item.added" {
                ResponsesToolEventKind::Added
            } else {
                ResponsesToolEventKind::Done
            },
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            id,
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
            // Only treat `response.status` as a finish reason when it is
            // terminal — early events can carry `status: "in_progress"`
            // and would otherwise be latched as the final finish reason
            // even though the response continues.
            event
                .get("response")
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_str())
                .filter(|status| {
                    !matches!(
                        *status,
                        "in_progress" | "queued" | "pending" | "created" | "started"
                    )
                })
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
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| event.get("usage").filter(|v| !v.is_null()).cloned())
}

pub fn responses_response_id(event: &serde_json::Value) -> Option<String> {
    event
        .get("response")
        .and_then(|r| r.get("id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        })
}

pub fn responses_stream_error(
    provider_id: &str,
    event: &serde_json::Value,
) -> Result<Option<AppError>, AppError> {
    let payload = if responses_event_type(event) == Some("error") {
        event.clone()
    } else {
        let Some(payload) = event.get("error").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        payload.clone()
    };
    let parsed = parse_json_value::<ResponsesStreamErrorPayload>(
        provider_id,
        "responses stream error",
        payload,
    )?;
    if parsed.code.is_none() && parsed.message.is_none() {
        // Empty error envelopes appear on some `response.completed`
        // events that carry `error: {}`. Don't report a phantom error.
        return Ok(None);
    }
    Ok(Some(classify_stream_error(
        provider_id,
        parsed.code.as_deref(),
        parsed.message.as_deref().unwrap_or_default(),
    )))
}

fn responses_event_type(event: &serde_json::Value) -> Option<&str> {
    event.get("type").and_then(|v| v.as_str())
}

// ─── Error classification ─────────────────────────────────────────────────────

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
        .any(|p| normalized.contains(p))
        || ((normalized.starts_with("400") || normalized.starts_with("413"))
            && normalized.contains("(no body)"))
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
    if provider_id.trim().eq_ignore_ascii_case("openai") && status == reqwest::StatusCode::NOT_FOUND
    {
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

// ─── Wire deserialization helpers ─────────────────────────────────────────────

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

#[derive(Debug, Deserialize)]
struct ProviderErrorEnvelope {
    error: ProviderErrorBody,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorBody {
    message: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    code: Option<serde_json::Value>,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, ProviderErrorKind};

    #[test]
    fn responses_stream_error_accepts_top_level_error_event() {
        let event = serde_json::json!({
            "type": "error",
            "code": "invalid_prompt",
            "message": "Invalid value: input_text"
        });

        let err = responses_stream_error("openai", &event)
            .expect("parse stream error")
            .expect("top-level error");

        match err {
            AppError::ProviderClassified {
                provider,
                message,
                kind,
                retryable,
            } => {
                assert_eq!(provider, "openai");
                assert_eq!(message, "Invalid value: input_text");
                assert_eq!(kind, ProviderErrorKind::ApiError);
                assert!(!retryable);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn responses_stream_error_ignores_empty_completed_error_envelope() {
        let event = serde_json::json!({
            "type": "response.completed",
            "error": {}
        });

        assert!(
            responses_stream_error("openai", &event)
                .expect("parse stream event")
                .is_none()
        );
    }

    #[test]
    fn responses_tool_event_uses_item_index_and_call_key_candidates() {
        let event = serde_json::json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "lookup",
                "arguments": ""
            }
        });

        let tool_event = responses_tool_event("openai", &event)
            .expect("parse tool event")
            .expect("tool event");

        assert_eq!(tool_event.id.as_deref(), Some("call_1"));
        assert_eq!(
            tool_event
                .stream_key_candidates("openai")
                .expect("stream keys"),
            vec![
                "item:item_1".to_string(),
                "idx:0".to_string(),
                "call:call_1".to_string()
            ]
        );
    }

    #[test]
    fn responses_tool_event_accepts_delta_with_item_id_only() {
        let event = serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "item_1",
            "delta": "{\"query\""
        });

        let tool_event = responses_tool_event("openai", &event)
            .expect("parse tool event")
            .expect("tool event");

        assert_eq!(tool_event.id.as_deref(), Some("item_1"));
        assert_eq!(
            tool_event
                .stream_key_candidates("openai")
                .expect("stream keys"),
            vec!["item:item_1".to_string()]
        );
    }

    #[test]
    fn responses_tool_event_does_not_expose_oversized_item_id_as_call_id() {
        let oversized_id = "k".repeat(412);
        let event = serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "item_id": oversized_id,
            "delta": "{\"query\""
        });

        let tool_event = responses_tool_event("openai", &event)
            .expect("parse tool event")
            .expect("tool event");

        assert_eq!(tool_event.id, None);
        assert_eq!(tool_event.item_id.as_deref(), Some(oversized_id.as_str()));
        assert_eq!(
            tool_event
                .stream_key_candidates("openai")
                .expect("stream keys"),
            vec![format!("item:{oversized_id}")]
        );
    }
}
