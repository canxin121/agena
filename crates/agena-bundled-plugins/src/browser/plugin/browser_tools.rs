use super::sessions::{BrowserAccess, BrowserSessions, browser_output, browser_text_prefix};
use super::*;
use crate::browser::driver::{DomFormat, MAX_BODY_BYTES};
use agena_plugin_host::sdk::{PluginErrorKind, host_api};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Instant;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const CONTENT_TTL: Duration = Duration::from_secs(15 * 60);
const CONTENT_CACHE_BYTES: usize = 16 * MAX_BODY_BYTES;
const CONTENT_CACHE_ENTRIES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "frame_id"),
    non_empty("session_id", "expression"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserEvaluateInput {
    session_id: String,
    /// JavaScript to evaluate in the page, including an async expression returning a Promise.
    #[arg(max_chars = 32768)]
    expression: String,
    /// Frame from browser.frames. The default is the main document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    /// Execution deadline in milliseconds. A still-running page may be closed on timeout.
    #[serde(default = "default_script_timeout_ms")]
    #[schemars(range(min = 50, max = 30000))]
    timeout_ms: u64,
    /// Maximum JSON bytes to return inline.
    #[serde(default = "default_script_result_bytes")]
    #[schemars(range(min = 256, max = 64000))]
    max_result_bytes: usize,
}

const fn default_script_timeout_ms() -> u64 {
    5_000
}

const fn default_script_result_bytes() -> usize {
    24_000
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BrowserContentFormat {
    /// Visible text at capture time.
    #[default]
    Text,
    /// Serialized DOM at capture time, including dynamic changes.
    DomHtml,
    /// Captured HTTP response for the current main document navigation.
    ResponseHtml,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id", "content_id"),
    non_empty("session_id"),
    non_empty_if_present("selector", "frame_id", "content_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserContentInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<BrowserContentFormat>,
    /// CSS selector or snapshot @eN for a DOM fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Frame ID from browser.frames. Scoped DOM reads default to the main document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    /// Read the same capture again using its content_id, next_offset and the owning session_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 64)]
    content_id: Option<String>,
    #[serde(default)]
    #[schemars(range(max = 1048576))]
    offset: usize,
    /// Maximum Unicode characters to include in this slice.
    #[serde(default = "default_content_slice_chars")]
    #[schemars(range(min = 1, max = 24000))]
    max_chars: usize,
}

const fn default_content_slice_chars() -> usize {
    8_000
}

struct ContentCapture {
    id: String,
    page: String,
    url: String,
    format: BrowserContentFormat,
    frame_id: Option<String>,
    captured_at: Instant,
    text: String,
    truncated: bool,
}

#[derive(Default)]
pub(super) struct ContentCache {
    captures: VecDeque<Arc<ContentCapture>>,
}

impl ContentCache {
    fn get(&mut self, session_id: &str, content_id: &str) -> Option<Arc<ContentCapture>> {
        self.expire();
        self.captures
            .iter()
            .find(|capture| capture.page == session_id && capture.id == content_id)
            .cloned()
    }

    fn insert(&mut self, capture: Arc<ContentCapture>) {
        self.expire();
        let mut bytes = self
            .captures
            .iter()
            .map(|capture| capture.text.len())
            .sum::<usize>();
        while self.captures.len() >= CONTENT_CACHE_ENTRIES
            || bytes + capture.text.len() > CONTENT_CACHE_BYTES
        {
            let Some(removed) = self.captures.pop_front() else {
                break;
            };
            bytes -= removed.text.len();
        }
        self.captures.push_back(capture);
    }

    fn expire(&mut self) {
        self.captures
            .retain(|capture| capture.captured_at.elapsed() < CONTENT_TTL);
    }

    pub(super) fn remove_page(&mut self, session_id: &str) {
        self.captures.retain(|capture| capture.page != session_id);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id"),
    non_empty("session_id", "selector"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserInspectInput {
    session_id: String,
    /// CSS selector or @eN from the latest snapshot.
    #[arg(max_chars = 4096)]
    selector: String,
    /// Frame from browser.frames. Snapshot refs can identify their own frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    /// Attribute names to retrieve, for example href, aria-label or data-state.
    #[serde(default)]
    #[schemars(length(max = 12))]
    attributes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id"), non_empty("session_id"))]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserDiagnosticsInput {
    session_id: String,
    query: DiagnosticsQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DiagnosticsQuery {
    Network {
        /// Continue using next_seq from a previous network query.
        #[serde(default)]
        since_seq: u64,
        #[serde(default = "default_diagnostics_limit")]
        #[schemars(range(min = 1, max = 40))]
        limit: usize,
        /// Substring filter for request URLs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(length(min = 1, max = 2048))]
        url_contains: Option<String>,
    },
    Request {
        /// Request ID returned by a network query for this page.
        request_id: u64,
        /// Fetch the captured text response body for this request.
        #[serde(default)]
        include_body: bool,
        #[serde(default = "default_content_slice_chars")]
        #[schemars(range(min = 1, max = 24000))]
        max_chars: usize,
    },
    Logs {
        /// Continue using next_seq from a previous log query.
        #[serde(default)]
        since_seq: u64,
        #[serde(default = "default_diagnostics_limit")]
        #[schemars(range(min = 1, max = 40))]
        limit: usize,
    },
}

const fn default_diagnostics_limit() -> usize {
    20
}

impl BrowserPlugin {
    pub(super) async fn evaluate_in_browser(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserEvaluateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let host = self.state()?.host.clone();
        let state = self.browser_state.clone();
        let input = input.clone();
        let cancellation = context.cancellation.clone();
        let callback = host_api::current_host_callback_context().unwrap_or_default();
        // Keep the page gate until a cancelled or timed-out execution is retired.
        let (cancel, cancelled) = oneshot::channel();
        let worker = tokio::spawn(host_api::run_in_isolated_host_callback_context(
            callback,
            async move { evaluate(access, input, state, host, cancelled, cancellation).await },
        ));
        let result = worker
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        drop(cancel);
        result
    }

    pub(super) async fn read_browser_content(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserContentInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if input.content_id.is_some()
            && (input.format.is_some() || input.selector.is_some() || input.frame_id.is_some())
        {
            return Err(PluginError::invalid_params(
                "Use content_id, session_id, offset and max_chars to continue a capture",
            ));
        }
        let format = input.format.unwrap_or_default();
        if matches!(format, BrowserContentFormat::ResponseHtml)
            && (input.selector.is_some() || input.frame_id.is_some())
        {
            return Err(PluginError::invalid_params(
                "response_html reads the main document response; omit frame_id and selector",
            ));
        }
        let access = self.browser_access(context, &input.session_id).await?;
        let page = &access.page;
        page.verify_committed_url().await?;
        if let Some(id) = &input.content_id {
            let capture = self.browser_state.content.lock().await.get(&page.id, id)
                .ok_or_else(|| PluginError::invalid_params("Browser content is unavailable in this page. Take a fresh browser.content capture."))?;
            return content_slice(&capture, input.offset, input.max_chars);
        }
        let revision = page.driver.document_revision()?;
        let (url, body) = match format {
            BrowserContentFormat::ResponseHtml => match page.driver.document_response().await? {
                Ok(capture) => capture,
                Err(status) => {
                    return Ok(browser_output(
                        "Browser content",
                        format!("Main document response HTML is {status}."),
                        json!({"session_id": page.id, "format": format, "status": status}),
                    ));
                }
            },
            BrowserContentFormat::Text | BrowserContentFormat::DomHtml => {
                let dom_format = if matches!(format, BrowserContentFormat::Text) {
                    DomFormat::Text
                } else {
                    DomFormat::Html
                };
                page.driver
                    .capture_dom(
                        dom_format,
                        input.selector.as_deref(),
                        input.frame_id.as_deref(),
                        MAX_BODY_BYTES,
                        Duration::from_secs(10),
                    )
                    .await?
            }
        };
        page.verify_committed_url().await?;
        if page.driver.document_revision()? != revision {
            return Err(PluginError::invalid_params(
                "The page navigated while reading content. Take a fresh browser.content capture.",
            ));
        }
        let capture = Arc::new(ContentCapture {
            id: uuid::Uuid::new_v4().simple().to_string(),
            page: page.id.clone(),
            url,
            format,
            frame_id: input.frame_id.clone(),
            captured_at: Instant::now(),
            text: body.text,
            truncated: body.truncated,
        });
        let output = content_slice(&capture, input.offset, input.max_chars)?;
        self.browser_state.content.lock().await.insert(capture);
        Ok(output)
    }

    pub(super) async fn inspect_browser_element(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserInspectInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if input
            .attributes
            .iter()
            .any(|name| name.trim().is_empty() || name.len() > 128)
        {
            return Err(PluginError::invalid_params(
                "Attribute names must be nonempty and at most 128 bytes",
            ));
        }
        let access = self.browser_access(context, &input.session_id).await?;
        let page = &access.page;
        let data = page.driver.action(json!({
            "action": "inspect", "selector": input.selector, "attributes": input.attributes, "frameId": input.frame_id,
        }), Duration::from_secs(10)).await?;
        page.verify_committed_url().await?;
        let count = data["match_count"].as_u64().unwrap_or(0);
        Ok(data_output(
            "Inspect browser element",
            format!("{count} matches"),
            json!({
                "session_id": page.id, "selector": input.selector, "frame_id": input.frame_id,
                "match_count": count, "element": data["element"],
            }),
        ))
    }

    pub(super) async fn read_browser_diagnostics(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserDiagnosticsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let page = &access.page;
        page.verify_committed_url().await?;
        let (summary, mut data) = match &input.query {
            DiagnosticsQuery::Network {
                since_seq,
                limit,
                url_contains,
            } => (
                "Browser network requests",
                page.driver
                    .network_requests(*since_seq, *limit, url_contains.as_deref())?,
            ),
            DiagnosticsQuery::Request {
                request_id,
                include_body,
                max_chars,
            } => (
                "Browser request details",
                page.driver
                    .network_request(*request_id, *include_body, *max_chars)
                    .await?,
            ),
            DiagnosticsQuery::Logs { since_seq, limit } => (
                "Browser console and exceptions",
                page.driver.diagnostics_logs(*since_seq, *limit)?,
            ),
        };
        page.verify_preview_lease().await?;
        data["session_id"] = json!(page.id);
        Ok(data_output("Browser diagnostics", summary, data))
    }

    pub(super) async fn list_browser_frames(
        &self,
        context: &ToolInvokeContext<'_>,
        session_id: &str,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, session_id).await?;
        let page = &access.page;
        let mut data = page.driver.frames().await?;
        page.verify_committed_url().await?;
        data["session_id"] = json!(session_id);
        Ok(data_output(
            "Browser frames",
            "Frame IDs for the browser page",
            data,
        ))
    }
}

async fn evaluate(
    access: BrowserAccess,
    input: BrowserEvaluateInput,
    state: Arc<BrowserSessions>,
    host: Arc<dyn HostClient>,
    mut cancelled: oneshot::Receiver<()>,
    cancellation: CancellationToken,
) -> SdkResult<ToolInvokeOutput> {
    let page = &access.page;
    page.verify_committed_url().await?;
    let before = page.driver.context_id(input.frame_id.as_deref()).await?;
    let result = tokio::select! {
        biased;
        _ = &mut cancelled => None,
        _ = cancellation.cancelled() => None,
        result = page.driver.evaluate_script(
            &input.expression, input.frame_id.as_deref(), Duration::from_millis(input.timeout_ms)
        ) => Some(result),
    };
    let mut payload = json!({"session_id": page.id, "frame_id": input.frame_id});
    let result = match result {
        Some(Ok(result)) => result,
        Some(Err(error)) if error.kind == PluginErrorKind::Timeout => {
            let closed = close_evaluation_page(&access, &state, host.as_ref()).await;
            payload["status"] = json!("timeout");
            payload["page_closed"] = json!(closed);
            let summary = if closed {
                "Script exceeded its deadline and Chrome closed the page. Open a page to continue."
            } else {
                "Script exceeded its deadline. Check browser.list before attempting another action."
            };
            return Ok(browser_output(
                "Evaluate JavaScript",
                summary.into(),
                payload,
            ));
        }
        Some(Err(error))
            if error
                .diagnostic_message()
                .contains("Execution was terminated") =>
        {
            payload["status"] = json!("timeout");
            payload["page_closed"] = json!(false);
            return Ok(browser_output(
                "Evaluate JavaScript",
                "Chrome terminated JavaScript at its execution deadline. Inspect the page before retrying.".into(),
                payload,
            ));
        }
        Some(Err(error)) if context_lost(error.diagnostic_message()) => {
            return context_changed(&access, payload).await;
        }
        Some(Err(error)) => return Err(error),
        None => {
            close_evaluation_page(&access, &state, host.as_ref()).await;
            return Err(PluginError::from_kind(
                PluginErrorKind::Cancelled,
                "Browser evaluation cancelled",
            ));
        }
    };
    if before.is_some() && page.driver.context_id(input.frame_id.as_deref()).await? != before {
        return context_changed(&access, payload).await;
    }
    page.verify_committed_url().await?;
    let opened = page.driver.settle_popups().await;
    if !opened.is_empty() {
        payload["opened_sessions"] = json!(opened);
    }
    if let Some(exception) = result.get("exceptionDetails") {
        let message = exception
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| exception["text"].as_str())
            .unwrap_or("JavaScript execution failed");
        let timed_out = message.contains("Execution was terminated");
        let status = if timed_out { "timeout" } else { "exception" };
        payload["status"] = json!(status);
        payload["exception"] = json!({
            "message": browser_text_prefix(message, 2048),
            "line": exception["lineNumber"].as_u64().map(|line| line + 1),
            "column": exception["columnNumber"].as_u64().map(|column| column + 1),
        });
        return Ok(data_output(
            "Evaluate JavaScript",
            "JavaScript evaluation failed",
            payload,
        ));
    }
    let value = &result["result"];
    payload["status"] = json!("ok");
    payload["type"] = value["type"].clone();
    let special = value["unserializableValue"]
        .as_str()
        .or_else(|| (value["type"] == "undefined").then_some("undefined"));
    if let Some(special) = special {
        payload["unserializable"] = json!(browser_text_prefix(special, 512));
    } else if let Some(result) = value.get("value") {
        let bytes = serde_json::to_vec(result)
            .map_err(|error| PluginError::internal_error(&error))?
            .len();
        if bytes > input.max_result_bytes {
            payload["status"] = json!("result_too_large");
            payload["result_bytes"] = json!(bytes);
            return Ok(data_output(
                "Evaluate JavaScript",
                "Result exceeds the inline budget. Return a smaller value.",
                payload,
            ));
        }
        payload["value"] = result.clone();
    } else {
        payload["status"] = json!("unserializable_result");
        return Ok(data_output(
            "Evaluate JavaScript",
            "Return a JSON-serializable value from the page.",
            payload,
        ));
    }
    Ok(data_output(
        "Evaluate JavaScript",
        "JavaScript evaluation completed",
        payload,
    ))
}

fn context_lost(message: &str) -> bool {
    message.contains("Execution context was destroyed")
        || message.contains("Cannot find context")
        || message.contains("Cannot find context with specified id")
        || message.contains("Inspected target navigated")
}

async fn context_changed(access: &BrowserAccess, mut data: Value) -> SdkResult<ToolInvokeOutput> {
    access.page.verify_preview_lease().await?;
    data["status"] = json!("context_changed");
    if access.page.verify_committed_url().await.is_ok()
        && let Ok(url) = access.page.committed_url().await
    {
        data["url"] = json!(url);
    }
    Ok(data_output(
        "Evaluate JavaScript",
        "The execution context changed. Inspect the page before repeating the operation.",
        data,
    ))
}

async fn close_evaluation_page(
    access: &BrowserAccess,
    state: &BrowserSessions,
    host: &dyn HostClient,
) -> bool {
    access.cancel_requests();
    match state
        .close_locked(
            &access.page,
            BackgroundActivityStatus::Failed,
            "Browser script exceeded its execution lifetime.",
            host,
        )
        .await
    {
        Ok(()) => true,
        Err(error) => {
            log_secondary_plugin_failure("retire page after browser evaluation", &error);
            access.page.driver.disconnect();
            false
        }
    }
}

fn content_slice(
    capture: &ContentCapture,
    offset: usize,
    max_chars: usize,
) -> SdkResult<ToolInvokeOutput> {
    let slice = agena_web::page_slice(&capture.text, offset, max_chars)
        .map_err(PluginError::invalid_params)?;
    let summary = format!(
        "{} of {} captured characters",
        slice.returned_chars, slice.total_chars
    );
    let continuation = slice
        .next_offset
        .map(|offset| {
            format!(
                "\n\nContinue with content_id {} and offset {}.",
                capture.id, offset
            )
        })
        .unwrap_or_default();
    Ok(ToolInvokeOutput::from_parts(
        "Browser content",
        summary,
        format!("{}{}", slice.markdown, continuation),
        Some(json!({
            "session_id": capture.page, "content_id": capture.id, "format": capture.format,
            "frame_id": capture.frame_id, "url": capture.url, "status": "captured",
            "content": slice.markdown, "offset": offset, "next_offset": slice.next_offset,
            "total_chars": slice.total_chars, "returned_chars": slice.returned_chars,
            "truncated": capture.truncated,
        })),
        Default::default(),
        Vec::new(),
    ))
}

fn data_output(title: &str, summary: impl Into<String>, data: Value) -> ToolInvokeOutput {
    ToolInvokeOutput::from_parts(
        title,
        summary,
        data.to_string(),
        Some(data),
        Default::default(),
        Vec::new(),
    )
}
