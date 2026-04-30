//! `agena-mcp-client` — async MCP (Model Context Protocol) client used by
//! the agena runtime.  Supports stdio and HTTP/SSE transports, exposes the
//! 2024-11-05 protocol surface, and routes server→client requests
//! (`sampling/createMessage`, …) through a pluggable handler.
//!
//! See `protocol` for wire types, `transport` for the I/O abstraction,
//! `client` for the per-server JSON-RPC bookkeeping, and `manager` for the
//! multi-server connection pool.

pub mod client;
pub mod error;
pub mod manager;
pub mod protocol;
pub mod token_store;
pub mod transport;

pub use client::{McpClient, ServerRequestHandler};
pub use error::{McpError, McpResult};
pub use manager::{ConnectedServer, HttpAuth, McpConnectionManager, ServerSpec, TokenStore};
pub use token_store::{FileTokenStore, TokenStoreError};
pub use protocol::{
    CallToolResult, ContentBlock, CreateMessageParams, CreateMessageResult, GetPromptResult,
    InitializeResult, JsonRpcError, ListPromptsResult, ListResourcesResult, ListToolsResult,
    ReadResourceResult, ResourceContents, ResourceDescriptor, ServerCapabilities, ToolDescriptor,
};
pub use transport::{HttpTransport, HttpTransportMode, McpTransport, StdioTransport};
