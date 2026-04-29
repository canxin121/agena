//! End-to-end-ish test that exercises the full McpClient against a
//! handwritten in-memory transport.  We don't spawn a real child process
//! here (CI may not have node/python); see `tests/stdio_smoke.rs` for
//! that.

use std::sync::Arc;

use agena_mcp_client::McpClient;
use agena_mcp_client::error::{McpError, McpResult};
use agena_mcp_client::protocol::{
    InboundMessage, JSONRPC_VERSION, JsonRpcResponse, ListToolsResult, RequestId, ToolDescriptor,
    method,
};
use agena_mcp_client::transport::McpTransport;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

/// Bidirectional pair: caller writes via `client_tx`, transport reads it
/// out of `client_rx`; transport writes via `server_tx` (becomes the
/// reader's inbox).
struct InMemTransport {
    outbox: mpsc::UnboundedSender<Value>,
    inbox: Mutex<mpsc::UnboundedReceiver<InboundMessage>>,
}

#[async_trait]
impl McpTransport for InMemTransport {
    async fn send(&self, payload: Value) -> McpResult<()> {
        self.outbox
            .send(payload)
            .map_err(|e| McpError::Transport(e.to_string()))
    }

    async fn recv(&self) -> McpResult<InboundMessage> {
        let mut g = self.inbox.lock().await;
        g.recv().await.ok_or(McpError::TransportClosed)
    }

    async fn close(&self) -> McpResult<()> {
        Ok(())
    }
}

/// Tiny test "server" that runs in a tokio task: reads outbound frames
/// from the client and produces canned responses.
fn spawn_fake_server(
    mut from_client: mpsc::UnboundedReceiver<Value>,
    to_client: mpsc::UnboundedSender<InboundMessage>,
) {
    tokio::spawn(async move {
        while let Some(msg) = from_client.recv().await {
            let id = msg.get("id").cloned();
            let method = msg
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let Some(id) = id else {
                // It's a notification; just drop it.
                continue;
            };
            let id: RequestId = serde_json::from_value(id).unwrap();
            let result = match method.as_str() {
                m if m == method::INITIALIZE => json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "fake", "version": "0.0.1" }
                }),
                m if m == method::TOOLS_LIST => json!({
                    "tools": [
                        { "name": "echo", "description": "echo back",
                          "inputSchema": {"type":"object"} }
                    ]
                }),
                m if m == method::TOOLS_CALL => {
                    let args = msg
                        .get("params")
                        .and_then(|p| p.get("arguments"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    json!({
                        "content": [
                            { "type": "text", "text": format!("got: {args}") }
                        ]
                    })
                }
                _ => json!(null),
            };
            let resp = JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id,
                result: Some(result),
                error: None,
            };
            let _ = to_client.send(InboundMessage::Response(resp));
        }
    });
}

#[tokio::test]
async fn initialize_list_tools_and_call_tool() {
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Value>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<InboundMessage>();
    spawn_fake_server(out_rx, in_tx);

    let transport = Arc::new(InMemTransport {
        outbox: out_tx,
        inbox: Mutex::new(in_rx),
    });
    let client = McpClient::new(transport);

    let init = client.initialize("agena-test", "0.0.1").await.unwrap();
    assert_eq!(init.protocol_version, "2024-11-05");

    let tools: ListToolsResult = client.list_tools().await.unwrap();
    assert_eq!(tools.tools.len(), 1);
    assert_eq!(tools.tools[0].name, "echo");
    let _: ToolDescriptor = tools.tools[0].clone();

    let result = client
        .call_tool("echo", Some(json!({"msg": "hi"})))
        .await
        .unwrap();
    assert_eq!(result.content.len(), 1);
}
