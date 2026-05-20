//! Transport abstraction and concrete implementations (stdio, HTTP/SSE, WebSocket).

use async_trait::async_trait;
use serde_json::Value;

use crate::error::McpResult;
use crate::protocol::InboundMessage;

pub mod http;
pub mod stdio;
pub mod ws;

pub use http::{HttpTransport, HttpTransportMode};
pub use stdio::StdioTransport;
pub use ws::WsTransport;

/// Bidirectional message channel to a single MCP server.
///
/// Implementations must be `Send + Sync` because the client owns one
/// transport and uses it concurrently from a writer task (sending requests
/// & responses) and a reader task (dispatching inbound frames).
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Serialize and send a JSON-RPC payload (request, notification, or
    /// response) to the server.
    async fn send(&self, payload: Value) -> McpResult<()>;

    /// Block until the next inbound frame is available.  Returns
    /// `TransportClosed` when the connection has been torn down.
    async fn recv(&self) -> McpResult<InboundMessage>;

    /// Initiate a graceful shutdown.  Subsequent `send` / `recv` calls
    /// should return `TransportClosed`.
    async fn close(&self) -> McpResult<()>;
}
