//! # agena-mcp-client
//!
//! MCP client wrapper built on top of the official Rust SDK (`rmcp`).
//!
//! Provides the MCP protocol types ([`protocol`]), connection management
//! ([`manager`]), OAuth login sessions ([`oauth`]), token storage
//! ([`token_store`]), and typed errors ([`McpError`] / [`McpResult`]).

pub mod error;
pub mod manager;
pub mod protocol;
pub mod token_store;

pub use error::{McpError, McpResult};
pub use manager::{
    ConnectedServer, HttpAuth, McpConnectionManager, McpCredentialMigration, McpCredentialState,
    McpServerAuthMode, McpServerStatus, McpToolPolicy, McpToolRisk, ReconnectPolicy, ServerSpec,
    TokenStore,
};
pub use protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourcesResult, ListToolsResult, PromptArgument, PromptDescriptor,
    PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ToolDescriptor,
};
pub use token_store::{
    FallbackTokenStore, FileTokenStore, KeyringOAuthCredentialStore, KeyringTokenStore,
    MCP_KEYRING_SERVICE, OAuthCredentialHealth, OAuthCredentialState, OAuthExpiryState,
    TokenStoreError,
};
