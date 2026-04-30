//! Async LSP client over a transport. Mirrors the agena-mcp-client pattern:
//! a background reader_loop demultiplexes inbound frames, request/response
//! correlation lives in a DashMap keyed by request id.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
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
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::error::{LspError, LspResult};
use crate::protocol::{
    InboundMessage, JSONRPC_VERSION, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    RequestId,
};
use crate::transport::LspTransport;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Notification surfaced by a server (e.g. `textDocument/publishDiagnostics`).
#[derive(Debug, Clone)]
pub struct ServerNotification {
    pub method: String,
    pub params: Value,
}

pub struct LspClient {
    inner: Arc<Inner>,
}

struct Inner {
    transport: Arc<dyn LspTransport>,
    next_id: AtomicI64,
    pending: DashMap<i64, oneshot::Sender<JsonRpcResponse>>,
    notifications: Mutex<Option<mpsc::UnboundedSender<ServerNotification>>>,
    diagnostics: DashMap<Uri, Vec<Diagnostic>>,
    initialized: arc_swap::ArcSwapOption<InitializeResult>,
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
        let inner = Arc::new(Inner {
            transport,
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            notifications: Mutex::new(None),
            diagnostics: DashMap::new(),
            initialized: arc_swap::ArcSwapOption::from(None),
            open_docs: DashMap::new(),
        });
        let inner_for_reader = inner.clone();
        tokio::spawn(async move { reader_loop(inner_for_reader).await });
        Arc::new(Self { inner })
    }

    /// Subscribe to every notification frame the server sends. Replaces
    /// any previous subscription. Diagnostics still land in the typed
    /// per-uri cache regardless.
    pub async fn subscribe_notifications(&self) -> mpsc::UnboundedReceiver<ServerNotification> {
        let (tx, rx) = mpsc::unbounded_channel();
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
        let _: Option<Value> = self.request_opt("shutdown", Value::Null).await?;
        let _ = self.notify("exit", Value::Null).await;
        let _ = self.inner.transport.close().await;
        Ok(())
    }

    /// Make sure the server has the current contents of `uri`. Sends
    /// `didOpen` on first sight, `didChange` if we've already opened it
    /// and the content hash differs, and is a no-op otherwise. Safe to
    /// call before every LSP request.
    pub async fn sync_document(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
    ) -> LspResult<()> {
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
                self.notify("textDocument/didOpen", params).await
            }
            Some(prev) if prev.content_hash == hash => Ok(()),
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
                self.notify("textDocument/didChange", params).await
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

    async fn request<P, R>(&self, method: &str, params: P) -> LspResult<R>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        let value = self.request_value(method, serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(value)?)
    }

    async fn request_opt<P, R>(&self, method: &str, params: P) -> LspResult<Option<R>>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        let value = self.request_value(method, serde_json::to_value(params)?).await?;
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
                return Err(LspError::Timeout(
                    DEFAULT_REQUEST_TIMEOUT.as_millis() as u64,
                ));
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

async fn reader_loop(inner: Arc<Inner>) {
    loop {
        let frame = match inner.transport.recv().await {
            Ok(f) => f,
            Err(err) => {
                tracing::debug!(target: "agena_lsp::reader", "transport ended: {err}");
                let mut keys: Vec<i64> = inner.pending.iter().map(|kv| *kv.key()).collect();
                for k in keys.drain(..) {
                    inner.pending.remove(&k);
                }
                return;
            }
        };
        match frame {
            InboundMessage::Response(resp) => {
                let id = match resp.id {
                    RequestId::Number(n) => n,
                };
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
                let _ = inner
                    .transport
                    .send(serde_json::to_value(&resp).unwrap_or(Value::Null))
                    .await;
            }
        }
    }
}

async fn handle_notification(inner: &Inner, n: JsonRpcNotification) {
    let params = n.params.unwrap_or(Value::Null);
    if n.method == "textDocument/publishDiagnostics" {
        if let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone()) {
            inner.diagnostics.insert(p.uri.clone(), p.diagnostics);
        }
    }
    let guard = inner.notifications.lock().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(ServerNotification {
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
