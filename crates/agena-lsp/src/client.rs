//! Async LSP client over a transport. Mirrors the agena-mcp-client pattern:
//! a background reader_loop demultiplexes inbound frames, request/response
//! correlation lives in a DashMap keyed by request id.

mod document;
mod freshness;
mod server;
pub use document::valid_position;
use freshness::PublishedDiagnostics;
pub use freshness::{DiagnosticReport, DocumentReceipt};

use portable_atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use dashmap::DashMap;
use lsp_types::{
    Diagnostic, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, InitializeResult,
    Location, PartialResultParams, Position, PublishDiagnosticsParams, ReferenceContext,
    ReferenceParams, TextDocumentIdentifier, TextDocumentPositionParams, Uri,
    WorkDoneProgressParams,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::error::{LspError, LspResult};
use crate::protocol::{
    InboundMessage, JSONRPC_VERSION, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    RequestId,
};
use crate::transport::LspTransport;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
// LSP's `ContentModified` error code. A server may report this while it is
// catching up with a `didOpen`/`didChange` notification.
const CONTENT_MODIFIED_ERROR_CODE: i64 = -32_801;

/// Rust-analyzer (and other servers that build a project index lazily) can
/// accept an LSP request immediately after `didOpen` while its semantic model
/// is still empty. In that brief window it answers definition/reference/hover
/// with a valid-but-empty result. Retrying only after a document was opened or
/// changed keeps ordinary negative lookups fast while making the first
/// navigation request useful.
const POST_SYNC_NAVIGATION_RETRY_DELAYS: &[Duration] = &[
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_millis(1_600),
];

/// Notification surfaced by a server (e.g. `textDocument/publishDiagnostics`).
#[derive(Debug, Clone)]
pub struct ServerNotification {
    pub method: String,
    pub params: Value,
}

/// How a [`LspClient::sync_document_with_status`] call affected the server's
/// document state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSyncStatus {
    /// The document was not known to the server and was opened with `didOpen`.
    Opened,
    /// The document was already open and was refreshed with `didChange`.
    Changed,
    /// The server already has the same document contents.
    Unchanged,
}

impl DocumentSyncStatus {
    fn needs_navigation_retry(self) -> bool {
        matches!(self, Self::Opened | Self::Changed)
    }
}

/// Client for one LSP server.
pub struct LspClient {
    inner: Arc<Inner>,
}

struct Inner {
    transport: Arc<dyn LspTransport>,
    next_id: AtomicI64,
    pending: DashMap<i64, oneshot::Sender<JsonRpcResponse>>,
    notifications: Mutex<Option<mpsc::Sender<ServerNotification>>>,
    diagnostics: DashMap<Uri, PublishedDiagnostics>,
    diagnostics_changed: watch::Sender<u64>,
    document_sync: Mutex<()>,
    document_sequence: AtomicI64,
    initialized: arc_swap::ArcSwapOption<InitializeResult>,
    session: OnceLock<server::Session>,
    closed: watch::Sender<Option<Arc<str>>>,
    /// Track which URIs we've sent didOpen for (and their content hash +
    /// version), so we can decide between didOpen / didChange.
    open_docs: DashMap<Uri, document::OpenDocState>,
}

impl LspClient {
    pub fn new(transport: Arc<dyn LspTransport>) -> Arc<Self> {
        let (closed, closed_rx) = watch::channel(None);
        let inner = Arc::new(Inner {
            transport: Arc::clone(&transport),
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            notifications: Mutex::new(None),
            diagnostics: DashMap::new(),
            diagnostics_changed: watch::channel(0).0,
            document_sync: Mutex::new(()),
            document_sequence: AtomicI64::new(1),
            initialized: arc_swap::ArcSwapOption::from(None),
            session: OnceLock::new(),
            closed,
            open_docs: DashMap::new(),
        });
        let weak_inner = Arc::downgrade(&inner);
        tokio::spawn(async move { reader_loop(weak_inner, transport, closed_rx).await });
        Arc::new(Self { inner })
    }

    /// Subscribe to every notification frame the server sends. Replaces
    /// any previous subscription. Diagnostics still land in the typed
    /// per-uri cache regardless.
    pub async fn subscribe_notifications(&self) -> mpsc::Receiver<ServerNotification> {
        let (tx, rx) = mpsc::channel(256);
        let mut guard = self.inner.notifications.lock().await;
        *guard = Some(tx);
        rx
    }

    /// Latest diagnostics published for `uri` (drained from the per-uri
    /// cache, returning a clone). Empty vec for unknown URIs.
    pub fn diagnostics_for(&self, uri: &Uri) -> Vec<Diagnostic> {
        let version = self.inner.open_docs.get(uri).map(|doc| doc.version);
        version.map_or_else(Vec::new, |version| {
            self.diagnostic_report(uri, version).diagnostics
        })
    }

    /// Snapshot every cached `(uri, diagnostics)` pair as plain strings. Used
    /// by read-only observability surfaces such as the `agena.lsp` plugin.
    pub fn diagnostics_snapshot(&self) -> Vec<(String, Vec<Diagnostic>)> {
        let documents: Vec<_> = self
            .inner
            .open_docs
            .iter()
            .map(|entry| (entry.key().clone(), entry.version))
            .collect();
        documents
            .into_iter()
            .filter_map(|(uri, version)| {
                let report = self.diagnostic_report(&uri, version);
                (!report.diagnostics.is_empty()).then(|| (uri.to_string(), report.diagnostics))
            })
            .collect()
    }

    pub fn server_info(&self) -> Option<Arc<InitializeResult>> {
        self.inner.initialized.load_full()
    }

    /// A client cannot be reused after either direction of its transport fails.
    pub fn is_alive(&self) -> bool {
        self.inner.closed.borrow().is_none() && self.inner.transport.is_alive()
    }

    fn ensure_alive(&self) -> LspResult<()> {
        if let Some(reason) = self.inner.closed.borrow().as_ref() {
            return Err(LspError::TransportClosed(reason.to_string()));
        }
        if !self.inner.transport.is_alive() {
            return Err(LspError::TransportClosed(
                "LSP transport has stopped".into(),
            ));
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> LspResult<()> {
        if !self.is_alive() {
            return self.close_transport().await;
        }
        let handshake = tokio::time::timeout(
            SHUTDOWN_REQUEST_TIMEOUT,
            self.request_opt::<Value, Value>("shutdown", Value::Null),
        )
        .await;
        let exit_result = if matches!(handshake, Ok(Ok(_))) {
            self.notify("exit", Value::Null).await
        } else {
            Ok(())
        };
        let close_result = self.close_transport().await;
        match handshake {
            Ok(Ok(_)) => match (exit_result, close_result) {
                (Ok(()), result) | (result, Ok(())) => result,
                (Err(exit_error), Err(close_error)) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&close_error),
                        "LSP exit notification and transport close both failed; returning the exit failure"
                    );
                    Err(exit_error)
                }
            },
            Ok(Err(error)) => {
                if let Err(close_error) = close_result {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "LSP transport close also failed after the shutdown request failed",
                            &close_error,
                        ),
                        "returning the primary LSP shutdown request failure"
                    );
                }
                Err(error)
            }
            Err(source) => {
                if let Err(close_error) = close_result {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "LSP transport close also failed after the shutdown handshake timed out",
                            &close_error,
                        ),
                        "returning the primary LSP shutdown timeout"
                    );
                }
                Err(LspError::Timeout {
                    operation: "LSP shutdown handshake".to_owned(),
                    timeout_ms: SHUTDOWN_REQUEST_TIMEOUT.as_millis() as u64,
                    source,
                })
            }
        }
    }

    /// Close the transport without waiting for the LSP shutdown handshake.
    /// Used when initialization itself failed: no initialized peer exists to
    /// answer `shutdown`, but the child process must not be leaked.
    pub(crate) async fn close_transport(&self) -> LspResult<()> {
        self.inner.disconnect("LSP client closed");
        self.inner.transport.close().await
    }

    pub async fn definition(
        &self,
        uri: Uri,
        position: Position,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        self.request_feature("textDocument/definition", params)
            .await
    }

    /// Resolve a definition after a document synchronization. A freshly
    /// opened or changed document can briefly produce a valid empty response
    /// while a language server performs background analysis; retry that
    /// narrow case rather than exposing a misleading negative result.
    pub async fn definition_after_sync(
        &self,
        uri: Uri,
        position: Position,
        sync_status: DocumentSyncStatus,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        self.retry_navigation_after_sync(sync_status, || self.definition(uri.clone(), position))
            .await
    }

    /// Find implementations of the symbol at a synchronized position.
    pub async fn implementation_after_sync(
        &self,
        uri: Uri,
        position: Position,
        status: DocumentSyncStatus,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        self.retry_navigation_after_sync(status, || {
            self.request_feature(
                "textDocument/implementation",
                GotoDefinitionParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri.clone() },
                        position,
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                },
            )
        })
        .await
    }

    pub async fn document_symbols(
        &self,
        uri: Uri,
    ) -> LspResult<Option<lsp_types::DocumentSymbolResponse>> {
        self.request_feature(
            "textDocument/documentSymbol",
            lsp_types::DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .await
    }

    pub async fn workspace_symbols(
        &self,
        query: String,
        status: DocumentSyncStatus,
    ) -> LspResult<Option<lsp_types::WorkspaceSymbolResponse>> {
        self.retry_navigation_after_sync(status, || {
            self.request_feature(
                "workspace/symbol",
                lsp_types::WorkspaceSymbolParams {
                    query: query.clone(),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                },
            )
        })
        .await
    }

    pub async fn references(
        &self,
        uri: Uri,
        position: Position,
        include_declaration: bool,
    ) -> LspResult<Option<Vec<Location>>> {
        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: ReferenceContext {
                include_declaration,
            },
        };
        self.request_feature("textDocument/references", params)
            .await
    }

    /// Find references after synchronizing the document. See
    /// [`Self::definition_after_sync`] for why freshly synced documents retry
    /// transient empty semantic results.
    pub async fn references_after_sync(
        &self,
        uri: Uri,
        position: Position,
        include_declaration: bool,
        sync_status: DocumentSyncStatus,
    ) -> LspResult<Option<Vec<Location>>> {
        self.retry_navigation_after_sync(sync_status, || {
            self.references(uri.clone(), position, include_declaration)
        })
        .await
    }

    pub async fn hover(&self, uri: Uri, position: Position) -> LspResult<Option<Hover>> {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        self.request_feature("textDocument/hover", params).await
    }

    /// Fetch hover information after synchronizing the document. See
    /// [`Self::definition_after_sync`] for retry semantics.
    pub async fn hover_after_sync(
        &self,
        uri: Uri,
        position: Position,
        sync_status: DocumentSyncStatus,
    ) -> LspResult<Option<Hover>> {
        self.retry_navigation_after_sync(sync_status, || self.hover(uri.clone(), position))
            .await
    }

    async fn retry_navigation_after_sync<T, F, Fut>(
        &self,
        sync_status: DocumentSyncStatus,
        mut request: F,
    ) -> LspResult<Option<T>>
    where
        T: NavigationResult,
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = LspResult<Option<T>>>,
    {
        let mut result = request().await;
        if !sync_status.needs_navigation_retry() || !should_retry_navigation(&result) {
            return result;
        }

        for delay in POST_SYNC_NAVIGATION_RETRY_DELAYS {
            tokio::time::sleep(*delay).await;
            result = request().await;
            if !should_retry_navigation(&result) {
                break;
            }
        }
        result
    }

    async fn request<P, R>(&self, method: &str, params: P) -> LspResult<R>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        let value = self
            .request_value(method, serde_json::to_value(params)?)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    async fn request_opt<P, R>(&self, method: &str, params: P) -> LspResult<Option<R>>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        let value = self
            .request_value(method, serde_json::to_value(params)?)
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(value)?))
    }

    async fn request_value(&self, method: &str, params: Value) -> LspResult<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::Number(id),
            method: method.to_string(),
            params: Some(params),
        };
        self.ensure_alive()?;
        let payload = serde_json::to_value(&req)?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(id, tx);
        let mut pending = PendingRequest {
            inner: &self.inner,
            id,
            cancel_on_drop: !matches!(method, "initialize" | "shutdown"),
        };
        let response = async {
            self.send(payload).await?;
            self.ensure_alive()?;
            rx.await.map_err(|error| {
                self.ensure_alive().err().unwrap_or_else(|| {
                    LspError::transport_closed(
                        format!("response channel closed for LSP request `{method}` (id {id})"),
                        &error,
                    )
                })
            })
        };
        let resp = match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, response).await {
            Ok(response) => {
                pending.cancel_on_drop = false;
                response?
            }
            Err(source) => {
                return Err(LspError::Timeout {
                    operation: format!("LSP request `{method}` (id {id})"),
                    timeout_ms: DEFAULT_REQUEST_TIMEOUT.as_millis() as u64,
                    source,
                });
            }
        };
        if let Some(err) = resp.error {
            return Err(LspError::Server {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub async fn notify<P>(&self, method: &str, params: P) -> LspResult<()>
    where
        P: serde::Serialize,
    {
        let n = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.to_string(),
            params: Some(serde_json::to_value(params)?),
        };
        self.send(serde_json::to_value(&n)?).await
    }

    async fn send(&self, payload: Value) -> LspResult<()> {
        self.ensure_alive()?;
        let result = self.inner.transport.send(payload).await;
        if let Err(
            error @ (LspError::Io(_) | LspError::Transport(_) | LspError::TransportClosed(_)),
        ) = &result
        {
            self.inner.disconnect(&error.to_string());
        }
        result
    }
}

trait NavigationResult {
    fn is_empty_navigation_result(&self) -> bool;
}

impl NavigationResult for GotoDefinitionResponse {
    fn is_empty_navigation_result(&self) -> bool {
        match self {
            Self::Scalar(_) => false,
            Self::Array(locations) => locations.is_empty(),
            Self::Link(links) => links.is_empty(),
        }
    }
}

impl NavigationResult for Vec<Location> {
    fn is_empty_navigation_result(&self) -> bool {
        self.is_empty()
    }
}

impl NavigationResult for Hover {
    fn is_empty_navigation_result(&self) -> bool {
        false
    }
}

impl NavigationResult for lsp_types::WorkspaceSymbolResponse {
    fn is_empty_navigation_result(&self) -> bool {
        match self {
            Self::Flat(symbols) => symbols.is_empty(),
            Self::Nested(symbols) => symbols.is_empty(),
        }
    }
}

fn should_retry_navigation<T: NavigationResult>(result: &LspResult<Option<T>>) -> bool {
    match result {
        Ok(None) => true,
        Ok(Some(value)) => value.is_empty_navigation_result(),
        Err(LspError::Server { code, .. }) => *code == CONTENT_MODIFIED_ERROR_CODE,
        Err(..) => false,
    }
}

async fn reader_loop(
    weak_inner: Weak<Inner>,
    transport: Arc<dyn LspTransport>,
    mut closed: watch::Receiver<Option<Arc<str>>>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            _ = closed.changed() => return,
            frame = transport.recv() => frame,
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                tracing::debug!(target: "agena_lsp::reader", %error, "LSP transport ended");
                if let Some(inner) = weak_inner.upgrade() {
                    inner.disconnect(&error.to_string());
                }
                return;
            }
        };
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };
        match frame {
            InboundMessage::Response(resp) => {
                let RequestId::Number(id) = resp.id else {
                    continue;
                };
                if let Some((_, tx)) = inner.pending.remove(&id) {
                    if tx.send(resp).is_err() {
                        tracing::debug!(
                            target: "agena_lsp::reader",
                            request_id = id,
                            "LSP response receiver was dropped before delivery"
                        );
                    }
                } else {
                    tracing::warn!(
                        target: "agena_lsp::reader",
                        "response with unknown id {id}"
                    );
                }
            }
            InboundMessage::Notification(n) => {
                handle_notification(&inner, n).await;
            }
            InboundMessage::Request(req) => {
                let reply = server::answer_request(inner.session.get(), req);
                drop(inner);
                let sending = async { transport.send(serde_json::to_value(reply)?).await };
                let result = tokio::select! {
                    biased;
                    _ = closed.changed() => return,
                    result = tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, sending) => result,
                };
                let reason = match result {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "timed out replying to the language server".into(),
                };
                if let Some(inner) = weak_inner.upgrade() {
                    inner.disconnect(&reason);
                }
                return;
            }
        }
    }
}

impl Inner {
    fn disconnect(&self, reason: &str) {
        self.closed.send_if_modified(|value| {
            if value.is_some() {
                return false;
            }
            *value = Some(Arc::from(reason));
            true
        });
        self.pending.clear();
        self.diagnostics_changed
            .send_modify(|value| *value = value.wrapping_add(1));
    }
}

struct PendingRequest<'a> {
    inner: &'a Inner,
    id: i64,
    cancel_on_drop: bool,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        let removed = self.inner.pending.remove(&self.id).is_some();
        if removed
            && self.cancel_on_drop
            && self.inner.closed.borrow().is_none()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let transport = Arc::clone(&self.inner.transport);
            let id = self.id;
            runtime.spawn(async move {
                let message = serde_json::json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "method": "$/cancelRequest",
                    "params": { "id": id },
                });
                if let Ok(Err(error)) =
                    tokio::time::timeout(Duration::from_millis(500), transport.send(message)).await
                {
                    tracing::debug!(%error, request_id = id, "could not cancel LSP request");
                }
            });
        }
    }
}

async fn handle_notification(inner: &Inner, n: JsonRpcNotification) {
    let params = n.params.unwrap_or(Value::Null);
    if n.method == "textDocument/publishDiagnostics"
        && let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
        && let Some(current_version) = inner.open_docs.get(&p.uri).map(|doc| doc.version)
        && p.version.is_none_or(|version| version <= current_version)
    {
        inner.diagnostics_changed.send_modify(|generation| {
            let existing = inner.diagnostics.get(&p.uri).and_then(|item| item.version);
            // An unversioned publication cannot displace matching versioned evidence.
            let older = match (existing, p.version) {
                (Some(old), Some(new)) => old > new,
                (Some(old), None) => old == current_version,
                _ => false,
            };
            if !older {
                *generation = generation.wrapping_add(1);
                inner.diagnostics.insert(
                    p.uri,
                    PublishedDiagnostics::new(p.version, *generation, p.diagnostics),
                );
            }
        });
    }
    let subscriber = inner.notifications.lock().await.clone();
    if let Some(tx) = subscriber {
        // Diagnostics have their own latest-value cache above. The generic
        // notification feed is observational, so a slow subscriber must not
        // create an unbounded queue or stop the LSP response reader.
        let method = n.method;
        if let Err(error) = tx.try_send(ServerNotification {
            method: method.clone(),
            params,
        }) {
            tracing::warn!(
                notification_method = %method,
                diagnostic = %error,
                "LSP notification was dropped because the observational queue was unavailable"
            );
        }
    }
}

// `Inner` is private; consumers should call `LspClient::*` methods.

fn hash_str(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests;
