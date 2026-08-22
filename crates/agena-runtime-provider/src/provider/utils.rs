use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, RwLock};

use futures_core::Stream;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::ProviderError;
use crate::provider::{CompletionResponse, ManagedCredential, should_retry_credential};
use agena_domain::{ModelId, ProviderId};
use agena_provider::CompletionStreamEvent;
use agena_provider::ProviderErrorKind;
pub use agena_provider::{ChatStreamChunk, ResponsesToolEvent, ResponsesToolEventKind};
use agena_provider::{CompletionFinishReason, CompletionToolCall, CompletionUsage};
pub use agena_provider::{
    auth_header_value, ensure_header_case_insensitive, insert_header_case_insensitive,
    merge_json_object_patch_map, merge_provider_metadata, merged_request_headers,
    normalize_base_url, normalize_optional_text, optional_non_empty, prompt_cache_header_entries,
    provider_metadata_value_is_meaningful, request_shape_fingerprint,
};

pub const ADAPTER_LOG_TARGET: &str = "agena::adapter";
const ADAPTER_LOG_STRING_LIMIT: usize = 2_048;
const MAX_PROVIDER_JSON_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_PROVIDER_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;

fn pretty_adapter_log_json(value: &serde_json::Value, context: &str) -> String {
    match serde_json::to_string_pretty(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                target: ADAPTER_LOG_TARGET,
                operation = context,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "sanitized adapter diagnostic JSON could not be serialized"
            );
            "<sanitized JSON serialization failed>".to_owned()
        }
    }
}

tokio::task_local! {
    static REQUEST_CANCELLATION: Option<tokio_util::sync::CancellationToken>;
}

/// Composition-owned hook for provider request-header enrichment.
///
/// Provider adapters must not reach back into the runtime plugin singleton.
/// The composition layer installs this port once during bootstrap; provider
/// code only knows the neutral header/cancellation boundary.
pub trait ProviderRequestHeaderHook: Send + Sync {
    fn resolve(
        &self,
        provider_id: &str,
        headers: BTreeMap<String, String>,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> BTreeMap<String, String>;
}

static REQUEST_HEADER_HOOK: OnceLock<RwLock<Option<Arc<dyn ProviderRequestHeaderHook>>>> =
    OnceLock::new();

pub fn install_request_header_hook(hook: Option<Arc<dyn ProviderRequestHeaderHook>>) {
    let slot = REQUEST_HEADER_HOOK.get_or_init(|| RwLock::new(None));
    let mut installed = match slot.write() {
        Ok(installed) => installed,
        Err(error) => {
            tracing::error!(
                operation = "install provider request-header hook",
                error = %error,
                "recovering poisoned provider request-header hook lock"
            );
            error.into_inner()
        }
    };
    *installed = hook;
}

/// Make the execution cancellation token visible to synchronous provider
/// request builders. Those builders may run a blocking `chat.headers` plugin
/// hook before their first network await; without this scope, cancelling the
/// outer provider future cannot be observed until the hook timeout expires.
pub async fn with_request_cancellation<F>(
    cancellation: Option<tokio_util::sync::CancellationToken>,
    future: F,
) -> F::Output
where
    F: std::future::Future,
{
    REQUEST_CANCELLATION.scope(cancellation, future).await
}

fn current_request_cancellation() -> Option<tokio_util::sync::CancellationToken> {
    REQUEST_CANCELLATION
        .try_with(|cancellation| cancellation.clone())
        .ok()
        .flatten()
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

    let hook = REQUEST_HEADER_HOOK
        .get()
        .and_then(|slot| match slot.read() {
            Ok(guard) => guard.clone(),
            Err(error) => {
                tracing::error!(
                    operation = "resolve provider request headers",
                    error = %error,
                    "recovering poisoned provider request-header hook lock"
                );
                error.into_inner().clone()
            }
        });
    if let Some(hook) = hook {
        combined = hook.resolve(provider_id, combined, current_request_cancellation());
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

pub fn serialize_request_body_with_patch(
    body: &impl serde::Serialize,
    patch: &BTreeMap<String, serde_json::Value>,
) -> Result<serde_json::Value, ProviderError> {
    let mut value = serde_json::to_value(body).map_err(ProviderError::from)?;
    if patch.is_empty() {
        return Ok(value);
    }

    let serde_json::Value::Object(target) = &mut value else {
        return Err(ProviderError::Config(
            "request body patch can only be applied to JSON object bodies".to_owned(),
        ));
    };

    merge_json_object_patch_map(target, patch);
    Ok(value)
}

// ─── HTTP response helpers ────────────────────────────────────────────────────

pub(crate) async fn response_text_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
    context: &str,
) -> Result<String, ProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ProviderError::Provider(format!(
            "{context} exceeds the {max_bytes}-byte response limit"
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ProviderError::Provider(format!(
                "{context} exceeds the {max_bytes}-byte response limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body)
        .map_err(|_| ProviderError::Provider(format!("{context} is not UTF-8 text")))
}

pub async fn parse_json_response_logged<T: DeserializeOwned>(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    response: reqwest::Response,
) -> Result<T, ProviderError> {
    if response.status().is_success() {
        ensure_response_content_type(provider_id, &response, "application/json")?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response_text_bounded(
            response,
            MAX_PROVIDER_JSON_RESPONSE_BYTES,
            "provider JSON response",
        )
        .await?;
        adapter_log_http_response_text(
            provider_id,
            adapter_kind,
            operation,
            status,
            &headers,
            body.as_str(),
        );
        return serde_json::from_str::<T>(body.as_str()).map_err(ProviderError::from);
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
) -> Result<(), ProviderError> {
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

    Err(ProviderError::Provider(format!(
        "{provider_id} returned unexpected content-type `{actual}` (expected {expected_prefix})"
    )))
}

pub fn parse_json_value<T: DeserializeOwned>(
    provider_id: &str,
    context: &str,
    value: serde_json::Value,
) -> Result<T, ProviderError> {
    serde_json::from_value(value).map_err(|err| {
        ProviderError::Provider(format!(
            "{provider_id} returned invalid {context} payload: {err}"
        ))
    })
}

/// Classify a provider JSON stream error (SSE / JSON-lines) into a
/// [`ProviderError`] carrying the real provider id. Transport/body errors map
/// to `ProviderError::Http` (retryable for timeout/connect/body/decode); a
/// malformed payload is treated as transient and retryable
/// (`MalformedResponse`) so the registry's backoff loop resamples the request
/// instead of failing the run immediately.
pub fn json_stream_error(
    provider_id: &str,
    error: crate::ProviderJsonStreamError,
) -> ProviderError {
    match error {
        crate::ProviderJsonStreamError::Http(error) => ProviderError::Http(error),
        crate::ProviderJsonStreamError::InvalidJson { format, source } => {
            ProviderError::ProviderClassified {
                provider: provider_id.to_owned(),
                message: format!("invalid {format} payload: {source}"),
                kind: ProviderErrorKind::MalformedResponse,
                retryable: true,
            }
        }
    }
}

pub async fn http_status_error_from_response_logged(
    provider_id: &str,
    adapter_kind: &str,
    operation: &str,
    response: reqwest::Response,
) -> ProviderError {
    let status = response.status();
    let headers = response.headers().clone();
    let raw_body = response_text_bounded(
        response,
        MAX_PROVIDER_ERROR_RESPONSE_BYTES,
        "provider error response",
    )
    .await
    .unwrap_or_else(|error| format!("<unavailable: {error}>"));

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
            if let Some(param) = parsed.error.param {
                message.push_str(&format!(" (param={param})"));
            }
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
    ProviderError::HttpStatus {
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
        let headers_json =
            pretty_adapter_log_json(&sanitized_headers, "serialize adapter request headers");
        let body_json = body.map(sanitize_json_value);
        let body_text = body_json
            .as_ref()
            .map(|value| pretty_adapter_log_json(value, "serialize adapter request body"))
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
        let headers_json =
            pretty_adapter_log_json(&sanitized_headers, "serialize adapter response headers");
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
    let event_text = pretty_adapter_log_json(&sanitized, "serialize adapter stream event");
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
        let headers_json =
            pretty_adapter_log_json(&sanitized_headers, "serialize adapter response headers");
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
    match serde_json::to_string(value) {
        Ok(text) => text.chars().count(),
        Err(error) => {
            tracing::error!(
                target: ADAPTER_LOG_TARGET,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "measure sanitized adapter JSON diagnostic",
                    &error,
                ),
                "adapter JSON diagnostic length is using a debug fallback"
            );
            format!("{value:?}").chars().count()
        }
    }
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
) -> Result<reqwest::Response, ProviderError>
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

#[derive(Default)]
struct AggregatedToolCalls {
    calls: BTreeMap<String, AggregatedToolCallState>,
    aliases: BTreeMap<String, String>,
}

impl AggregatedToolCalls {
    fn state_for_event(
        &mut self,
        stream_key: String,
        provider_call_id: Option<&str>,
    ) -> &mut AggregatedToolCallState {
        let provider_call_id = provider_call_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let resolved_key = self.resolve_stream_key(stream_key, provider_call_id);
        let state = self.calls.entry(resolved_key).or_default();
        if let Some(provider_call_id) = provider_call_id {
            state.id = Some(provider_call_id.to_owned());
        }
        state
    }

    fn resolve_stream_key(&mut self, stream_key: String, provider_call_id: Option<&str>) -> String {
        let Some(provider_call_id) = provider_call_id else {
            if self.calls.contains_key(stream_key.as_str()) {
                return stream_key;
            }
            if let Some(existing_key) = self
                .aliases
                .get(stream_key.as_str())
                .filter(|existing_key| self.calls.contains_key(existing_key.as_str()))
            {
                return existing_key.clone();
            }
            return stream_key;
        };

        if let Some(existing_key) = self.calls.iter().find_map(|(key, state)| {
            (state.id.as_deref().map(str::trim) == Some(provider_call_id)).then(|| key.clone())
        }) {
            self.aliases.insert(stream_key, existing_key.clone());
            return existing_key;
        }

        let canonical_key = format!("id:{provider_call_id}");
        if self.calls.contains_key(canonical_key.as_str()) {
            self.aliases.insert(stream_key, canonical_key.clone());
            return canonical_key;
        }

        let existing_stream_key = if self.calls.contains_key(stream_key.as_str()) {
            Some(stream_key.clone())
        } else {
            self.aliases
                .get(stream_key.as_str())
                .filter(|existing_key| self.calls.contains_key(existing_key.as_str()))
                .cloned()
        };
        let can_rekey_existing_stream =
            existing_stream_key.as_deref().is_some_and(|existing_key| {
                self.calls.get(existing_key).is_some_and(|state| {
                    state.id.as_deref().is_none()
                        || state.id.as_deref().map(str::trim) == Some(provider_call_id)
                })
            });
        if can_rekey_existing_stream
            && let Some(existing_stream_key) = existing_stream_key
            && existing_stream_key != canonical_key
        {
            let state = self
                .calls
                .remove(existing_stream_key.as_str())
                .expect("checked aggregated tool stream exists");
            self.calls.insert(canonical_key.clone(), state);
            for alias_target in self.aliases.values_mut() {
                if alias_target == &existing_stream_key {
                    *alias_target = canonical_key.clone();
                }
            }
        }

        self.aliases.insert(stream_key, canonical_key.clone());
        canonical_key
    }
}

#[allow(clippy::type_complexity)]
pub async fn aggregate_stream<S>(
    provider_id: &str,
    fallback_model: ModelId,
    stream: S,
) -> Result<CompletionResponse, ProviderError>
where
    S: Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Unpin,
{
    let mut stream = stream;
    let mut text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls = AggregatedToolCalls::default();
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
                let id = normalize_optional_text(id);
                let state = tool_calls.state_for_event(stream_key, id.as_deref());
                if let Some(name) = normalize_optional_text(name) {
                    state.name = Some(name);
                }
                state.arguments.push_str(arguments_delta.as_str());
            }
            CompletionStreamEvent::ToolCallSnapshot {
                stream_key,
                id,
                name,
                arguments_json,
                ..
            } => {
                let id = normalize_optional_text(id);
                let state = tool_calls.state_for_event(stream_key, id.as_deref());
                if let Some(name) = normalize_optional_text(name) {
                    state.name = Some(name);
                }
                let snapshot = arguments_json.trim();
                let snapshot_is_degenerate = snapshot.is_empty() || snapshot == "{}";
                if !snapshot_is_degenerate || state.arguments.trim().is_empty() {
                    state.arguments = arguments_json;
                }
            }
            CompletionStreamEvent::ProviderNativeToolCallStarted { .. }
            | CompletionStreamEvent::ProviderNativeToolCallCompleted { .. }
            | CompletionStreamEvent::ProviderRetry { .. } => {}
            CompletionStreamEvent::Completed {
                provider_id: pid,
                model,
                finish_reason,
                usage,
                provider_metadata,
                end_turn: _,
            } => match completed.as_mut() {
                Some((
                    completed_pid,
                    completed_model,
                    completed_finish_reason,
                    completed_usage,
                    completed_provider_metadata,
                )) => {
                    if !pid.as_ref().trim().is_empty() {
                        *completed_pid = pid;
                    }
                    if !model.as_ref().trim().is_empty() {
                        *completed_model = model;
                    }
                    if finish_reason.is_some() {
                        *completed_finish_reason = finish_reason;
                    }
                    if usage
                        .as_ref()
                        .is_some_and(|usage| usage != &CompletionUsage::default())
                    {
                        *completed_usage = usage;
                    }
                    *completed_provider_metadata = merge_provider_metadata(
                        completed_provider_metadata.take(),
                        provider_metadata,
                    );
                }
                None => {
                    let usage = usage.filter(|usage| usage != &CompletionUsage::default());
                    completed = Some((pid, model, finish_reason, usage, provider_metadata));
                }
            },
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
        .calls
        .into_iter()
        .map(|(stream_key, state)| {
            let id = normalize_optional_text(state.id).unwrap_or(stream_key);
            let name = optional_non_empty(state.name).ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} stream ended with tool call without name"
                ))
            })?;
            Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: state.arguments,
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;

    if text.is_empty() && calls.is_empty() && finish_reason.is_none() {
        return Err(ProviderError::Provider(format!(
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
    normalize_optional_text(response_id).map(|id| serde_json::json!({ "response_id": id }))
}

pub fn provider_metadata_with_chat_reasoning_state(
    provider_metadata: Option<serde_json::Value>,
    assistant_reasoning_field: Option<&str>,
    reasoning_details: Option<serde_json::Value>,
    copilot_reasoning_opaque: Option<String>,
) -> Option<serde_json::Value> {
    let assistant_reasoning_field = assistant_reasoning_field
        .map(str::trim)
        .filter(|value| matches!(*value, "reasoning_content" | "reasoning_details"));
    let mut metadata = match provider_metadata {
        Some(serde_json::Value::Object(metadata)) => metadata,
        Some(metadata) => serde_json::Map::from_iter([("provider_metadata".to_owned(), metadata)]),
        None => serde_json::Map::new(),
    };
    if let Some(field) = assistant_reasoning_field {
        metadata.insert(
            "assistant_reasoning_field".to_owned(),
            serde_json::Value::String(field.to_owned()),
        );
    }
    if let Some(details) = reasoning_details.filter(provider_metadata_value_is_meaningful) {
        metadata.insert("openai_chat_reasoning_details".to_owned(), details);
    }
    if let Some(opaque) = copilot_reasoning_opaque.filter(|value| !value.trim().is_empty()) {
        metadata.insert(
            "copilot_reasoning_opaque".to_owned(),
            serde_json::Value::String(opaque),
        );
    }
    (!metadata.is_empty()).then_some(serde_json::Value::Object(metadata))
}

pub fn require_terminal_stream_event(
    provider_id: &str,
    protocol: &str,
    terminal_seen: bool,
) -> Result<(), ProviderError> {
    if terminal_seen {
        Ok(())
    } else {
        Err(ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: format!("{protocol} stream closed before a terminal response event"),
            kind: ProviderErrorKind::MalformedResponse,
            // A truncated transport is safe to retry before output. After
            // output, the registry only retries providers that explicitly
            // opt into replay-safe prefix verification.
            retryable: true,
        })
    }
}

// ─── OpenAI Responses API event helpers ──────────────────────────────────────

pub fn chat_stream_error(provider_id: &str, event: &serde_json::Value) -> Option<ProviderError> {
    stream_error_from_event(provider_id, event)
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
) -> Result<Option<ResponsesToolEvent>, ProviderError> {
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
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Delta,
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            namespace: optional_non_empty(parsed.namespace),
            name: optional_non_empty(parsed.name),
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
        return Ok(Some(ResponsesToolEvent {
            kind: ResponsesToolEventKind::Done,
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            namespace: optional_non_empty(parsed.namespace),
            name: optional_non_empty(parsed.name),
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
        return Ok(Some(ResponsesToolEvent {
            kind: if event_type == "response.output_item.added" {
                ResponsesToolEventKind::Added
            } else {
                ResponsesToolEventKind::Done
            },
            output_index: parsed.output_index,
            item_id: item_id.clone(),
            call_id: call_id.clone(),
            namespace: optional_non_empty(parsed.item.namespace),
            name: optional_non_empty(parsed.item.name),
            arguments: optional_non_empty(parsed.item.arguments),
        }));
    }

    Ok(None)
}

/// OpenAI Responses `response.completed` may carry an optional `end_turn`
/// boolean on the response envelope. When the model explicitly signals the
/// turn is not complete (`end_turn=false`), the session layer can continue
/// driving the model without waiting for a tool call. A missing field, a
/// non-boolean value, or a gateway that omits the field all yield `None`.
pub fn responses_end_turn(event: &serde_json::Value) -> Option<bool> {
    event
        .get("response")
        .and_then(|r| r.get("end_turn"))
        .and_then(|v| v.as_bool())
}

pub fn responses_finish_reason(event: &serde_json::Value) -> Option<String> {
    normalize_optional_text(
        event
            .get("response")
            .and_then(|r| r.get("stop_reason"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
    )
    .or_else(|| {
        normalize_optional_text(
            event
                .get("response")
                .and_then(|r| r.get("incomplete_details"))
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
        )
    })
    .or_else(|| {
        normalize_optional_text(
            event
                .get("response")
                .and_then(|r| r.get("status_details"))
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
        )
    })
    .or_else(|| {
        // Only treat `response.status` as a finish reason when it is
        // terminal — early events can carry `status: "in_progress"`
        // and would otherwise be latched as the final finish reason
        // even though the response continues.
        normalize_optional_text(
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
                .map(ToOwned::to_owned),
        )
    })
    .or_else(|| {
        normalize_optional_text(
            event
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
        )
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
    normalize_optional_text(
        event
            .get("response")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
    )
    .or_else(|| {
        normalize_optional_text(
            event
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
        )
    })
}

pub fn responses_stream_error(
    provider_id: &str,
    event: &serde_json::Value,
) -> Result<Option<ProviderError>, ProviderError> {
    Ok(stream_error_from_event(provider_id, event))
}

fn stream_error_from_event(provider_id: &str, event: &serde_json::Value) -> Option<ProviderError> {
    let event_type = responses_event_type(event);
    let hard_error_event = matches!(event_type, Some("error" | "response.failed"));
    let nested = event
        .get("response")
        .and_then(|response| response.get("error"))
        .filter(|value| !value.is_null())
        .or_else(|| event.get("error").filter(|value| !value.is_null()));

    if !hard_error_event && nested.is_none() {
        return None;
    }

    let code = nested
        .and_then(|payload| error_payload_field(payload, "code"))
        .or_else(|| error_payload_field(event, "code"));
    let message = nested
        .and_then(error_payload_message)
        .or_else(|| error_payload_message(event));

    // Some successful terminal events include `error: {}`. Only explicit
    // error/failed event types are errors when the envelope has no details.
    if code.is_none() && message.is_none() && !hard_error_event {
        return None;
    }

    let fallback = match event_type {
        Some("response.failed") => "provider response failed",
        _ => "provider stream error",
    };
    let message = message
        .as_deref()
        .unwrap_or_else(|| if code.is_none() { fallback } else { "" });
    Some(classify_stream_error(provider_id, code.as_deref(), message))
}

fn error_payload_field(payload: &serde_json::Value, field: &str) -> Option<String> {
    let value = payload.get(field)?.clone();
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => (!value.trim().is_empty()).then_some(value),
        value => Some(value.to_string()),
    }
}

fn error_payload_message(payload: &serde_json::Value) -> Option<String> {
    if let Some(message) = payload.as_str().filter(|value| !value.trim().is_empty()) {
        return Some(message.to_owned());
    }
    error_payload_field(payload, "message")
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
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::Authentication,
            retryable: false,
        };
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        };
    }
    if status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::GATEWAY_TIMEOUT
    {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        };
    }
    if provider_id.trim().eq_ignore_ascii_case("openai") && status == reqwest::StatusCode::NOT_FOUND
    {
        return ProviderErrorClassification {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        };
    }
    let retryable = status == reqwest::StatusCode::CONFLICT || status.is_server_error();
    ProviderErrorClassification {
        kind: if status.is_server_error() {
            ProviderErrorKind::Unavailable
        } else {
            ProviderErrorKind::InvalidRequest
        },
        retryable,
    }
}

fn classify_stream_error(provider_id: &str, code: Option<&str>, message: &str) -> ProviderError {
    let normalized_code = code.unwrap_or_default().trim().to_ascii_lowercase();

    if normalized_code == "context_length_exceeded" {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: "Input exceeds context window. Try shortening your prompt.".to_owned(),
            kind: ProviderErrorKind::ContextOverflow,
            retryable: false,
        };
    }
    if matches!(
        normalized_code.as_str(),
        "rate_limit_exceeded" | "rate_limit_error" | "too_many_requests"
    ) {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: if message.trim().is_empty() {
                normalized_code.clone()
            } else {
                format!("{normalized_code}: {}", message.trim())
            },
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        };
    }
    if matches!(
        normalized_code.as_str(),
        "invalid_api_key" | "authentication_error" | "unauthorized"
    ) {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: if message.trim().is_empty() {
                normalized_code.clone()
            } else {
                format!("{normalized_code}: {}", message.trim())
            },
            kind: ProviderErrorKind::Authentication,
            retryable: false,
        };
    }
    if normalized_code == "insufficient_quota" {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: "Quota exceeded. Please check your plan and billing details.".to_owned(),
            kind: ProviderErrorKind::QuotaExceeded,
            retryable: false,
        };
    }
    if normalized_code == "usage_not_included" {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message:
                "To use Codex models and OpenAI reasoning summaries, upgrade to Plus plan first."
                    .to_owned(),
            kind: ProviderErrorKind::QuotaExceeded,
            retryable: false,
        };
    }
    if normalized_code == "invalid_prompt" {
        return ProviderError::ProviderClassified {
            provider: provider_id.to_owned(),
            message: if message.trim().is_empty() {
                "Invalid prompt.".to_owned()
            } else {
                message.to_owned()
            },
            kind: ProviderErrorKind::InvalidRequest,
            retryable: false,
        };
    }

    let kind = if is_context_overflow_message(message) {
        ProviderErrorKind::ContextOverflow
    } else {
        ProviderErrorKind::Unavailable
    };
    let retryable = kind != ProviderErrorKind::ContextOverflow
        && is_retryable_stream_error(normalized_code.as_str(), message);
    let message = match (normalized_code.as_str(), message.trim()) {
        ("", "") => "provider stream error".to_owned(),
        (code, "") => code.to_owned(),
        ("", message) => message.to_owned(),
        (code, message) if message.to_ascii_lowercase().contains(code) => message.to_owned(),
        (code, message) => format!("{code}: {message}"),
    };
    ProviderError::ProviderClassified {
        provider: provider_id.to_owned(),
        message,
        retryable,
        kind,
    }
}

fn is_retryable_stream_error(code: &str, message: &str) -> bool {
    if matches!(
        code,
        "rate_limit_exceeded"
            | "server_error"
            | "internal_error"
            | "overloaded"
            | "overloaded_error"
            | "server_is_overloaded"
            | "slow_down"
            | "request_timeout"
            | "timeout"
            | "temporarily_unavailable"
            | "service_unavailable"
            | "connection_error"
    ) {
        return true;
    }

    let message = message.to_ascii_lowercase();
    [
        "rate limit",
        "too many requests",
        "temporarily unavailable",
        "service unavailable",
        "server overloaded",
        "server is busy",
        "internal server error",
        "request timeout",
        "timed out",
        "connection reset",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
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
    namespace: Option<String>,
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
    namespace: Option<String>,
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
    namespace: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorEnvelope {
    error: ProviderErrorBody,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorBody {
    message: String,
    #[serde(default)]
    param: Option<serde_json::Value>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    code: Option<serde_json::Value>,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn classified_message(error: ProviderError) -> String {
        match error {
            ProviderError::ProviderClassified { message, .. } => message,
            other => panic!("expected classified provider error, got {other:?}"),
        }
    }

    #[test]
    fn provider_failures_have_stable_semantic_kinds() {
        assert_eq!(
            classify_http_error("example", reqwest::StatusCode::UNAUTHORIZED, "token=secret").kind,
            ProviderErrorKind::Authentication
        );
        assert_eq!(
            classify_http_error(
                "example",
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                "raw upstream body"
            )
            .kind,
            ProviderErrorKind::RateLimited
        );
        assert_eq!(
            classify_http_error(
                "example",
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                "raw upstream body"
            )
            .kind,
            ProviderErrorKind::Unavailable
        );
        assert_eq!(
            classify_stream_error("example", Some("insufficient_quota"), "billing raw")
                .provider_error_kind(),
            Some(ProviderErrorKind::QuotaExceeded)
        );
    }

    #[test]
    fn responses_failed_reads_nested_response_error() {
        let event = json!({
            "type": "response.failed",
            "response": {
                "status": "failed",
                "error": {
                    "code": "rate_limit_exceeded",
                    "message": "slow down"
                }
            }
        });

        let error = responses_stream_error("openai", &event)
            .expect("error payload should decode")
            .expect("response.failed should be an error");
        assert!(error.retryable());
        assert_eq!(classified_message(error), "rate_limit_exceeded: slow down");
    }

    #[test]
    fn responses_failed_without_payload_is_still_an_error() {
        let event = json!({
            "type": "response.failed",
            "response": { "status": "failed" }
        });

        let error = responses_stream_error("openai", &event)
            .expect("error payload should decode")
            .expect("response.failed should be an error");
        assert_eq!(classified_message(error), "provider response failed");
    }

    #[test]
    fn stream_error_falls_back_to_code_when_message_is_missing() {
        let event = json!({
            "type": "error",
            "code": "internal_error"
        });

        let error = responses_stream_error("openai", &event)
            .expect("error payload should decode")
            .expect("error event should be surfaced");
        assert!(error.retryable());
        assert_eq!(classified_message(error), "internal_error");
    }

    #[test]
    fn codex_overload_codes_are_retryable_without_a_message() {
        for code in ["server_is_overloaded", "slow_down"] {
            let event = json!({ "type": "error", "code": code });
            let error = responses_stream_error("openai", &event)
                .expect("error payload should decode")
                .expect("error event should be surfaced");
            assert!(error.retryable(), "{code} should be retryable");
            assert_eq!(classified_message(error), code);
        }
    }

    #[test]
    fn realtime_error_reads_nested_error_envelope() {
        let event = json!({
            "type": "error",
            "error": {
                "code": "invalid_request_error",
                "message": "bad request"
            }
        });

        let error = responses_stream_error("openai", &event)
            .expect("error payload should decode")
            .expect("realtime error should be surfaced");
        assert!(!error.retryable());
        assert_eq!(
            classified_message(error),
            "invalid_request_error: bad request"
        );
    }

    #[test]
    fn chat_error_envelope_is_surfaced() {
        let event = json!({
            "error": {
                "code": "context_length_exceeded",
                "message": "maximum context length exceeded"
            }
        });

        let error = chat_stream_error("compatible", &event).expect("chat error should surface");
        assert!(!error.retryable());
        assert_eq!(
            error.provider_error_kind(),
            Some(ProviderErrorKind::ContextOverflow)
        );
    }

    #[test]
    fn empty_error_on_completed_event_is_ignored() {
        let event = json!({
            "type": "response.completed",
            "error": {}
        });
        assert!(
            responses_stream_error("openai", &event)
                .expect("error payload should decode")
                .is_none()
        );
    }

    #[test]
    fn partial_stream_without_terminal_event_is_rejected() {
        let error = require_terminal_stream_event("compatible", "chat completions", false)
            .expect_err("truncated stream must fail");
        assert!(
            error
                .to_string()
                .contains("closed before a terminal response event")
        );
        assert!(error.retryable());
        require_terminal_stream_event("compatible", "chat completions", true)
            .expect("terminal stream should pass");
    }

    #[test]
    fn json_stream_error_carries_provider_id_and_retryable_malformed_kind() {
        let error = json_stream_error(
            "my-provider",
            crate::ProviderJsonStreamError::InvalidJson {
                format: "SSE",
                source: serde_json::from_str::<serde_json::Value>("{broken").unwrap_err(),
            },
        );
        match error {
            ProviderError::ProviderClassified {
                provider,
                kind,
                retryable,
                ..
            } => {
                assert_eq!(provider, "my-provider");
                assert_eq!(kind, ProviderErrorKind::MalformedResponse);
                assert!(retryable);
            }
            other => panic!("expected ProviderClassified, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn aggregate_stream_keeps_valid_tool_and_terminal_state_from_empty_updates() {
        let provider_id = ProviderId::new("fixture");
        let model = ModelId::new("fixture-model");
        let usage = CompletionUsage {
            input_tokens: 17,
            ..CompletionUsage::default()
        };
        let events = vec![
            Ok(CompletionStreamEvent::ToolCallDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: "idx:0".to_owned(),
                id: Some("call_valid".to_owned()),
                name: Some("fs.read".to_owned()),
                arguments_delta: r#"{"file_path":"README.md"}"#.to_owned(),
            }),
            Ok(CompletionStreamEvent::ToolCallSnapshot {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: "idx:7".to_owned(),
                id: Some("call_valid".to_owned()),
                name: Some(String::new()),
                arguments_json: "{}".to_owned(),
            }),
            Ok(CompletionStreamEvent::ToolCallDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: "idx:0".to_owned(),
                id: Some("   ".to_owned()),
                name: Some("   ".to_owned()),
                arguments_delta: String::new(),
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: provider_id.clone(),
                model: model.clone(),
                finish_reason: Some(CompletionFinishReason::ToolCalls),
                usage: Some(usage.clone()),
                provider_metadata: Some(serde_json::json!({
                    "response_id": "resp_valid",
                    "nested": { "first": true }
                })),
                end_turn: None,
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id,
                model,
                finish_reason: None,
                usage: Some(CompletionUsage::default()),
                provider_metadata: Some(serde_json::json!({
                    "nested": { "second": true }
                })),
                end_turn: None,
            }),
        ];

        let response = aggregate_stream(
            "fixture",
            ModelId::new("fallback-model"),
            futures_util::stream::iter(events),
        )
        .await
        .expect("stream aggregates");
        assert_eq!(
            response.finish_reason,
            Some(CompletionFinishReason::ToolCalls)
        );
        assert_eq!(response.usage, Some(usage));
        assert_eq!(
            response.provider_metadata.as_ref().unwrap()["response_id"],
            "resp_valid"
        );
        assert_eq!(
            response.provider_metadata.as_ref().unwrap()["nested"],
            serde_json::json!({"first": true, "second": true})
        );
        assert_eq!(
            response.tool_calls,
            vec![CompletionToolCall::Function {
                id: "call_valid".to_owned(),
                name: "fs.read".to_owned(),
                arguments_json: r#"{"file_path":"README.md"}"#.to_owned(),
            }]
        );
    }

    #[test]
    fn responses_ids_ignore_blank_nested_updates_and_normalize_metadata() {
        let event = serde_json::json!({
            "id": "  resp_fallback  ",
            "response": { "id": "   " }
        });
        assert_eq!(
            responses_response_id(&event).as_deref(),
            Some("resp_fallback")
        );
        assert_eq!(
            response_id_metadata(Some("  resp_1  ".to_owned())),
            Some(serde_json::json!({"response_id": "resp_1"}))
        );
        assert_eq!(response_id_metadata(Some("   ".to_owned())), None);
        let finish = serde_json::json!({
            "stop_reason": "  stop  ",
            "response": { "stop_reason": "   " }
        });
        assert_eq!(responses_finish_reason(&finish).as_deref(), Some("stop"));
    }
}
