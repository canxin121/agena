//! High-level MCP client built on top of an [`McpTransport`].
//!
//! Owns the JSON-RPC request/response bookkeeping (next id, pending map),
//! a background reader task that demultiplexes inbound frames, and a
//! pluggable handler for server→client requests (sampling/createMessage,
//! roots/list, etc.).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::error::{McpError, McpResult};
use crate::protocol::{
    CallToolParams, CallToolResult, CreateMessageParams, CreateMessageResult, GetPromptParams,
    GetPromptResult, Implementation, InboundMessage, InitializeParams, InitializeResult,
    JSONRPC_VERSION, JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PROTOCOL_VERSION, ReadResourceParams,
    ReadResourceResult, RequestId, ServerCapabilities, method,
};
use crate::transport::McpTransport;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Handler invoked when the server sends a request back to the client
/// (e.g. `sampling/createMessage`).  Returning `Ok(value)` becomes the
/// JSON-RPC `result`; returning `Err` becomes a JSON-RPC error response.
pub type ServerRequestHandler = Arc<
    dyn Fn(
            String,
            Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, JsonRpcError>> + Send>,
        > + Send
        + Sync,
>;

pub struct McpClient {
    inner: Arc<Inner>,
}

struct Inner {
    transport: Arc<dyn McpTransport>,
    next_id: AtomicI64,
    pending: DashMap<RequestId, oneshot::Sender<JsonRpcResponse>>,
    server_handler: arc_swap::ArcSwapOption<ServerRequestHandler>,
    server_caps: arc_swap::ArcSwapOption<ServerCapabilities>,
    server_info: arc_swap::ArcSwapOption<Implementation>,
    request_timeout: Duration,
}

impl McpClient {
    /// Wrap a transport and immediately spin up the reader task.  Caller
    /// is expected to invoke [`Self::initialize`] before any other RPC.
    pub fn new(transport: Arc<dyn McpTransport>) -> Self {
        let inner = Arc::new(Inner {
            transport,
            next_id: AtomicI64::new(1),
            pending: DashMap::new(),
            server_handler: arc_swap::ArcSwapOption::from(None),
            server_caps: arc_swap::ArcSwapOption::from(None),
            server_info: arc_swap::ArcSwapOption::from(None),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        });
        let inner_clone = inner.clone();
        tokio::spawn(async move { reader_loop(inner_clone).await });
        Self { inner }
    }

    pub fn set_server_request_handler(&self, handler: ServerRequestHandler) {
        self.inner.server_handler.store(Some(Arc::new(handler)));
    }

    pub fn server_capabilities(&self) -> Option<Arc<ServerCapabilities>> {
        self.inner.server_caps.load_full()
    }

    pub fn server_info(&self) -> Option<Arc<Implementation>> {
        self.inner.server_info.load_full()
    }

    pub async fn initialize(
        &self,
        client_name: &str,
        client_version: &str,
    ) -> McpResult<InitializeResult> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: Default::default(),
            client_info: Implementation {
                name: client_name.to_string(),
                version: client_version.to_string(),
            },
        };
        let result: InitializeResult = self
            .request_typed(method::INITIALIZE, Some(serde_json::to_value(&params)?))
            .await?;
        self.inner
            .server_caps
            .store(Some(Arc::new(result.capabilities.clone())));
        self.inner
            .server_info
            .store(Some(Arc::new(result.server_info.clone())));
        // Per spec, send the `notifications/initialized` notification once
        // initialization is complete.
        self.notify(method::INITIALIZED, None).await?;
        Ok(result)
    }

    pub async fn list_tools(&self) -> McpResult<ListToolsResult> {
        self.request_typed(method::TOOLS_LIST, Some(json!({}))).await
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> McpResult<CallToolResult> {
        let params = CallToolParams {
            name: name.to_string(),
            arguments,
        };
        self.request_typed(method::TOOLS_CALL, Some(serde_json::to_value(&params)?))
            .await
    }

    pub async fn list_resources(&self) -> McpResult<ListResourcesResult> {
        self.request_typed(method::RESOURCES_LIST, Some(json!({})))
            .await
    }

    pub async fn read_resource(&self, uri: &str) -> McpResult<ReadResourceResult> {
        let params = ReadResourceParams { uri: uri.to_string() };
        self.request_typed(method::RESOURCES_READ, Some(serde_json::to_value(&params)?))
            .await
    }

    pub async fn list_prompts(&self) -> McpResult<ListPromptsResult> {
        self.request_typed(method::PROMPTS_LIST, Some(json!({}))).await
    }

    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> McpResult<GetPromptResult> {
        let params = GetPromptParams { name: name.to_string(), arguments };
        self.request_typed(method::PROMPTS_GET, Some(serde_json::to_value(&params)?))
            .await
    }

    pub async fn ping(&self) -> McpResult<()> {
        let _: Value = self.request_typed(method::PING, Some(json!({}))).await?;
        Ok(())
    }

    pub async fn shutdown(&self) -> McpResult<()> {
        self.inner.transport.close().await
    }

    async fn request_typed<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> McpResult<T> {
        let value = self.request(method, params).await?;
        Ok(serde_json::from_value(value)?)
    }

    async fn request(&self, method: &str, params: Option<Value>) -> McpResult<Value> {
        let id = RequestId::Number(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let req = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(id.clone(), tx);
        if let Err(e) = self.inner.transport.send(serde_json::to_value(&req)?).await {
            self.inner.pending.remove(&id);
            return Err(e);
        }
        let resp = match tokio::time::timeout(self.inner.request_timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => {
                self.inner.pending.remove(&id);
                return Err(McpError::TransportClosed);
            }
            Err(_) => {
                self.inner.pending.remove(&id);
                return Err(McpError::Timeout);
            }
        };
        if let Some(err) = resp.error {
            return Err(McpError::Rpc { code: err.code, message: err.message });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> McpResult<()> {
        let n = JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.to_string(),
            params,
        };
        self.inner.transport.send(serde_json::to_value(&n)?).await
    }
}

async fn reader_loop(inner: Arc<Inner>) {
    loop {
        let frame = match inner.transport.recv().await {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(target: "agena_mcp_client::reader", "transport ended: {e}");
                // Cancel all pending requests.
                let mut keys: Vec<RequestId> = inner.pending.iter().map(|kv| kv.key().clone()).collect();
                for k in keys.drain(..) {
                    if let Some((_, tx)) = inner.pending.remove(&k) {
                        // Drop the sender — the awaiter will see a recv error.
                        drop(tx);
                    }
                }
                return;
            }
        };
        match frame {
            InboundMessage::Response(resp) => {
                if let Some((_, tx)) = inner.pending.remove(&resp.id) {
                    let _ = tx.send(resp);
                } else {
                    tracing::warn!(
                        target: "agena_mcp_client::reader",
                        "response with unknown id {:?}",
                        resp.id
                    );
                }
            }
            InboundMessage::Notification(n) => {
                tracing::trace!(
                    target: "agena_mcp_client::reader",
                    method = %n.method,
                    "server notification"
                );
                // We currently don't surface notifications to callers; future
                // work: route resource/tool list_changed back into the
                // connection manager so the catalog refreshes.
            }
            InboundMessage::Request(req) => {
                let handler = inner.server_handler.load_full();
                let inner_for_resp = inner.clone();
                let id = req.id.clone();
                let method_name = req.method.clone();
                let params = req.params.unwrap_or(Value::Null);
                tokio::spawn(async move {
                    let result = match handler {
                        Some(h) => (h)(method_name.clone(), params).await,
                        None => Err(JsonRpcError {
                            code: -32601,
                            message: format!("method '{method_name}' not handled by client"),
                            data: None,
                        }),
                    };
                    if let Err(e) = inner_for_resp_send(inner_for_resp, id, result).await {
                        tracing::warn!(
                            target: "agena_mcp_client::reader",
                            "failed to send response: {e}"
                        );
                    }
                });
            }
        }
    }
}

async fn inner_for_resp_send(
    inner: Arc<Inner>,
    id: RequestId,
    result: Result<Value, JsonRpcError>,
) -> McpResult<()> {
    let resp = match result {
        Ok(v) => JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(v),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(e),
        },
    };
    inner.transport.send(serde_json::to_value(&resp)?).await
}

/// Convenience: deserialize a `sampling/createMessage` params payload.
pub fn parse_create_message_params(value: &Value) -> Result<CreateMessageParams, serde_json::Error> {
    serde_json::from_value(value.clone())
}

/// Convenience: serialize a `sampling/createMessage` result payload.
pub fn serialize_create_message_result(result: &CreateMessageResult) -> serde_json::Result<Value> {
    serde_json::to_value(result)
}
