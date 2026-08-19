//! Async LSP client over a transport. Mirrors the agena-mcp-client pattern:
//! a background reader_loop demultiplexes inbound frames, request/response
//! correlation lives in a DashMap keyed by request id.

use portable_atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use dashmap::DashMap;
use lsp_types::{
    ClientCapabilities, Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams,
    InitializeParams, InitializeResult, Location, PartialResultParams, Position,
    PublishDiagnosticsParams, ReferenceContext, ReferenceParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Uri,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams,
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
    Duration::from_millis(1_000),
    Duration::from_millis(1_500),
    Duration::from_millis(2_000),
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
    diagnostics: DashMap<Uri, Vec<Diagnostic>>,
    initialized: arc_swap::ArcSwapOption<InitializeResult>,
    shutdown: watch::Sender<bool>,
    /// Track which URIs we've sent didOpen for (and their content hash +
    /// version), so we can decide between didOpen / didChange.
    open_docs: DashMap<Uri, OpenDocState>,
}

#[derive(Debug, Clone)]
struct OpenDocState {
    version: i32,
    content_hash: u64,
    language_id: String,
}

impl LspClient {
    pub fn new(transport: Arc<dyn LspTransport>) -> Arc<Self> {
        let (shutdown, shutdown_rx) = watch::channel(false);
        let inner = Arc::new(Inner {
            transport: Arc::clone(&transport),
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            notifications: Mutex::new(None),
            diagnostics: DashMap::new(),
            initialized: arc_swap::ArcSwapOption::from(None),
            shutdown,
            open_docs: DashMap::new(),
        });
        let weak_inner = Arc::downgrade(&inner);
        tokio::spawn(async move { reader_loop(weak_inner, transport, shutdown_rx).await });
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
        self.inner
            .diagnostics
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// Snapshot every cached `(uri, diagnostics)` pair as plain strings. Used
    /// by read-only observability surfaces such as the `agena.lsp` plugin.
    pub fn diagnostics_snapshot(&self) -> Vec<(String, Vec<Diagnostic>)> {
        self.inner
            .diagnostics
            .iter()
            .map(|entry| (entry.key().as_str().to_string(), entry.value().clone()))
            .collect()
    }

    pub fn server_info(&self) -> Option<Arc<InitializeResult>> {
        self.inner.initialized.load_full()
    }

    pub async fn initialize(
        &self,
        root_uri: Option<Uri>,
        client_name: &str,
        client_version: &str,
        initialization_options: Option<Value>,
    ) -> LspResult<InitializeResult> {
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_path: None,
            root_uri,
            initialization_options,
            capabilities: ClientCapabilities::default(),
            trace: None,
            workspace_folders: None,
            client_info: Some(lsp_types::ClientInfo {
                name: client_name.to_string(),
                version: Some(client_version.to_string()),
            }),
            locale: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        let result: InitializeResult = self.request("initialize", params).await?;
        self.inner.initialized.store(Some(Arc::new(result.clone())));
        // The spec requires the initialized notification before any other
        // request — in practice servers tolerate either order, but we send
        // it eagerly for correctness.
        let _ = self.notify("initialized", serde_json::json!({})).await;
        Ok(result)
    }

    pub async fn shutdown(&self) -> LspResult<()> {
        let handshake = tokio::time::timeout(
            SHUTDOWN_REQUEST_TIMEOUT,
            self.request_opt::<Value, Value>("shutdown", Value::Null),
        )
        .await;
        if matches!(handshake, Ok(Ok(_))) {
            let _ = self.notify("exit", Value::Null).await;
        }
        let close_result = self.inner.transport.close().await;
        let _ = self.inner.shutdown.send(true);
        match handshake {
            Ok(Ok(_)) => close_result,
            Ok(Err(error)) => Err(error),
            Err(_) => Err(LspError::Timeout(
                SHUTDOWN_REQUEST_TIMEOUT.as_millis() as u64
            )),
        }
    }

    /// Close the transport without waiting for the LSP shutdown handshake.
    /// Used when initialization itself failed: no initialized peer exists to
    /// answer `shutdown`, but the child process must not be leaked.
    pub(crate) async fn close_transport(&self) -> LspResult<()> {
        self.inner.transport.close().await
    }

    /// Make sure the server has the current contents of `uri`. Sends
    /// `didOpen` on first sight, `didChange` if we've already opened it
    /// and the content hash differs, and is a no-op otherwise. Safe to
    /// call before every LSP request.
    pub async fn sync_document(&self, uri: Uri, text: String, language_id: &str) -> LspResult<()> {
        self.sync_document_with_status(uri, text, language_id)
            .await
            .map(|_| ())
    }

    /// Synchronize a document and report whether the server needs to process
    /// a new version. Callers that issue semantic navigation immediately can
    /// pass the returned status to the `*_after_sync` methods below, which
    /// retry transient empty responses while the server builds its index.
    pub async fn sync_document_with_status(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
    ) -> LspResult<DocumentSyncStatus> {
        let hash = hash_str(&text);
        let entry = self.inner.open_docs.get(&uri).map(|r| r.value().clone());
        match entry {
            None => {
                let state = OpenDocState {
                    version: 1,
                    content_hash: hash,
                    language_id: language_id.to_string(),
                };
                self.inner.open_docs.insert(uri.clone(), state);
                let params = DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri,
                        language_id: language_id.to_string(),
                        version: 1,
                        text,
                    },
                };
                self.notify("textDocument/didOpen", params)
                    .await
                    .map(|_| DocumentSyncStatus::Opened)
            }
            Some(prev) if prev.content_hash == hash => Ok(DocumentSyncStatus::Unchanged),
            Some(prev) => {
                let new_version = prev.version.saturating_add(1);
                self.inner.open_docs.insert(
                    uri.clone(),
                    OpenDocState {
                        version: new_version,
                        content_hash: hash,
                        language_id: prev.language_id.clone(),
                    },
                );
                let params = DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri,
                        version: new_version,
                    },
                    content_changes: vec![TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text,
                    }],
                };
                self.notify("textDocument/didChange", params)
                    .await
                    .map(|_| DocumentSyncStatus::Changed)
            }
        }
    }

    /// Tell the server to drop a document we previously opened.
    pub async fn close_document(&self, uri: Uri) -> LspResult<()> {
        if self.inner.open_docs.remove(&uri).is_some() {
            let params = DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
            };
            self.notify("textDocument/didClose", params).await?;
        }
        Ok(())
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
        self.request_opt("textDocument/definition", params).await
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
        self.request_opt("textDocument/references", params).await
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
        self.request_opt("textDocument/hover", params).await
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
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(id, tx);
        let payload = serde_json::to_value(&req)?;
        if let Err(err) = self.inner.transport.send(payload).await {
            self.inner.pending.remove(&id);
            return Err(err);
        }
        let resp = match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, rx).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_)) => return Err(LspError::TransportClosed),
            Err(_) => {
                self.inner.pending.remove(&id);
                return Err(LspError::Timeout(DEFAULT_REQUEST_TIMEOUT.as_millis() as u64));
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
        self.inner.transport.send(serde_json::to_value(&n)?).await
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

fn should_retry_navigation<T: NavigationResult>(result: &LspResult<Option<T>>) -> bool {
    match result {
        Ok(None) => true,
        Ok(Some(value)) => value.is_empty_navigation_result(),
        Err(LspError::Server { code, .. }) => *code == CONTENT_MODIFIED_ERROR_CODE,
        Err(_) => false,
    }
}

async fn reader_loop(
    weak_inner: Weak<Inner>,
    transport: Arc<dyn LspTransport>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            frame = transport.recv() => frame,
        };
        let frame = match frame {
            Ok(f) => f,
            Err(err) => {
                tracing::debug!(target: "agena_lsp::reader", "transport ended: {err}");
                if let Some(inner) = weak_inner.upgrade() {
                    inner.pending.clear();
                }
                return;
            }
        };
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };
        match frame {
            InboundMessage::Response(resp) => {
                let RequestId::Number(id) = resp.id;
                if let Some((_, tx)) = inner.pending.remove(&id) {
                    let _ = tx.send(resp);
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
                // Reply with method-not-found so the server unblocks. We
                // do not implement server→client requests beyond logging.
                tracing::debug!(
                    target: "agena_lsp::reader",
                    method = %req.method,
                    "ignoring server→client request"
                );
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(crate::protocol::JsonRpcError {
                        code: -32601,
                        message: format!("method '{}' not handled by client", req.method),
                        data: None,
                    }),
                };
                drop(inner);
                let _ = transport
                    .send(serde_json::to_value(&resp).unwrap_or(Value::Null))
                    .await;
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

async fn handle_notification(inner: &Inner, n: JsonRpcNotification) {
    let params = n.params.unwrap_or(Value::Null);
    if n.method == "textDocument/publishDiagnostics"
        && let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
    {
        inner.diagnostics.insert(p.uri.clone(), p.diagnostics);
    }
    let subscriber = inner.notifications.lock().await.clone();
    if let Some(tx) = subscriber {
        // Diagnostics have their own latest-value cache above. The generic
        // notification feed is observational, so a slow subscriber must not
        // create an unbounded queue or stop the LSP response reader.
        let _ = tx.try_send(ServerNotification {
            method: n.method,
            params,
        });
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
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use tokio::time::timeout;

    use super::{DocumentSyncStatus, GotoDefinitionResponse, LspClient, Position, Uri};
    use crate::{
        protocol::{InboundMessage, JSONRPC_VERSION, JsonRpcResponse, RequestId},
        transport::InMemoryTransport,
    };

    fn response(id: i64, result: serde_json::Value) -> InboundMessage {
        InboundMessage::Response(JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::Number(id),
            result: Some(result),
            error: None,
        })
    }

    fn request_id(payload: &serde_json::Value) -> i64 {
        payload
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .expect("client should send a JSON-RPC request id")
    }

    #[tokio::test]
    async fn definition_after_open_retries_an_empty_initial_response() {
        let (transport, mut outbound, inbound) = InMemoryTransport::pair();
        let client = LspClient::new(transport);
        let uri: Uri = "file:///workspace/src/lib.rs".parse().unwrap();
        let position = Position::new(2, 28);
        let status = client
            .sync_document_with_status(uri.clone(), "pub fn probe() {}\n".to_string(), "rust")
            .await
            .unwrap();
        assert_eq!(status, DocumentSyncStatus::Opened);
        let did_open = outbound.recv().await.expect("didOpen notification");
        assert_eq!(
            did_open.get("method").and_then(serde_json::Value::as_str),
            Some("textDocument/didOpen")
        );

        let task = tokio::spawn({
            let client = client.clone();
            let uri = uri.clone();
            async move {
                client
                    .definition_after_sync(uri, position, status)
                    .await
                    .unwrap()
            }
        });

        let first = outbound.recv().await.expect("first definition request");
        assert_eq!(
            first.get("method").and_then(serde_json::Value::as_str),
            Some("textDocument/definition")
        );
        inbound
            .send(response(request_id(&first), json!([])))
            .await
            .unwrap();

        let retry = timeout(Duration::from_secs(1), outbound.recv())
            .await
            .expect("retry should be scheduled after an empty response")
            .expect("retry definition request");
        assert_eq!(
            retry.get("method").and_then(serde_json::Value::as_str),
            Some("textDocument/definition")
        );
        inbound
            .send(response(
                request_id(&retry),
                json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": 0, "character": 7 },
                        "end": { "line": 0, "character": 12 }
                    }
                }),
            ))
            .await
            .unwrap();

        let response = task.await.unwrap().expect("definition response");
        assert!(matches!(response, GotoDefinitionResponse::Scalar(_)));
    }

    #[tokio::test]
    async fn dropping_client_releases_reader_transport_without_peer_eof() {
        let (transport, _outbound, _inbound) = InMemoryTransport::pair();
        let client = LspClient::new(transport.clone());
        assert!(Arc::strong_count(&transport) > 1);

        drop(client);
        timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&transport) > 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reader task releases the transport when its owner is dropped");
    }
}
