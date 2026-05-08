//! Server side of Model Context Protocol (MCP). Lets the agena runtime
//! expose itself as an MCP backend so external clients (IDE plugins,
//! other agents) can list / invoke tools over JSON-RPC.
//!
//! See `agena-mcp-client` for the client-side implementation.

use std::sync::Arc;

use agena_mcp_client::protocol::{
    self, CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    Implementation, InboundMessage, InitializeResult, JsonRpcError, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, ListPromptsResult, ListResourcesResult, ListToolsResult,
    PromptDescriptor, PromptsCapability, ReadResourceParams, ReadResourceResult,
    ResourceDescriptor, ResourcesCapability, ServerCapabilities, ToolDescriptor, ToolsCapability,
};
use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum McpServerError {
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub instructions: Option<String>,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "agena".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            instructions: Some(
                "Agena exposes its local tools, resources, and prompts over MCP.".to_owned(),
            ),
        }
    }
}

#[async_trait]
pub trait McpServerBackend: Send + Sync + 'static {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError>;
    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError>;

    async fn list_resources(&self) -> Result<Vec<ResourceDescriptor>, McpServerError> {
        Ok(Vec::new())
    }

    async fn read_resource(
        &self,
        params: ReadResourceParams,
    ) -> Result<ReadResourceResult, McpServerError> {
        Err(McpServerError::NotFound(params.uri))
    }

    async fn list_prompts(&self) -> Result<Vec<PromptDescriptor>, McpServerError> {
        Ok(Vec::new())
    }

    async fn get_prompt(&self, params: GetPromptParams) -> Result<GetPromptResult, McpServerError> {
        Err(McpServerError::NotFound(params.name))
    }
}

#[derive(Clone)]
pub struct McpServer<B> {
    backend: Arc<B>,
    info: ServerInfo,
}

impl<B> McpServer<B>
where
    B: McpServerBackend,
{
    pub fn new(backend: B) -> Self {
        Self::with_info(backend, ServerInfo::default())
    }

    pub fn with_info(backend: B, info: ServerInfo) -> Self {
        Self {
            backend: Arc::new(backend),
            info,
        }
    }

    pub async fn serve_stdio<R, W>(&self, reader: R, writer: W) -> Result<(), McpServerError>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut lines = reader.lines();
        let mut writer = writer;
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)?;
            if let Some(response) = self.handle_value(value).await? {
                let mut encoded = serde_json::to_vec(&response)?;
                encoded.push(b'\n');
                writer.write_all(&encoded).await?;
                writer.flush().await?;
            }
        }
        Ok(())
    }

    pub async fn handle_value(
        &self,
        value: Value,
    ) -> Result<Option<JsonRpcResponse>, McpServerError> {
        match InboundMessage::from_value(value)? {
            InboundMessage::Request(request) => Ok(Some(self.handle_request(request).await)),
            InboundMessage::Notification(notification) => {
                self.handle_notification(notification).await;
                Ok(None)
            }
            InboundMessage::Response(_) => Ok(None),
        }
    }

    async fn handle_notification(&self, notification: JsonRpcNotification) {
        if notification.method != protocol::method::INITIALIZED {
            tracing::debug!(method = notification.method, "ignoring MCP notification");
        }
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let result = match request.method.as_str() {
            protocol::method::INITIALIZE => self.initialize(),
            protocol::method::PING => Ok(serde_json::json!({})),
            protocol::method::TOOLS_LIST => self.tools_list().await,
            protocol::method::TOOLS_CALL => self.tools_call(request.params).await,
            protocol::method::RESOURCES_LIST => self.resources_list().await,
            protocol::method::RESOURCES_READ => self.resources_read(request.params).await,
            protocol::method::PROMPTS_LIST => self.prompts_list().await,
            protocol::method::PROMPTS_GET => self.prompts_get(request.params).await,
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("method not found: {}", request.method),
                data: None,
            }),
        };
        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: protocol::JSONRPC_VERSION.to_owned(),
                id: request.id,
                result: Some(value),
                error: None,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: protocol::JSONRPC_VERSION.to_owned(),
                id: request.id,
                result: None,
                error: Some(error),
            },
        }
    }

    fn initialize(&self) -> Result<Value, JsonRpcError> {
        serialize_result(InitializeResult {
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: Some(ResourcesCapability {
                    subscribe: Some(false),
                    list_changed: Some(false),
                }),
                prompts: Some(PromptsCapability {
                    list_changed: Some(false),
                }),
                logging: None,
                experimental: Default::default(),
            },
            server_info: Implementation {
                name: self.info.name.clone(),
                version: self.info.version.clone(),
            },
            instructions: self.info.instructions.clone(),
        })
    }

    async fn tools_list(&self) -> Result<Value, JsonRpcError> {
        let tools = self.backend.list_tools().await.map_err(to_json_rpc_error)?;
        serialize_result(ListToolsResult {
            tools,
            next_cursor: None,
        })
    }

    async fn tools_call(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = decode_params::<CallToolParams>(params)?;
        let result = self
            .backend
            .call_tool(params)
            .await
            .map_err(to_json_rpc_error)?;
        serialize_result(result)
    }

    async fn resources_list(&self) -> Result<Value, JsonRpcError> {
        let resources = self
            .backend
            .list_resources()
            .await
            .map_err(to_json_rpc_error)?;
        serialize_result(ListResourcesResult {
            resources,
            next_cursor: None,
        })
    }

    async fn resources_read(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = decode_params::<ReadResourceParams>(params)?;
        let result = self
            .backend
            .read_resource(params)
            .await
            .map_err(to_json_rpc_error)?;
        serialize_result(result)
    }

    async fn prompts_list(&self) -> Result<Value, JsonRpcError> {
        let prompts = self
            .backend
            .list_prompts()
            .await
            .map_err(to_json_rpc_error)?;
        serialize_result(ListPromptsResult {
            prompts,
            next_cursor: None,
        })
    }

    async fn prompts_get(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = decode_params::<GetPromptParams>(params)?;
        let result = self
            .backend
            .get_prompt(params)
            .await
            .map_err(to_json_rpc_error)?;
        serialize_result(result)
    }
}

pub async fn serve_stdio<B>(backend: B) -> Result<(), McpServerError>
where
    B: McpServerBackend,
{
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    McpServer::new(backend).serve_stdio(stdin, stdout).await
}

pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text { text: text.into() }],
        is_error: None,
    }
}

pub fn text_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text { text: text.into() }],
        is_error: Some(true),
    }
}

fn decode_params<T>(params: Option<Value>) -> Result<T, JsonRpcError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(params.unwrap_or_else(|| serde_json::json!({}))).map_err(|err| {
        JsonRpcError {
            code: -32602,
            message: format!("invalid params: {err}"),
            data: None,
        }
    })
}

fn serialize_result<T>(value: T) -> Result<Value, JsonRpcError>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|err| JsonRpcError {
        code: -32603,
        message: format!("serialize result: {err}"),
        data: None,
    })
}

fn to_json_rpc_error(error: McpServerError) -> JsonRpcError {
    match error {
        McpServerError::InvalidParams(message) => JsonRpcError {
            code: -32602,
            message,
            data: None,
        },
        McpServerError::NotFound(message) => JsonRpcError {
            code: -32004,
            message,
            data: None,
        },
        other => JsonRpcError {
            code: -32603,
            message: other.to_string(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_mcp_client::protocol::{RequestId, ToolDescriptor};
    use serde_json::json;

    #[derive(Clone)]
    struct TestBackend;

    #[async_trait]
    impl McpServerBackend for TestBackend {
        async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
            Ok(vec![ToolDescriptor {
                name: "echo".to_owned(),
                description: Some("Echo input".to_owned()),
                input_schema: Some(json!({"type":"object"})),
            }])
        }

        async fn call_tool(
            &self,
            params: CallToolParams,
        ) -> Result<CallToolResult, McpServerError> {
            Ok(text_result(params.name))
        }
    }

    #[tokio::test]
    async fn initializes_with_capabilities() {
        let server = McpServer::new(TestBackend);
        let response = server
            .handle_value(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name":"test", "version":"0"}
                }
            }))
            .await
            .unwrap()
            .unwrap();

        assert!(response.error.is_none());
        assert_eq!(response.id, RequestId::Number(1));
        assert!(response.result.unwrap()["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn lists_and_calls_tools() {
        let server = McpServer::new(TestBackend);
        let listed = server
            .handle_value(json!({"jsonrpc":"2.0","id":"list","method":"tools/list"}))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(listed.result.unwrap()["tools"][0]["name"], "echo");

        let called = server
            .handle_value(json!({
                "jsonrpc":"2.0",
                "id":"call",
                "method":"tools/call",
                "params":{"name":"echo","arguments":{"text":"hi"}}
            }))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(called.result.unwrap()["content"][0]["text"], "echo");
    }
}
