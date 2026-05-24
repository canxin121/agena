//! `agena-mcp-client` — MCP client wrapper built on top of the official
//! Rust SDK (`rmcp`).

pub mod error;
pub mod manager;
pub mod protocol;
pub mod token_store;

pub use error::{McpError, McpResult};
pub use manager::{ConnectedServer, HttpAuth, McpConnectionManager, ServerSpec, TokenStore};
pub use protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PromptArgument, PromptDescriptor,
    PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ToolDescriptor,
};
pub use token_store::{FileTokenStore, TokenStoreError};
